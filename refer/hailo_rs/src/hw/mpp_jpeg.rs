/// MPP 硬體 JPEG 編碼器（NV12 → JPEG）。
///
/// `MppJpegEncoder` 對應一個固定解析度的編碼器實例。
/// 初始化開銷僅在建立時發生一次，之後每幀編碼只需一次 `encode_put_frame`。
///
/// 僅在 `target_arch = "aarch64"` 時啟用；其餘平台 `new()` 永遠回傳 `None`。
#[allow(unused)]
use std::ffi::c_void;

// ── FFI 宣告 ─────────────────────────────────────────────────────────────────

/// 對應 hw_shim.c 中的 `RkMppJpeg` 結構（不透明指標）。
#[repr(C)]
struct RkMppJpegOpaque {
    _private: [u8; 0],
}

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn rk_mpp_jpeg_create(width: i32, height: i32, quality: i32, format: i32) -> *mut RkMppJpegOpaque;
    fn rk_mpp_jpeg_destroy(enc: *mut RkMppJpegOpaque);
    fn rk_mpp_jpeg_encode(
        enc: *mut RkMppJpegOpaque,
        nv12_data: *const c_void,
        nv12_size: usize,
        out_buf: *mut c_void,
        out_cap: usize
    ) -> i32;
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MppJpegFormat {
    Nv12,
    Nv24,
}

impl MppJpegFormat {
    fn as_mpp_format(self) -> i32 {
        match self {
            Self::Nv12 => 0,
            Self::Nv24 => 15,
        }
    }

    pub fn required_len(self, width: i32, height: i32) -> usize {
        let pixels = (width * height) as usize;
        match self {
            Self::Nv12 => (pixels * 3) / 2,
            Self::Nv24 => pixels * 3,
        }
    }
}

// ── 公開結構 ──────────────────────────────────────────────────────────────────

/// 硬體 JPEG 編碼器包裝。
///
/// 以 NV12（YUV 4:2:0 semi-planar）格式輸入，輸出標準 JPEG 位元串流。
/// 若需要從 BGR 編碼，請先用 [`crate::hw::rga::rga_cvt_resize`] 轉換為 NV12。
pub struct MppJpegEncoder {
    #[allow(unused)]
    ptr: *mut RkMppJpegOpaque,
    width: i32,
    height: i32,
    input_format: MppJpegFormat,
}

// SAFETY: MPP 內部有 mutex；跨執行緒傳遞指標是安全的。
unsafe impl Send for MppJpegEncoder {}
unsafe impl Sync for MppJpegEncoder {}

impl MppJpegEncoder {
    /// 建立編碼器。
    ///
    /// - `width`, `height`：影像解析度（像素）。
    /// - `quality`：JPEG 量化參數（1–99；建議 75–85）。
    ///
    /// 非 aarch64 平台或 MPP 初始化失敗時回傳 `None`。
    pub fn new(width: i32, height: i32, quality: i32) -> Option<Self> {
        Self::new_with_format(width, height, quality, MppJpegFormat::Nv12)
    }

    /// 建立指定輸入像素格式的編碼器。
    pub fn new_with_format(
        width: i32,
        height: i32,
        quality: i32,
        input_format: MppJpegFormat,
    ) -> Option<Self> {
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: rk_mpp_jpeg_create 是 C 函式，回傳 NULL 或有效指標。
            let ptr = unsafe { rk_mpp_jpeg_create(width, height, quality, input_format.as_mpp_format()) };
            if ptr.is_null() {
                None
            } else {
                Some(Self {
                    ptr,
                    width,
                    height,
                    input_format,
                })
            }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = (width, height, quality, input_format);
            None
        }
    }

    /// 將一幀 NV12 / NV24 資料編碼為 JPEG。
    ///
    /// 輸入緩衝區長度至少需符合目前編碼器的像素格式。
    /// 成功時回傳 JPEG 位元串流（`Vec<u8>`），失敗或平台不支援時回傳 `None`。
    pub fn encode(&self, input: &[u8]) -> Option<Vec<u8>> {
        #[cfg(target_arch = "aarch64")]
        {
            let required = self.input_format.required_len(self.width, self.height);
            if input.len() < required {
                return None;
            }
            // 預留充足輸出空間：JPEG 壓縮後往往比原始 NV12 小很多，
            // 用 2× 作為安全上界。
            let out_cap = required * 2;
            let mut out = vec![0u8; out_cap];
            let n = unsafe {
                rk_mpp_jpeg_encode(
                    self.ptr,
                    input.as_ptr() as *const c_void,
                    required,
                    out.as_mut_ptr() as *mut c_void,
                    out_cap
                )
            };
            if n > 0 {
                out.truncate(n as usize);
                Some(out)
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            let _ = input;
            None
        }
    }

    pub fn input_format(&self) -> MppJpegFormat {
        self.input_format
    }

    pub fn required_input_len(&self) -> usize {
        self.input_format.required_len(self.width, self.height)
    }

    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
}

impl Drop for MppJpegEncoder {
    fn drop(&mut self) {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            rk_mpp_jpeg_destroy(self.ptr);
        }
    }
}
