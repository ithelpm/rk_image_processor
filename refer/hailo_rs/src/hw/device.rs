use std::ffi::{ CString, c_char };

use crate::hailo_ffi::hailo_sys;

#[allow(dead_code)]
/// HailoDevice structure in Rust
pub struct HailoDevice {
    /// 儲存好讀的 PCIe BDF 字串 (例如 "0000:01:00.0")，方便 Log 和 UI 顯示
    bdf_id: String,

    /// C指標在這裡
    inner_device: hailo_sys::hailo_device,
}

#[allow(dead_code)]
impl HailoDevice {
    /// 建構子：自動掃描並連線第一張找到的卡片
    pub fn scan_and_connect() -> Result<Self, String> {
        unsafe {
            // 1. 掃描設備
            const MAX_DEVICES: usize = 4;
            let mut pcie_infos: [
                hailo_sys::hailo_pcie_device_info_t;
                MAX_DEVICES
            ] = std::mem::zeroed();
            let mut count: usize = 0;

            let status = hailo_sys::hailo_scan_pcie_devices(
                pcie_infos.as_mut_ptr(),
                MAX_DEVICES,
                &mut count
            );
            if status != hailo_sys::hailo_status_HAILO_SUCCESS || count == 0 {
                return Err("找不到 Hailo 設備或掃描失敗".to_string());
            }

            // 2. 格式化字串
            let info = pcie_infos[0];
            let bdf_string = format!(
                "{:04x}:{:02x}:{:02x}.{:x}",
                info.domain,
                info.bus,
                info.device,
                info.func
            );

            // 3. 呼叫底下的指定 ID 連線邏輯
            Self::connect_by_id(&bdf_string)
        }
    }

    /// 建構子：允許手動傳入 ID 字串來連線
    pub fn connect_by_id(bdf: &str) -> Result<Self, String> {
        unsafe {
            let c_str = CString::new(bdf).map_err(|_| "字串轉換 CString 失敗")?;
            let mut device_id: hailo_sys::hailo_device_id_t = std::mem::zeroed();

            for (i, &byte) in c_str.as_bytes_with_nul().iter().enumerate() {
                if i >= device_id.id.len() {
                    break;
                }
                device_id.id[i] = byte as c_char;
            }

            let mut device_ptr: hailo_sys::hailo_device = std::ptr::null_mut();
            let status = hailo_sys::hailo_create_device_by_id(&device_id, &mut device_ptr);

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                // 回傳封裝好的 Struct
                Ok(Self {
                    bdf_id: bdf.to_string(),
                    inner_device: device_ptr,
                })
            } else {
                Err(format!("連線設備失敗，狀態碼: {}", status))
            }
        }
    }

    /// Getter：取得設備 ID，UI 要顯示狀態時很好用
    pub fn id(&self) -> &str {
        &self.bdf_id
    }

    /// 內部 Getter：未來我們建立 VDevice 時，會需要把這個底層指標傳進去
    /// 使用 pub(crate) 限制只有我們自己的專案可以使用，防止外部亂搞
    pub(crate) fn inner_ptr(&self) -> hailo_sys::hailo_device {
        self.inner_device
    }
}
