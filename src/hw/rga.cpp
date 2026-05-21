#include "rga.hpp"
#include "../hw_shim.h"

namespace rga
{
    std::string RgaResult::message() const
    {
        switch (error)
        {
        case RgaError::Unsupported:    return "RGA: platform not supported";
        case RgaError::Driver:         return "RGA driver error: " + std::to_string(driver_code);
        case RgaError::BufferTooSmall: return "RGA: buffer too small";
        }
        return {};
    }

    RgaResult rga_cvt_resize(std::span<uint8_t> src, int src_w, int src_h, int src_fmt,
                             std::span<uint8_t> dst, int dst_w, int dst_h, int dst_fmt)
#ifdef __aarch64__
    {
        int ret = rk_rga_cvt_resize(src.data(), src_w, src_h, src_fmt,
                                    dst.data(), dst_w, dst_h, dst_fmt);
        return ret >= 0 ? RgaResult::success() : RgaResult::driver_err(ret);
    }
#else
    {
        (void)src; (void)src_w; (void)src_h; (void)src_fmt;
        (void)dst; (void)dst_w; (void)dst_h; (void)dst_fmt;
        return RgaResult::unsupported();
    }
#endif

} // namespace rga
