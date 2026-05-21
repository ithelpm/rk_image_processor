use opencv::core::Rect;
use std::path::PathBuf;
use std::sync::{ Arc, Mutex };
use tokio::sync::{ watch, Mutex as AsyncMutex };

use kmmacro::TriggerRule;

use crate::hw::MppJpegEncoder;
use crate::infer::PostprocessConfig;

use super::options::{ CameraSource, ServerOptions };
use super::types::{
    AnnotationPayload,
    HealthResponse,
    OcrAnnotation,
    RoiPayload,
    SettingsResponse,
};

pub const DEFAULT_FRAME_WIDTH: i32 = 1280;
pub const DEFAULT_FRAME_HEIGHT: i32 = 720;

pub enum MppJpegState {
    Uninitialized,
    Ready(MppJpegEncoder),
    Disabled,
}

impl MppJpegState {
    pub fn from_env() -> Self {
        if Self::disabled_by_env() { Self::Disabled } else { Self::Uninitialized }
    }

    pub fn disabled_by_env() -> bool {
        std::env
            ::var("HYPER_MIX_DISABLE_MPP")
            .map(|value|
                matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
            )
            .unwrap_or(false)
    }

    pub fn encoder(&self) -> Option<&MppJpegEncoder> {
        match self {
            Self::Ready(enc) => Some(enc),
            Self::Uninitialized | Self::Disabled => None,
        }
    }
}

/// Shared server state — cheap to clone (all heavy data sits behind Arc).
#[derive(Clone)]
pub struct BackendState {
    pub options: ServerOptions,
    pub config_path: PathBuf,
    pub config: Arc<Mutex<PostprocessConfig>>,
    pub manual_rois: Arc<Mutex<Vec<Rect>>>,
    pub latest_annotations: Arc<Mutex<Vec<OcrAnnotation>>>,
    pub latest_frame_size: Arc<Mutex<(i32, i32)>>,
    pub last_error: Arc<Mutex<Option<String>>>,
    /// Broadcast channel for JPEG-encoded preview frames.
    pub frame_tx: watch::Sender<Vec<u8>>,
    /// Live camera source — may differ from `options.camera_source` after a runtime switch.
    pub camera_source: Arc<Mutex<CameraSource>>,
    /// Monotonically-increasing version counter; incrementing signals the capture thread
    /// to restart with the new `camera_source`.
    pub camera_restart_tx: watch::Sender<u64>,
    /// MPP 硬體 JPEG 編碼器狀態（初始化失敗後會停用並改走 OpenCV fallback）。
    pub mpp_jpeg: Arc<Mutex<MppJpegState>>,
    /// RS485 串列埠路徑，給 mbus 函式使用。
    pub mbus_port: String,
    /// 序列化所有 mbus 串列埠操作，避免同時開啟同一裝置。
    pub mbus_lock: Arc<AsyncMutex<()>>,
    /// 廣播最新 OCR 辨識文字清單，供觸發引擎訂閱。
    pub annotations_tx: watch::Sender<Vec<String>>,
    /// 觸發規則記憶體快取（已載入自 ~/.kmmacro_rules.json）。
    pub trigger_rules: Arc<Mutex<Vec<TriggerRule>>>,
}

impl BackendState {
    pub fn snapshot_config(&self) -> PostprocessConfig {
        self.config
            .lock()
            .map(|c| *c)
            .unwrap_or_default()
    }

    pub fn replace_config(&self, config: PostprocessConfig) -> Result<(), String> {
        self.config
            .lock()
            .map(|mut c| {
                *c = config;
            })
            .map_err(|_| "無法更新後處理設定".to_owned())
    }

    pub fn snapshot_manual_rois(&self) -> Vec<Rect> {
        self.manual_rois
            .lock()
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    pub fn replace_manual_rois(&self, rois: Vec<Rect>) -> Result<(), String> {
        self.manual_rois
            .lock()
            .map(|mut r| {
                *r = rois;
            })
            .map_err(|_| "無法更新手動 ROI".to_owned())
    }

    pub fn snapshot_annotations(&self) -> Vec<OcrAnnotation> {
        self.latest_annotations
            .lock()
            .map(|a| a.clone())
            .unwrap_or_default()
    }

    pub fn replace_annotations(&self, annotations: Vec<OcrAnnotation>) {
        // 提取 OCR 文字並廣播給觸發引擎
        let texts: Vec<String> = annotations
            .iter()
            .map(|a| a.text.clone())
            .collect();
        let _ = self.annotations_tx.send(texts);
        if let Ok(mut a) = self.latest_annotations.lock() {
            *a = annotations;
        }
    }

    pub fn frame_size(&self) -> (i32, i32) {
        self.latest_frame_size
            .lock()
            .map(|s| *s)
            .unwrap_or((DEFAULT_FRAME_WIDTH, DEFAULT_FRAME_HEIGHT))
    }

    pub fn set_frame_size(&self, width: i32, height: i32) {
        if let Ok(mut s) = self.latest_frame_size.lock() {
            *s = (width, height);
        }
    }

    pub fn set_error(&self, message: impl Into<String>) {
        if let Ok(mut e) = self.last_error.lock() {
            *e = Some(message.into());
        }
    }

    pub fn clear_error(&self) {
        if let Ok(mut e) = self.last_error.lock() {
            *e = None;
        }
    }

    pub fn snapshot_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .map(|e| e.clone())
            .unwrap_or(None)
    }

    pub fn snapshot_camera_source(&self) -> CameraSource {
        self.camera_source
            .lock()
            .map(|s| s.clone())
            .unwrap_or(CameraSource::Index(0))
    }

    /// Update the live camera source and signal the capture thread to restart.
    pub fn set_camera_source(&self, source: CameraSource) {
        if let Ok(mut s) = self.camera_source.lock() {
            *s = source;
        }
        let next_version = self.camera_restart_tx.borrow().saturating_add(1);
        let _ = self.camera_restart_tx.send(next_version);
    }

    pub fn health_response(&self) -> HealthResponse {
        let (frame_width, frame_height) = self.frame_size();
        HealthResponse {
            host: self.options.host.clone(),
            port: self.options.port,
            mock: self.options.mock,
            camera_source: self.snapshot_camera_source().display(),
            frame_width,
            frame_height,
            manual_roi_count: self.snapshot_manual_rois().len(),
            annotation_count: self.snapshot_annotations().len(),
            last_error: self.snapshot_error(),
            static_dir: self.options.static_dir.as_ref().map(|p| p.display().to_string()),
        }
    }

    pub fn settings_response(&self) -> SettingsResponse {
        let (frame_width, frame_height) = self.frame_size();
        SettingsResponse {
            config: self.snapshot_config(),
            manual_rois: self.snapshot_manual_rois().into_iter().map(RoiPayload::from).collect(),
            annotations: self
                .snapshot_annotations()
                .into_iter()
                .map(AnnotationPayload::from)
                .collect(),
            frame_width,
            frame_height,
            last_error: self.snapshot_error(),
            mock: self.options.mock,
        }
    }
}
