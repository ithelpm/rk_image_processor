// src/infer/network.rs
use crate::hailo_ffi::hailo_sys;
use crate::hw::HailoVDevice;
use crate::infer::hef::HailoHef;

pub struct HailoNetworkGroup {
    inner_group: hailo_sys::hailo_configured_network_group,

    // 依然把陣列指標存起來備用
    #[allow(unused)]
    network_groups_array: hailo_sys::hailo_configured_network_group,
    #[allow(unused)]
    network_groups_count: usize,
}

impl HailoNetworkGroup {
    pub fn configure(vdevice: &HailoVDevice, hef: &HailoHef) -> Result<Self, String> {
        unsafe {
            let mut configure_params: hailo_sys::hailo_configure_params_t = std::mem::zeroed();

            let init_status = hailo_sys::hailo_init_configure_params(
                hef.inner_ptr(),
                hailo_sys::hailo_stream_interface_t_HAILO_STREAM_INTERFACE_PCIE, // 告訴底層我們走 PCIe 通道
                &mut configure_params
            );

            if init_status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("初始化配置參數失敗，狀態碼: {}", init_status));
            }

            let mut network_groups_ptr: hailo_sys::hailo_configured_network_group = std::ptr::null_mut();
            let mut network_groups_count: usize = 1;

            // call configure function
            let config_status = hailo_sys::hailo_configure_vdevice(
                vdevice.inner_ptr(),
                hef.inner_ptr(),
                &mut configure_params, // 傳入可變參照，自動轉型為 *mut hailo_configure_params_t
                &mut network_groups_ptr, // 傳入可變參照，自動轉型為剛好的兩層指標
                &mut network_groups_count
            );

            if config_status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("網路配置到晶片失敗: {}", config_status));
            }

            if network_groups_count == 0 || network_groups_ptr.is_null() {
                return Err("配置成功但沒有回傳任何 Network Group".to_string());
            }

            // 由於 network_groups_ptr 本身就指向陣列的開頭，我們直接把它當作第一個 group
            Ok(Self {
                inner_group: network_groups_ptr, // 第一個網路群組
                network_groups_array: network_groups_ptr,
                network_groups_count,
            })
        }
    }

    pub(crate) fn inner_ptr(&self) -> hailo_sys::hailo_configured_network_group {
        self.inner_group
    }
}

// 實作 Drop
impl Drop for HailoNetworkGroup {
    fn drop(&mut self) {
        // 既然找不到專屬的 free 函式，代表資源是由 VDevice 統一管理的。
        // 我們這裡就不去硬叫底層 free 記憶體，避免導致 Double Free 崩潰。
        // 只要 VDevice 被釋放，這裡的資源自然就會乾淨。
    }
}
