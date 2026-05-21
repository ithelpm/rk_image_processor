/// RGA3 硬體加速色彩轉換與縮放。
///
/// 透過 hw_shim.c（hw_shim 靜態函式庫）呼叫 librga。
/// 僅在 `target_arch = "aarch64"` 時啟用；其餘平台會回傳 `RgaUnsupported` 錯誤。
#[allow(unused)]
use std::ffi::c_int;

/// RK_FORMAT_* 像素格式常數（定義於 <rga/rga.h>）
pub mod fmt {
    pub const YCB_CR_420_SP: i32 = 0xa << 8; // NV12
    pub const YCB_CR_422_SP: i32 = 0x8 << 8; // NV16
    pub const YCB_CR_444_SP: i32 = 0x32 << 8; // NV24
    pub const BGR_888: i32 = 0x7 << 8; // BGR24（OpenCV 預設）
    pub const RGB_888: i32 = 0x2 << 8; // RGB24
}

#[derive(Debug)]
pub enum RgaError {
    /// 目標平台不支援 RGA（非 aarch64）
    Unsupported,
    /// RGA driver 回傳錯誤碼
    #[allow(unused)]
    Driver(i32),
    /// 輸入緩衝區太小
    #[allow(unused)]
    BufferTooSmall,
}

impl std::fmt::Display for RgaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RgaError::Unsupported => write!(f, "RGA 不支援此平台"),
            RgaError::Driver(n) => write!(f, "RGA driver 錯誤: {n}"),
            RgaError::BufferTooSmall => write!(f, "RGA 緩衝區大小不足"),
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    fn rk_rga_cvt_resize(
        src_va: *mut std::ffi::c_void,
        src_w: c_int,
        src_h: c_int,
        src_fmt: c_int,
        dst_va: *mut std::ffi::c_void,
        dst_w: c_int,
        dst_h: c_int,
        dst_fmt: c_int
    ) -> c_int;
}

/// 將 `src` 緩衝區（`src_fmt` 格式，`src_w × src_h`）透過 RGA3
/// 色彩轉換（並選擇性縮放）後寫入 `dst`（`dst_fmt` 格式，`dst_w × dst_h`）。
///
/// RGA 會在同一個 pass 完成格式轉換 + 縮放，比分開呼叫 OpenCV 更省 CPU。
pub fn rga_cvt_resize(
    src: &mut [u8],
    src_w: i32,
    src_h: i32,
    src_fmt: i32,
    dst: &mut [u8],
    dst_w: i32,
    dst_h: i32,
    dst_fmt: i32
) -> Result<(), RgaError> {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: 兩個 slice 都是有效的、對齊的記憶體區段；
        // RGA driver 會透過 DMA 讀取 src 並寫入 dst。
        let ret = unsafe {
            rk_rga_cvt_resize(
                src.as_mut_ptr() as *mut _,
                src_w,
                src_h,
                src_fmt,
                dst.as_mut_ptr() as *mut _,
                dst_w,
                dst_h,
                dst_fmt
            )
        };
        if ret >= 0 {
            Ok(())
        } else {
            Err(RgaError::Driver(ret))
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (src, src_w, src_h, src_fmt, dst, dst_w, dst_h, dst_fmt);
        Err(RgaError::Unsupported)
    }
}
