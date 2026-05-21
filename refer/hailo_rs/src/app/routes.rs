use async_stream::stream;
use axum::{
    body::{ Body, Bytes },
    extract::{ Path, State },
    http::{ header, StatusCode },
    response::{ IntoResponse, Response },
    routing::{ delete, get, post, put },
    Json,
    Router,
};
use std::convert::Infallible;
use tower_http::services::{ ServeDir, ServeFile };

use super::{
    config::{ list_video_devices, save_postprocess_config },
    image::clamp_roi_to_frame,
    options::CameraSource,
    state::BackendState,
    types::*,
};

use kmmacro::{
    load_store,
    run_script,
    save_store,
    MacroScript,
    MacroStore,
    load_rule_store,
    save_rule_store,
    RuleStore,
    TriggerRule,
};
use mbus::{ snapshot_all, dam0888_write_do };
use mbus::devices::dido::RelayChannel;

const MJPEG_BOUNDARY: &str = "frame";

// ── Router ────────────────────────────────────────────────────────────────────

pub fn build_router(state: BackendState) -> Router {
    let api = Router::new()
        .route("/api/health", get(api_health))
        .route("/api/settings", get(api_get_settings).put(api_update_settings))
        .route("/api/annotations", get(api_annotations))
        .route("/api/frame.jpg", get(api_frame_snapshot))
        .route("/api/frame.mjpeg", get(api_frame_stream))
        .route("/api/camera", put(api_update_camera))
        .route("/api/devices", get(api_get_devices))
        .route("/api/mbus/snapshot", get(api_mbus_snapshot))
        .route("/api/mbus/do", put(api_mbus_do_update))
        // ── 巨集 API ──────────────────────────────────────────────────────────
        .route("/api/macros", get(api_list_macros).post(api_save_macro))
        .route("/api/macros/{name}", delete(api_delete_macro))
        .route("/api/macros/{name}/run", post(api_run_macro))
        // ── 觸發規則 API ──────────────────────────────────────────────────────
        .route("/api/rules", get(api_list_rules).post(api_save_rule))
        .route("/api/rules/{name}", delete(api_delete_rule))
        .route("/api/rules/{name}/enable", put(api_set_rule_enabled))
        .with_state(state.clone());

    if let Some(static_dir) = &state.options.static_dir {
        let index_file = static_dir.join("index.html");
        if index_file.exists() {
            return api.fallback_service(
                ServeDir::new(static_dir.clone()).not_found_service(ServeFile::new(index_file))
            );
        }
    }

    api
}

// ── API handlers ──────────────────────────────────────────────────────────────

async fn api_health(State(state): State<BackendState>) -> Json<HealthResponse> {
    Json(state.health_response())
}

async fn api_get_settings(State(state): State<BackendState>) -> Json<SettingsResponse> {
    Json(state.settings_response())
}

async fn api_annotations(State(state): State<BackendState>) -> Json<Vec<AnnotationPayload>> {
    Json(state.snapshot_annotations().into_iter().map(AnnotationPayload::from).collect())
}

async fn api_update_settings(
    State(state): State<BackendState>,
    Json(request): Json<SettingsUpdateRequest>
) -> Result<Json<SettingsResponse>, (StatusCode, String)> {
    let (fw, fh) = state.frame_size();
    let manual_rois = request.manual_rois
        .into_iter()
        .filter_map(|roi| {
            clamp_roi_to_frame(opencv::core::Rect::new(roi.x, roi.y, roi.width, roi.height), fw, fh)
        })
        .collect::<Vec<_>>();

    state.replace_config(request.config).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state.replace_manual_rois(manual_rois).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Switching to manual mode: immediately flush stale auto-detected annotations so
    // they don't linger until the OCR thread processes the next frame.
    if request.config.manual_roi_mode {
        state.replace_annotations(vec![]);
    }

    if request.persist {
        save_postprocess_config(&state.config_path, &state.snapshot_config()).map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        ))?;
    }

    state.clear_error();
    Ok(Json(state.settings_response()))
}

/// Switch the active camera source at runtime.
/// The capture thread detects the change and reopens the device automatically.
async fn api_update_camera(
    State(state): State<BackendState>,
    Json(request): Json<CameraUpdateRequest>
) -> Result<Json<HealthResponse>, (StatusCode, String)> {
    if state.options.mock {
        return Err((StatusCode::BAD_REQUEST, "Mock 模式下無法切換攝影機".to_owned()));
    }
    state.set_camera_source(CameraSource::parse(&request.source));
    Ok(Json(state.health_response()))
}

/// List available `/dev/videoN` devices and the current camera source.
async fn api_get_devices(State(state): State<BackendState>) -> Json<DevicesResponse> {
    Json(DevicesResponse {
        devices: list_video_devices(),
        current: state.snapshot_camera_source().display(),
    })
}

