#pragma once

#include <cstdint>
#include <span>
#include <string>
#include "../hw_shim.h"

namespace rga
{
    namespace fmt
    {
        constexpr int YCB_CR_420_SP = 0xa << 8;  // NV12
        constexpr int YCB_CR_422_SP = 0x8 << 8;  // NV16
        constexpr int YCB_CR_444_SP = 0x32 << 8; // NV24
        constexpr int BGR_888 = 0x7 << 8;        // BGR24（OpenCV 預設）
        constexpr int RGB_888 = 0x2 << 8;        // RGB24
    }

    enum class RgaError
    {
        Unsupported,
        Driver,
        BufferTooSmall,
    };

    struct RgaResult
    {
        bool ok;
        RgaError error = RgaError::Unsupported;
        int driver_code = 0; // 只有 Driver 錯誤才有效

        // 模仿 Rust 的 Ok(()) / Err(...)
        static RgaResult success() { return {true}; }
        static RgaResult unsupported() { return {false, RgaError::Unsupported, 0}; }
        static RgaResult driver_err(int code) { return {false, RgaError::Driver, code}; }

        // 讓 if (result) 可以直接用，對應 Rust 的 is_ok()
        explicit operator bool() const { return ok; }

        std::string message() const;

    public:
        RgaResult rga_cvt_resize(std::span<uint8_t> src, int src_w, int src_h, int src_fmt,
                            std::span<uint8_t> dst, int dst_w, int dst_h, int dst_fmt)
        {
            int ret = rk_rga_cvt_resize(src.data(), src_w, src_h, src_fmt,
                                        dst.data(), dst_w, dst_h, dst_fmt);
            if (ret >= 0)
            {
                ok = true;
            }
            else
            {
                driver_err(ret);
            }
        }
    };

    // ── 主要介面 ───────────────────────────────────────────────────
    RgaResult rga_cvt_resize(
        std::span<uint8_t> src, int src_w, int src_h, int src_fmt,
        std::span<uint8_t> dst, int dst_w, int dst_h, int dst_fmt);

} // namespace rga