#pragma once

// RGA3 hardware colour-conversion and scaling interface.
//
// rga_cvt_resize() converts and optionally scales a frame in a single hardware
// pass, combining colour-space conversion and resize without intermediate buffers.
//
// Only active on aarch64 (RK3588). On other architectures every call returns
// RgaError::Unsupported so the caller can fall back to a software path.

#include <cstdint>
#include <span>
#include <string>

namespace rga
{
    // RK_FORMAT_* pixel format constants (from <rga/rga.h>).
    namespace fmt
    {
        constexpr int YCB_CR_420_SP = 0xa << 8;  // NV12 — semi-planar YUV 4:2:0
        constexpr int YCB_CR_422_SP = 0x8 << 8;  // NV16 — semi-planar YUV 4:2:2
        constexpr int YCB_CR_444_SP = 0x32 << 8; // NV24 — semi-planar YUV 4:4:4
        constexpr int BGR_888       = 0x7 << 8;  // BGR24 packed (OpenCV default)
        constexpr int RGB_888       = 0x2 << 8;  // RGB24 packed
    }

    enum class RgaError
    {
        Unsupported,    // Platform does not support RGA (non-aarch64)
        Driver,         // RGA driver returned a non-zero error code
        BufferTooSmall,
    };

    struct RgaResult
    {
        bool     ok;
        RgaError error       = RgaError::Unsupported;
        int      driver_code = 0; // Valid only when error == Driver

        static RgaResult success()            { return {true}; }
        static RgaResult unsupported()        { return {false, RgaError::Unsupported, 0}; }
        static RgaResult driver_err(int code) { return {false, RgaError::Driver, code}; }

        explicit operator bool() const { return ok; }

        std::string message() const;
    };

    // Convert and optionally scale src (src_fmt, src_w×src_h) into dst
    // (dst_fmt, dst_w×dst_h) via the RGA3 hardware engine in a single pass.
    // Format constants are defined in rga::fmt.
    RgaResult rga_cvt_resize(
        std::span<uint8_t> src, int src_w, int src_h, int src_fmt,
        std::span<uint8_t> dst, int dst_w, int dst_h, int dst_fmt);

} // namespace rga
