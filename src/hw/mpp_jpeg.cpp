#include "mpp_jpeg.hpp"

namespace mpp_jpeg
{
    // Map MppJpegFormat to the integer value expected by rk_mpp_jpeg_create().
    static int as_mpp_format(MppJpegFormat fmt)
    {
        switch (fmt)
        {
        case MppJpegFormat::Nv12: return 0;
        case MppJpegFormat::Nv24: return 15;
        }
        return 0;
    }

    // Minimum input buffer size in bytes for the given format and dimensions.
    static std::size_t required_len(MppJpegFormat fmt, int width, int height)
    {
        std::size_t pixels = static_cast<std::size_t>(width) * height;
        switch (fmt)
        {
        case MppJpegFormat::Nv12: return (pixels * 3) / 2;
        case MppJpegFormat::Nv24: return pixels * 3;
        }
        return 0;
    }

    MppJpegEncoder::MppJpegEncoder(RkMppJpeg* ptr, int width, int height, MppJpegFormat format)
        : ptr_(ptr, &rk_mpp_jpeg_destroy)
        , width_(width)
        , height_(height)
        , input_format_(format)
    {}

    std::optional<MppJpegEncoder> MppJpegEncoder::create(
        int width, int height, int quality, MppJpegFormat format)
    {
#ifdef __aarch64__
        RkMppJpeg* ptr = rk_mpp_jpeg_create(width, height, quality, as_mpp_format(format));
        if (ptr == nullptr)
            return std::nullopt;
        return MppJpegEncoder(ptr, width, height, format);
#else
        (void)width; (void)height; (void)quality; (void)format;
        return std::nullopt;
#endif
    }

    std::optional<std::vector<uint8_t>> MppJpegEncoder::encode(
        std::span<const uint8_t> input) const
    {
#ifdef __aarch64__
        std::size_t required = required_len(input_format_, width_, height_);
        if (input.size() < required)
            return std::nullopt;

        // Output buffer: 2× the raw YUV size is a safe upper bound for JPEG.
        std::size_t out_cap = required * 2;
        std::vector<uint8_t> out(out_cap);

        int n = rk_mpp_jpeg_encode(
            ptr_.get(),
            input.data(), required,
            out.data(),   out_cap);

        if (n <= 0)
            return std::nullopt;

        out.resize(static_cast<std::size_t>(n));
        return out;
#else
        (void)input;
        return std::nullopt;
#endif
    }

    std::size_t MppJpegEncoder::required_input_len() const
    {
        return required_len(input_format_, width_, height_);
    }

} // namespace mpp_jpeg
