use std::{ fs, path::{ Path, PathBuf } };

use crate::infer::PostprocessConfig;

pub const CONFIG_FILE_NAME: &str = "postprocess_config.json";
pub const CONFIG_PATH_ENV: &str = "HYPER_MIX_CONFIG_PATH";

pub fn postprocess_config_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(raw) = std::env::var_os(CONFIG_PATH_ENV) {
        let path = PathBuf::from(raw);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        return Ok(path);
    }

    let exe_dir = std::env
        ::current_exe()?
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "無法取得執行檔目錄"))?
        .to_owned();

    Ok(exe_dir.join(CONFIG_FILE_NAME))
}

pub fn load_postprocess_config(
    path: &Path
) -> Result<PostprocessConfig, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(PostprocessConfig::default());
    }
    let file = fs::File::open(path)?;
    Ok(serde_json::from_reader(file)?)
}

pub fn save_postprocess_config(
    path: &Path,
    config: &PostprocessConfig
) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create(path)?;
    serde_json::to_writer_pretty(file, config)?;
    Ok(())
}

/// Lists available V4L2 video devices under `/dev`.
pub fn list_video_devices() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut devices: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if s.starts_with("video") && s["video".len()..].parse::<u32>().is_ok() {
                Some(format!("/dev/{s}"))
            } else {
                None
            }
        })
        .collect();
    devices.sort();
    devices
}
