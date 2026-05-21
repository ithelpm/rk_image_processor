//! 觸發引擎背景任務
//!
//! 訂閱 OCR 辨識文字廣播，並定期輪詢 Modbus 快照，
//! 依照已啟用的觸發規則評估條件，條件成立且冷卻時間已過時執行動作。

use std::{ collections::HashMap, time::{ Duration, Instant } };

use tokio::time::interval;

use kmmacro::{ TriggerAction, TriggerCondition, TriggerRule, run_script };
use regex::Regex;
use mbus::{ dam0888_write_do, snapshot_all, AllDevicesResult };
use mbus::devices::dido::RelayChannel;

use super::state::BackendState;

const MBUS_POLL_MS: u64 = 500;

/// 觸發引擎主迴圈，在 tokio task 中運行。
pub async fn run_trigger_engine(state: BackendState) {
    let mut annotations_rx = state.annotations_tx.subscribe();
    let mut mbus_tick = interval(Duration::from_millis(MBUS_POLL_MS));
    // rule_name → 上次觸發時間
    let mut cooldowns: HashMap<String, Instant> = HashMap::new();

    loop {
        tokio::select! {
            // OCR 辨識結果更新時立即評估 OCR 類型規則
            Ok(_) = annotations_rx.changed() => {
                let texts = annotations_rx.borrow().clone();
                let rules = snapshot_rules(&state);
                for rule in &rules {
                    if !rule.enabled { continue; }
                    if matches!(&rule.condition,
                        TriggerCondition::OcrContains { .. } | TriggerCondition::OcrMatches { .. })
                    {
                        if condition_met(&rule.condition, &texts, None) {
                            maybe_fire(&state, rule, &mut cooldowns).await;
                        }
                    }
                }
            }

            // 定時輪詢 Modbus — 評估所有規則（OCR 條件也同時評估）
            _ = mbus_tick.tick() => {
                let texts = annotations_rx.borrow().clone();
                let rules = snapshot_rules(&state);
                if rules.is_empty() { continue; }

                // 取得 Modbus 快照（失敗時跳過本輪 Modbus 規則，但 OCR 規則仍評估）
                let mbus = {
                    let port = state.mbus_port.clone();
                    let _guard = state.mbus_lock.lock().await;
                    snapshot_all(&port).await
                };

                for rule in &rules {
                    if !rule.enabled { continue; }
                    let met = match &rule.condition {
                        TriggerCondition::OcrContains { .. }
                        | TriggerCondition::OcrMatches { .. } => {
                            condition_met(&rule.condition, &texts, None)
                        }
                        _ => condition_met(&rule.condition, &texts, Some(&mbus)),
                    };
                    if met {
                        maybe_fire(&state, rule, &mut cooldowns).await;
                    }
                }
            }
        }
    }
}

// ── 輔助函式 ─────────────────────────────────────────────────────────────────

fn snapshot_rules(state: &BackendState) -> Vec<TriggerRule> {
    state.trigger_rules
        .lock()
        .map(|r| r.clone())
        .unwrap_or_default()
}

