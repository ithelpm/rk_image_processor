use crate::{ hailo_ffi::hailo_sys };

pub struct HailoVDevice {
    inner_vdevice: hailo_sys::hailo_vdevice,
}

impl HailoVDevice {
    /// 直接建立虛擬設備，HailoRT 會自動幫我們尋找並綁定底層的實體晶片！
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut params: hailo_sys::hailo_vdevice_params_t = std::mem::zeroed();
            let init_status = hailo_sys::hailo_init_vdevice_params(&mut params);
            if init_status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("初始化 VDevice 參數失敗，狀態碼: {}", init_status));
            }

            let mut vdevice_ptr: hailo_sys::hailo_vdevice = std::ptr::null_mut();

            let status = hailo_sys::hailo_create_vdevice(&mut params, &mut vdevice_ptr);

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                suppress_firmware_logger(vdevice_ptr);
                Ok(Self { inner_vdevice: vdevice_ptr })
            } else {
                Err(format!("建立 VDevice 失敗，狀態碼: {}", status))
            }
        }
    }

    pub(crate) fn inner_ptr(&self) -> hailo_sys::hailo_vdevice {
        self.inner_vdevice
    }
}

fn suppress_firmware_logger(vdevice: hailo_sys::hailo_vdevice) {
    unsafe {
        const MAX_PHYSICAL_DEVICES: usize = 8;

        let mut devices = [std::ptr::null_mut(); MAX_PHYSICAL_DEVICES];
        let mut device_count = devices.len();
        let status = hailo_sys::hailo_get_physical_devices(
            vdevice,
            devices.as_mut_ptr(),
            &mut device_count
        );

        if status != hailo_sys::hailo_status_HAILO_SUCCESS {
            return;
        }

        for device in devices.iter().take(device_count) {
            if device.is_null() {
                continue;
            }

            let disable_status = hailo_sys::hailo_set_fw_logger(
                *device,
                hailo_sys::hailo_fw_logger_level_t_HAILO_FW_LOGGER_LEVEL_FATAL,
                0
            );

            if disable_status != hailo_sys::hailo_status_HAILO_SUCCESS {
                let _ = hailo_sys::hailo_set_fw_logger(
                    *device,
                    hailo_sys::hailo_fw_logger_level_t_HAILO_FW_LOGGER_LEVEL_FATAL,
                    hailo_sys::hailo_fw_logger_interface_t_HAILO_FW_LOGGER_INTERFACE_PCIE
                );
            }
        }
    }
}

impl Drop for HailoVDevice {
    fn drop(&mut self) {
        unsafe {
            if !self.inner_vdevice.is_null() {
                hailo_sys::hailo_release_vdevice(self.inner_vdevice);
            }
        }
    }
}