async fn api_frame_snapshot(State(state): State<BackendState>) -> Response {
    let frame = state.frame_tx.borrow().clone();
    if frame.is_empty() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate, max-age=0"),
        ],
        frame,
    ).into_response()
}

async fn api_frame_stream(State(state): State<BackendState>) -> Response {
    let mut receiver = state.frame_tx.subscribe();
    let first_frame = receiver.borrow().clone();
    let stream = stream! {
        if !first_frame.is_empty() {
            yield Ok::<Bytes, Infallible>(mjpeg_chunk(&first_frame));
        }
        loop {
            if receiver.changed().await.is_err() {
                break;
            }
            let frame = receiver.borrow().clone();
            if frame.is_empty() {
                continue;
            }
            yield Ok::<Bytes, Infallible>(mjpeg_chunk(&frame));
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/x-mixed-replace; boundary={MJPEG_BOUNDARY}")
        )
        .header(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate, max-age=0")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ── MJPEG framing ─────────────────────────────────────────────────────────────

pub fn mjpeg_chunk(frame: &[u8]) -> Bytes {
    let mut payload = format!(
        "--{MJPEG_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        frame.len()
    ).into_bytes();
    payload.extend_from_slice(frame);
    payload.extend_from_slice(b"\r\n");
    Bytes::from(payload)
}

// ── MBus handlers ──────────────────────────────────────────────────────────────

/// Convert wind speed (m/s) to Beaufort wind force scale (0–12).
fn beaufort_scale(speed_ms: f32) -> u8 {
    match speed_ms {
        s if s < 0.3 => 0,
        s if s < 1.6 => 1,
        s if s < 3.4 => 2,
        s if s < 5.5 => 3,
        s if s < 8.0 => 4,
        s if s < 10.8 => 5,
        s if s < 13.9 => 6,
        s if s < 17.2 => 7,
        s if s < 20.8 => 8,
        s if s < 24.5 => 9,
        s if s < 28.5 => 10,
        s if s < 32.7 => 11,
        _ => 12,
    }
}

/// 讀取所有 RS485 設備的即時快照（串列埠只開啟一次，透過 set_slave 切換設備）。
/// 個別設備超時或錯誤時回傳 null 並在 errors 陣列記錄，不影響其他設備。
async fn api_mbus_snapshot(State(state): State<BackendState>) -> Json<MbusSnapshotResponse> {
    let port = state.mbus_port.clone();
    let _guard = state.mbus_lock.lock().await;
    let result = snapshot_all(&port).await;

    let mut errors = Vec::new();

    let dam0888 = match result.dam0888 {
        Ok(snap) => {
            let ai = snap.analog_inputs;
            Some(MbusDam0888 {
                digital_inputs: snap.digital_inputs,
                digital_outputs: snap.digital_outputs,
                analog_inputs: ai,
                temperature_c: (ai[1] / 10.0) * 100.0 - 20.0,
                humidity_rh: (ai[2] / 10.0) * 100.0,
                noise_db: ai[3] / 0.01 / 10.0 + 30.0,
                distance_mm: 50.0 + (ai[4] / 10.0) * 350.0,
            })
        }
        Err(e) => {
            errors.push(format!("DAM0888: {e}"));
            None
        }
    };

    let cd20s = match result.cd20s {
        Ok(m) =>
            Some(MbusCd20s {
                raw_integer: m.raw_integer,
                float_value: m.float_value,
                vibration_mms: m.vibration_mms,
            }),
        Err(e) => {
            errors.push(format!("CD20S: {e}"));
            None
        }
    };

    let sd3788b = match result.sd3788b {
        Ok(w) => {
            let speed = w.speed_ms;
            Some(MbusSd3788b {
                raw: w.raw,
                speed_ms: speed,
                wind_force: beaufort_scale(speed),
            })
        }
        Err(e) => {
            errors.push(format!("SD3788B: {e}"));
            None
        }
    };

    Json(MbusSnapshotResponse { dam0888, cd20s, sd3788b, errors })
}

/// 設定 DAM0888 單一 DO 通道狀態。
/// Body: `{ "channel": 0..7, "state": true|false }`
async fn api_mbus_do_update(
    State(state): State<BackendState>,
    Json(req): Json<MbusDoUpdateRequest>
) -> Result<StatusCode, (StatusCode, String)> {
    let channel = match req.channel {
        0 => RelayChannel::DO1,
        1 => RelayChannel::DO2,
        2 => RelayChannel::DO3,
        3 => RelayChannel::DO4,
        4 => RelayChannel::DO5,
        5 => RelayChannel::DO6,
        6 => RelayChannel::DO7,
        7 => RelayChannel::DO8,
        n => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("channel {n} 超出範圍，須為 0~7"),
            ));
        }
    };

    let port = state.mbus_port.clone();
    let _guard = state.mbus_lock.lock().await;
    dam0888_write_do(&port, channel, req.state).await.map_err(|e| (
        StatusCode::INTERNAL_SERVER_ERROR,
        e.to_string(),
    ))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── 巨集 handlers ──────────────────────────────────────────────────────────────

/// 回傳所有已儲存的巨集腳本清單。
async fn api_list_macros() -> Json<Vec<MacroScript>> {
    let scripts = tokio::task
        ::spawn_blocking(|| { load_store(None).unwrap_or_default().scripts }).await
        .unwrap_or_default();
    Json(scripts)
}

/// 新增或更新一份巨集腳本（以 name 作為主鍵）。
async fn api_save_macro(Json(script): Json<MacroScript>) -> Result<
    StatusCode,
    (StatusCode, String)
> {
    tokio::task
        ::spawn_blocking(move || {
            let mut store = load_store(None).unwrap_or_default();
            if let Some(pos) = store.scripts.iter().position(|s| s.name == script.name) {
                store.scripts[pos] = script;
            } else {
                store.scripts.push(script);
            }
            save_store(None, &store)
        }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// 刪除指定名稱的巨集腳本。
async fn api_delete_macro(Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    tokio::task
        ::spawn_blocking(move || {
            let mut store = load_store(None).unwrap_or_default();
            store.scripts.retain(|s| s.name != name);
            save_store(None, &store)
        }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// 執行指定名稱的巨集腳本（在 blocking thread 中透過 /dev/hidg0 送出 HID 事件）。
async fn api_run_macro(Path(name): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    let script = tokio::task
        ::spawn_blocking(
            move || -> Result<MacroScript, String> {
                let store = load_store(None).map_err(|e| e.to_string())?;
                store.scripts
                    .into_iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| "找不到指定的巨集腳本".to_owned())
            }
        ).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::NOT_FOUND, e))?;

    tokio::task
        ::spawn_blocking(move || run_script(&script)).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── 觸發規則 handlers ────────────────────────────────────────────────────────

/// 回傳所有已儲存的觸發規則。
async fn api_list_rules(State(state): State<BackendState>) -> Json<Vec<TriggerRule>> {
    let rules = state.trigger_rules
        .lock()
        .map(|r| r.clone())
        .unwrap_or_default();
    Json(rules)
}

/// 新增或更新一條觸發規則（以 name 作為主鍵），同時寫入磁碟並更新記憶體快取。
async fn api_save_rule(
    State(state): State<BackendState>,
    Json(rule): Json<TriggerRule>
) -> Result<StatusCode, (StatusCode, String)> {
    tokio::task
        ::spawn_blocking({
            let rule = rule.clone();
            move || {
                let mut store = load_rule_store(None).unwrap_or_default();
                if let Some(pos) = store.rules.iter().position(|r| r.name == rule.name) {
                    store.rules[pos] = rule;
                } else {
                    store.rules.push(rule);
                }
                save_rule_store(None, &store)
            }
        }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Ok(mut rules) = state.trigger_rules.lock() {
        if let Some(pos) = rules.iter().position(|r| r.name == rule.name) {
            rules[pos] = rule;
        } else {
            rules.push(rule);
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// 刪除指定名稱的觸發規則。
async fn api_delete_rule(
    State(state): State<BackendState>,
    Path(name): Path<String>
) -> Result<StatusCode, (StatusCode, String)> {
    tokio::task
        ::spawn_blocking({
            let name = name.clone();
            move || {
                let mut store = load_rule_store(None).unwrap_or_default();
                store.rules.retain(|r| r.name != name);
                save_rule_store(None, &store)
            }
        }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Ok(mut rules) = state.trigger_rules.lock() {
        rules.retain(|r| r.name != name);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
struct EnableRequest {
    enabled: bool,
}

/// 切換指定規則的啟用狀態。
async fn api_set_rule_enabled(
    State(state): State<BackendState>,
    Path(name): Path<String>,
    Json(body): Json<EnableRequest>
) -> Result<StatusCode, (StatusCode, String)> {
    tokio::task
        ::spawn_blocking({
            let name = name.clone();
            move || {
                let mut store = load_rule_store(None).unwrap_or_default();
                if let Some(r) = store.rules.iter_mut().find(|r| r.name == name) {
                    r.enabled = body.enabled;
                }
                save_rule_store(None, &store)
            }
        }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Ok(mut rules) = state.trigger_rules.lock() {
        if let Some(r) = rules.iter_mut().find(|r| r.name == name) {
            r.enabled = body.enabled;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}