fn condition_met(
    cond: &TriggerCondition,
    texts: &[String],
    mbus: Option<&AllDevicesResult>
) -> bool {
    match cond {
        TriggerCondition::OcrContains { text } => {
            texts.iter().any(|t| t.contains(text.as_str()))
        }
        TriggerCondition::OcrEquals { text } => { texts.iter().any(|t| t == text) }
        TriggerCondition::OcrMatches { pattern } => {
            Regex::new(pattern)
                .map(|re| texts.iter().any(|t| re.is_match(t)))
                .unwrap_or(false)
        }
        TriggerCondition::MbusDiHigh { channel } => {
            // channel is 1-indexed (1~8); subtract 1 for array access
            let idx = channel.saturating_sub(1) as usize;
            mbus.and_then(|m| m.dam0888.as_ref().ok())
                .map(|d| d.digital_inputs.get(idx).copied().unwrap_or(false))
                .unwrap_or(false)
        }
        TriggerCondition::MbusDiLow { channel } => {
            let idx = channel.saturating_sub(1) as usize;
            mbus.and_then(|m| m.dam0888.as_ref().ok())
                .map(|d| !d.digital_inputs.get(idx).copied().unwrap_or(true))
                .unwrap_or(false)
        }
        TriggerCondition::MbusAiAbove { channel, threshold } => {
            let idx = channel.saturating_sub(1) as usize;
            mbus.and_then(|m| m.dam0888.as_ref().ok())
                .and_then(|d| d.analog_inputs.get(idx).copied())
                .map(|v| v > *threshold)
                .unwrap_or(false)
        }
        TriggerCondition::MbusAiBelow { channel, threshold } => {
            let idx = channel.saturating_sub(1) as usize;
            mbus.and_then(|m| m.dam0888.as_ref().ok())
                .and_then(|d| d.analog_inputs.get(idx).copied())
                .map(|v| v < *threshold)
                .unwrap_or(false)
        }
        TriggerCondition::WindSpeedAbove { threshold } => {
            mbus.and_then(|m| m.sd3788b.as_ref().ok())
                .map(|w| w.speed_ms > *threshold)
                .unwrap_or(false)
        }
        TriggerCondition::WindSpeedBelow { threshold } => {
            mbus.and_then(|m| m.sd3788b.as_ref().ok())
                .map(|w| w.speed_ms < *threshold)
                .unwrap_or(false)
        }
        TriggerCondition::Cd20sAbove { threshold } => {
            mbus.and_then(|m| m.cd20s.as_ref().ok())
                .map(|v| v.vibration_mms > *threshold)
                .unwrap_or(false)
        }
        TriggerCondition::Cd20sBelow { threshold } => {
            mbus.and_then(|m| m.cd20s.as_ref().ok())
                .map(|v| v.vibration_mms < *threshold)
                .unwrap_or(false)
        }
    }
}

async fn maybe_fire(
    state: &BackendState,
    rule: &TriggerRule,
    cooldowns: &mut HashMap<String, Instant>
) {
    let now = Instant::now();
    if let Some(&last) = cooldowns.get(&rule.name) {
        if now.duration_since(last) < Duration::from_secs(rule.cooldown_secs as u64) {
            return;
        }
    }
    cooldowns.insert(rule.name.clone(), now);
    fire_action(state, &rule.action).await;
}

async fn fire_action(state: &BackendState, action: &TriggerAction) {
    match action {
        TriggerAction::RunMacro { script_name } => {
            let name = script_name.clone();
            let result = tokio::task::spawn_blocking(move || {
                let store = kmmacro::load_store(None).map_err(|e| e.to_string())?;
                let script = store.scripts
                    .into_iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| format!("找不到巨集腳本: {name}"))?;
                run_script(&script)
            }).await;
            if let Err(e) = result.as_ref().map(|r| r.as_ref()) {
                eprintln!("[trigger] 執行巨集失敗: {e:?}");
            }
        }
        TriggerAction::SetMbusDo { channel, value } => {
            let channel = *channel;
            let value = *value;
            // channel is 1-indexed (1~8)
            let relay = match channel {
                1 => RelayChannel::DO1,
                2 => RelayChannel::DO2,
                3 => RelayChannel::DO3,
                4 => RelayChannel::DO4,
                5 => RelayChannel::DO5,
                6 => RelayChannel::DO6,
                7 => RelayChannel::DO7,
                8 => RelayChannel::DO8,
                n => {
                    eprintln!("[trigger] DO 通道 {n} 超出範圍（須為 1~8）");
                    return;
                }
            };
            let port = state.mbus_port.clone();
            let _guard = state.mbus_lock.lock().await;
            if let Err(e) = dam0888_write_do(&port, relay, value).await {
                eprintln!("[trigger] 設定 DO{channel} 失敗: {e}");
            }
        }
    }
}
