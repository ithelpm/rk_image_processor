use crate::hailo_ffi::hailo_sys;
use std::ffi::CString;

pub struct HailoHef {
    inner_hef: hailo_sys::hailo_hef,
}

impl HailoHef {
    /// 方法一：從檔案系統載入 (適合模型很大，或是需要動態抽換模型的場景)
    #[allow(unused)]
    pub fn from_file(file_path: &str) -> Result<Self, String> {
        unsafe {
            let c_path = CString::new(file_path).map_err(|_| "路徑包含無效字元")?;
            let mut hef_ptr: hailo_sys::hailo_hef = std::ptr::null_mut();

            // 注意！新版 API 的接收指標 hef_ptr 變成第一個參數了
            let status = hailo_sys::hailo_create_hef_file(&mut hef_ptr, c_path.as_ptr());

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                Ok(Self { inner_hef: hef_ptr })
            } else {
                Err(format!("從檔案載入 HEF 失敗，狀態碼: {}", status))
            }
        }
    }

    #[allow(dead_code)]
    /// 方法二：從記憶體載入 (適合想要把模型直接打包進執行檔的場景)
    pub fn from_buffer(buffer: &[u8]) -> Result<Self, String> {
        unsafe {
            let mut hef_ptr: hailo_sys::hailo_hef = std::ptr::null_mut();

            // 將 Rust 的 byte slice 轉換為 C 需要的 void* 與長度
            let status = hailo_sys::hailo_create_hef_buffer(
                &mut hef_ptr,
                buffer.as_ptr() as *const std::ffi::c_void, // 轉型為 C 的 void 指標
                buffer.len()
            );

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                Ok(Self { inner_hef: hef_ptr })
            } else {
                Err(format!("從緩衝區載入 HEF 失敗，狀態碼: {}", status))
            }
        }
    }

    pub(crate) fn inner_ptr(&self) -> hailo_sys::hailo_hef {
        self.inner_hef
    }
}

// 根據你的文件註解，釋放函式依然是 hailo_release_hef，所以 Drop 實作不變
impl Drop for HailoHef {
    fn drop(&mut self) {
        unsafe {
            if !self.inner_hef.is_null() {
                hailo_sys::hailo_release_hef(self.inner_hef);
            }
        }
    }
}
