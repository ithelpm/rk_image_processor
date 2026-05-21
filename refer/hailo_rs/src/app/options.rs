use std::path::PathBuf;

const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_PORT: u16 = 8080;
const DEFAULT_CAMERA_INDEX: i32 = 0;

/// Camera input source — either a V4L2 integer index or a filesystem device path.
#[derive(Debug, Clone)]
pub enum CameraSource {
    Index(i32),
    Path(String),
}

impl CameraSource {
    /// Parse from an env-var or CLI value.
    /// Strings starting with `/` are treated as filesystem paths; anything else
    /// is parsed as an integer index (defaulting to 0 on failure).
    pub fn parse(s: &str) -> Self {
        if s.starts_with('/') {
            Self::Path(s.to_owned())
        } else if let Ok(n) = s.parse::<i32>() {
            Self::Index(n)
        } else {
            Self::Index(DEFAULT_CAMERA_INDEX)
        }
    }

    /// Human-readable representation used in API responses.
    pub fn display(&self) -> String {
        match self {
            Self::Index(n) => n.to_string(),
            Self::Path(p) => p.clone(),
        }
    }
}

const DEFAULT_MBUS_PORT: &str = "/dev/ttyUSB0";

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub host: String,
    pub port: u16,
    /// Initial camera source (overridden at runtime via the camera API).
    pub camera_source: CameraSource,
    pub static_dir: Option<PathBuf>,
    pub mock: bool,
    /// RS485 串列埠路徑，用於 Modbus RTU 設備通訊。
    pub mbus_port: String,
}

impl ServerOptions {
    pub fn parse() -> Result<Self, Box<dyn std::error::Error>> {
        let mut options = Self {
            host: std::env::var("HYPER_MIX_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_owned()),
            port: std::env
                ::var("HYPER_MIX_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_PORT),
            camera_source: std::env
                ::var("HYPER_MIX_CAMERA_INDEX")
                .map(|v| CameraSource::parse(&v))
                .unwrap_or(CameraSource::Index(DEFAULT_CAMERA_INDEX)),
            static_dir: std::env::var("HYPER_MIX_STATIC_DIR").ok().map(PathBuf::from),
            mock: std::env
                ::var("HYPER_MIX_MOCK")
                .ok()
                .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            mbus_port: std::env
                ::var("HYPER_MIX_MBUS_PORT")
                .unwrap_or_else(|_| DEFAULT_MBUS_PORT.to_owned()),
        };

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => {
                    options.host = args.next().ok_or("--host 需要一個值")?;
                }
                "--port" => {
                    let v = args.next().ok_or("--port 需要一個值")?;
                    options.port = v.parse()?;
                }
                "--camera-index" => {
                    let v = args.next().ok_or("--camera-index 需要一個值")?;
                    options.camera_source = CameraSource::parse(&v);
                }
                "--static-dir" => {
                    let v = args.next().ok_or("--static-dir 需要一個值")?;
                    options.static_dir = Some(PathBuf::from(v));
                }
                "--mock" => {
                    options.mock = true;
                }
                "--mbus-port" => {
                    let v = args.next().ok_or("--mbus-port 需要一個值")?;
                    options.mbus_port = v;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => {
                    return Err(format!("未知參數: {other}").into());
                }
            }
        }

        if options.static_dir.is_none() {
            options.static_dir = discover_static_dir();
        }

        Ok(options)
    }
}

pub fn print_help() {
    println!(
        "Hyper Mixed Web API\n\n\
          --host <addr>          監聽位址，預設 0.0.0.0\n\
          --port <port>          監聽埠號，預設 8080\n\
          --camera-index <src>   攝影機索引或裝置路徑 (如 0 或 /dev/video0)，預設 0\n\
          --static-dir <path>    前端靜態檔目錄\n\
          --mock                 使用模擬影像與 OCR 結果\n\
          --mbus-port <path>     RS485 串列埠路徑，預設 /dev/ttyUSB0\n"
    );
}

pub fn discover_static_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("frontend"),
        dir.join("../frontend/dist"),
        dir.join("../../frontend/dist"),
    ];
    candidates.into_iter().find(|c| c.join("index.html").exists())
}
