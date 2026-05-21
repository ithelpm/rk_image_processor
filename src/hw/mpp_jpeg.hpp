#pragma once

// MPP hardware JPEG encoder (NV12 / NV24 → JPEG).
//
// MppJpegEncoder wraps one fixed-resolution encoder instance.
// Initialisation cost is paid once in create(); subsequent encode() calls
// reuse a pre-allocated ION buffer to avoid per-frame IOMMU remapping.
//
// Only functional on aarch64 (RK3588). On other platforms create() returns
// std::nullopt so callers can fall back to a software encoder.

#include <cstddef>
#include <cstdint>
#include <memory>
#include <optional>
#include <span>
#include <vector>
#include "../hw_shim.h"

namespace mpp_jpeg
{
    enum class MppJpegFormat
    {
        Nv12, // YUV 4:2:0 semi-planar — lower bandwidth, recommended default
        Nv24, // YUV 4:4:4 semi-planar — full chroma, higher quality
    };

    // Hardware JPEG encoder. Non-copyable; move-only.
    //
    // Thread safety: MPP uses an internal mutex, so the same instance may be
    // accessed from multiple threads without additional locking.
    class MppJpegEncoder
    {
    public:
        // Factory method. Returns nullopt if MPP initialisation fails or on non-aarch64.
        // quality: JPEG quantisation parameter (1–99; 75–85 recommended).
        static std::optional<MppJpegEncoder> create(
            int width, int height, int quality,
            MppJpegFormat format = MppJpegFormat::Nv12);

        // Encode one frame. Returns nullopt if input is too small or encoding fails.
        std::optional<std::vector<uint8_t>> encode(std::span<const uint8_t> input) const;

        MppJpegFormat input_format()      const { return input_format_; }
        std::size_t   required_input_len() const;
        int width()  const { return width_; }
        int height() const { return height_; }

        MppJpegEncoder(const MppJpegEncoder&)            = delete;
        MppJpegEncoder& operator=(const MppJpegEncoder&) = delete;

        MppJpegEncoder(MppJpegEncoder&&)            = default;
        MppJpegEncoder& operator=(MppJpegEncoder&&) = default;

    private:
        // ptr_ owns the C handle; rk_mpp_jpeg_destroy is called automatically on destruction.
        std::unique_ptr<RkMppJpeg, decltype(&rk_mpp_jpeg_destroy)> ptr_;
        int           width_;
        int           height_;
        MppJpegFormat input_format_;

        // Private: use create() to construct.
        MppJpegEncoder(RkMppJpeg* ptr, int width, int height, MppJpegFormat format);
    };

} // namespace mpp_jpeg
