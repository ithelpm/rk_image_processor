use opencv::core::Rect;
use serde::{ Deserialize, Serialize };

use crate::infer::PostprocessConfig;

// ── MBus wire types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MbusDam0888 {
    pub digital_inputs: [bool; 8],
    pub digital_outputs: [bool; 8],
    pub analog_inputs: [f32; 8],
    /// AI2 — Temperature sensor: (Vin/10)*100 − 20  [°C]
    pub temperature_c: f32,
    /// AI3 — Humidity sensor: (Vin/10)*100  [%RH]
    pub humidity_rh: f32,
    /// AI4 — Noise sensor: (Vin/0.01)/10 + 30  [dB]
    pub noise_db: f32,
    /// AI5 — Ultrasonic distance sensor: 50 + Vin/10*350  [mm]
    pub distance_mm: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MbusCd20s {
    pub raw_integer: i16,
    pub float_value: f32,
    /// Vibration velocity in mm/s (IEEE 754 from sensor, direct physical value)
    pub vibration_mms: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MbusSd3788b {
    pub raw: u16,
    pub speed_ms: f32,
    /// Beaufort wind force level (0–12)
    pub wind_force: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MbusSnapshotResponse {
    pub dam0888: Option<MbusDam0888>,
    pub cd20s: Option<MbusCd20s>,
    pub sd3788b: Option<MbusSd3788b>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MbusDoUpdateRequest {
    /// DO 通道索引 0~7，對應 DO1~DO8
    pub channel: u8,
    pub state: bool,
}

// ── Domain type ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OcrAnnotation {
    pub roi: Rect,
    pub text: String,
}

// ── Wire types (HTTP request / response bodies) ───────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RoiPayload {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationPayload {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub host: String,
    pub port: u16,
    pub mock: bool,
    /// Current camera source as a string: "0" for index 0, "/dev/video1" for a path.
    pub camera_source: String,
    pub frame_width: i32,
    pub frame_height: i32,
    pub manual_roi_count: usize,
    pub annotation_count: usize,
    pub last_error: Option<String>,
    pub static_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    pub config: PostprocessConfig,
    pub manual_rois: Vec<RoiPayload>,
    pub annotations: Vec<AnnotationPayload>,
    pub frame_width: i32,
    pub frame_height: i32,
    pub last_error: Option<String>,
    pub mock: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsUpdateRequest {
    pub config: PostprocessConfig,
    pub manual_rois: Vec<RoiPayload>,
    pub persist: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CameraUpdateRequest {
    /// Integer index ("0") or device path ("/dev/video1").
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DevicesResponse {
    /// All `/dev/videoN` devices found on the host.
    pub devices: Vec<String>,
    /// Currently active camera source string.
    pub current: String,
}

// ── Conversions ───────────────────────────────────────────────────────────────

impl From<Rect> for RoiPayload {
    fn from(r: Rect) -> Self {
        Self { x: r.x, y: r.y, width: r.width, height: r.height }
    }
}

impl From<OcrAnnotation> for AnnotationPayload {
    fn from(a: OcrAnnotation) -> Self {
        Self { x: a.roi.x, y: a.roi.y, width: a.roi.width, height: a.roi.height, text: a.text }
    }
}
