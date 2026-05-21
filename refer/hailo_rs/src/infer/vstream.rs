use crate::hailo_ffi::hailo_sys;
use crate::infer::network::HailoNetworkGroup;
use std::os::raw::c_void;

pub struct HailoVStreams {
    input_vstreams: Vec<hailo_sys::hailo_input_vstream>,
    output_vstreams: Vec<hailo_sys::hailo_output_vstream>,
}

// 告訴編譯器：這個結構體可以在執行緒之間安全地移動。
// 因為我們會在 Worker Thread 使用它，所以必須實作 Send。
unsafe impl Send for HailoVStreams {}

impl HailoVStreams {
    pub fn create(network_group: &HailoNetworkGroup) -> Result<Self, String> {
        unsafe {
            // ==========================================
            // 1. 建立進水管 (Input VStreams)
            // ==========================================
            const MAX_INPUTS: usize = 4;
            let mut input_params: [
                hailo_sys::hailo_input_vstream_params_by_name_t;
                MAX_INPUTS
            ] = std::mem::zeroed();
            let mut input_count: usize = MAX_INPUTS;

            let status = hailo_sys::hailo_make_input_vstream_params(
                network_group.inner_ptr(),
                true,
                hailo_sys::hailo_format_type_t_HAILO_FORMAT_TYPE_AUTO,
                input_params.as_mut_ptr(),
                &mut input_count
            );

            if status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("產生 Input 參數失敗: {}", status));
            }

            // 根據實際需要的數量，動態建立一個填滿 NULL 的 Vec 陣列
            let mut input_vstreams = vec![std::mem::zeroed(); input_count];

            let status = hailo_sys::hailo_create_input_vstreams(
                network_group.inner_ptr(),
                input_params.as_ptr(),
                input_count,
                // 傳入 Vec 的底層記憶體指標，HailoRT 會把建好的水管放進去！
                input_vstreams.as_mut_ptr()
            );

            if status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("建立 Input VStreams 失敗: {}", status));
            }

            // ==========================================
            // 2. 建立Output VStreams
            // ==========================================
            const MAX_OUTPUTS: usize = 8;
            let mut output_params: [
                hailo_sys::hailo_output_vstream_params_by_name_t;
                MAX_OUTPUTS
            ] = std::mem::zeroed();
            let mut output_count: usize = MAX_OUTPUTS;

            let status = hailo_sys::hailo_make_output_vstream_params(
                network_group.inner_ptr(),
                true,
                hailo_sys::hailo_format_type_t_HAILO_FORMAT_TYPE_AUTO,
                output_params.as_mut_ptr(),
                &mut output_count
            );

            if status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("產生 Output 參數失敗: {}", status));
            }

            let mut output_vstreams = vec![std::mem::zeroed(); output_count];

            let status = hailo_sys::hailo_create_output_vstreams(
                network_group.inner_ptr(),
                output_params.as_ptr(),
                output_count,
                output_vstreams.as_mut_ptr()
            );

            if status != hailo_sys::hailo_status_HAILO_SUCCESS {
                return Err(format!("建立 Output VStreams 失敗: {}", status));
            }

            Ok(Self {
                input_vstreams,
                output_vstreams,
            })
        }
    }

    pub fn write_input(&self, data_ptr: *const u8, data_size: usize) -> Result<(), String> {
        unsafe {
            if self.input_vstreams.is_empty() {
                return Err("沒有可用的 Input VStream".to_string());
            }

            // 直接從 Vec 中取出第 0 根水管
            let first_input = self.input_vstreams[0];

            let status = hailo_sys::hailo_vstream_write_raw_buffer(
                first_input,
                data_ptr as *const c_void,
                data_size
            );

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                Ok(())
            } else {
                Err(format!("寫入失敗: {}", status))
            }
        }
    }

    pub fn read_output(&self, buffer_ptr: *mut u8, buffer_size: usize) -> Result<(), String> {
        unsafe {
            if self.output_vstreams.is_empty() {
                return Err("沒有可用的 Output VStream".to_string());
            }

            let first_output = self.output_vstreams[0];

            let status = hailo_sys::hailo_vstream_read_raw_buffer(
                first_output,
                buffer_ptr as *mut c_void,
                buffer_size
            );

            if status == hailo_sys::hailo_status_HAILO_SUCCESS {
                Ok(())
            } else {
                Err(format!("讀取失敗: {}", status))
            }
        }
    }
}

impl Drop for HailoVStreams {
    fn drop(&mut self) {
        unsafe {
            if !self.input_vstreams.is_empty() {
                hailo_sys::hailo_release_input_vstreams(
                    self.input_vstreams.as_mut_ptr(),
                    self.input_vstreams.len()
                );
            }
            if !self.output_vstreams.is_empty() {
                hailo_sys::hailo_release_output_vstreams(
                    self.output_vstreams.as_mut_ptr(),
                    self.output_vstreams.len()
                );
            }
        }
    }
}
