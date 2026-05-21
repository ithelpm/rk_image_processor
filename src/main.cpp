#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <vector>
#include <string>
#include <fstream>
#include <optional>

#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <poll.h>
#include <linux/videodev2.h>

#include "hw/rga.hpp"
#include "hw/mpp_jpeg.hpp"

static constexpr int     DEFAULT_W       = 1920;
static constexpr int     DEFAULT_H       = 1080;
static constexpr int     DEFAULT_FRAMES  = 10;
static constexpr int     JPEG_QUALITY    = 82;
static constexpr int     N_BUFS          = 4;
static constexpr int     POLL_TIMEOUT_MS = 2000;

// ── V4L2 相機 ─────────────────────────────────────────────────────────────────

struct Camera
{
    int      fd     = -1;
    int      width  = 0;
    int      height = 0;
    uint32_t pixfmt = 0;

    struct Buf { void* ptr = nullptr; size_t len = 0; };
    std::vector<Buf> bufs;
};

static void camera_close(Camera& cam)
{
    for (auto& b : cam.bufs)
        if (b.ptr) munmap(b.ptr, b.len);
    if (cam.fd >= 0) close(cam.fd);
    cam = {};
}

// V4L2 fourcc → RGA 格式常數；無對應時回傳 -1
static int v4l2_to_rga_fmt(uint32_t fourcc)
{
    switch (fourcc) {
    case V4L2_PIX_FMT_NV12:  return rga::fmt::YCB_CR_420_SP;
    case V4L2_PIX_FMT_NV16:  return rga::fmt::YCB_CR_422_SP;
    case V4L2_PIX_FMT_BGR24: return rga::fmt::BGR_888;
    default: return -1;
    }
}

static bool camera_open(Camera& cam, const char* device, int want_w, int want_h)
{
    cam.fd = open(device, O_RDWR | O_NONBLOCK);
    if (cam.fd < 0) { std::perror("open"); return false; }

    v4l2_capability cap{};
    if (ioctl(cam.fd, VIDIOC_QUERYCAP, &cap) < 0) {
        std::perror("VIDIOC_QUERYCAP"); return false;
    }
    if (!(cap.capabilities & V4L2_CAP_VIDEO_CAPTURE) ||
        !(cap.capabilities & V4L2_CAP_STREAMING)) {
        std::fprintf(stderr, "[v4l2] 裝置不支援 CAPTURE + STREAMING\n");
        return false;
    }

    // 依序嘗試 RGA 可處理的格式
    static const uint32_t fmt_prefs[] = {
        V4L2_PIX_FMT_NV12,
        V4L2_PIX_FMT_NV16,
        V4L2_PIX_FMT_BGR24,
    };

    for (auto pref : fmt_prefs) {
        v4l2_format fmt{};
        fmt.type                = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        fmt.fmt.pix.width       = static_cast<uint32_t>(want_w);
        fmt.fmt.pix.height      = static_cast<uint32_t>(want_h);
        fmt.fmt.pix.pixelformat = pref;
        fmt.fmt.pix.field       = V4L2_FIELD_ANY;

        if (ioctl(cam.fd, VIDIOC_S_FMT, &fmt) == 0) {
            cam.pixfmt = fmt.fmt.pix.pixelformat;
            cam.width  = static_cast<int>(fmt.fmt.pix.width);
            cam.height = static_cast<int>(fmt.fmt.pix.height);
            char fcc[5]{};
            std::memcpy(fcc, &cam.pixfmt, 4);
            std::printf("[v4l2] 格式: %s  %dx%d\n", fcc, cam.width, cam.height);
            return true;
        }
    }

    std::fprintf(stderr, "[v4l2] 相機不支援 NV12 / NV16 / BGR24，無法使用 RGA\n");
    return false;
}

static bool camera_alloc_bufs(Camera& cam)
{
    v4l2_requestbuffers req{};
    req.count  = N_BUFS;
    req.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    req.memory = V4L2_MEMORY_MMAP;
    if (ioctl(cam.fd, VIDIOC_REQBUFS, &req) < 0) {
        std::perror("VIDIOC_REQBUFS"); return false;
    }

    cam.bufs.resize(req.count);
    for (uint32_t i = 0; i < req.count; ++i) {
        v4l2_buffer buf{};
        buf.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index  = i;
        if (ioctl(cam.fd, VIDIOC_QUERYBUF, &buf) < 0) {
            std::perror("VIDIOC_QUERYBUF"); return false;
        }
        cam.bufs[i].len = buf.length;
        cam.bufs[i].ptr = mmap(nullptr, buf.length,
                               PROT_READ | PROT_WRITE,
                               MAP_SHARED, cam.fd,
                               static_cast<off_t>(buf.m.offset));
        if (cam.bufs[i].ptr == MAP_FAILED) {
            std::perror("mmap"); return false;
        }
    }
    return true;
}

static bool camera_start(Camera& cam)
{
    for (uint32_t i = 0; i < cam.bufs.size(); ++i) {
        v4l2_buffer buf{};
        buf.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index  = i;
        if (ioctl(cam.fd, VIDIOC_QBUF, &buf) < 0) {
            std::perror("VIDIOC_QBUF"); return false;
        }
    }
    uint32_t type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    if (ioctl(cam.fd, VIDIOC_STREAMON, &type) < 0) {
        std::perror("VIDIOC_STREAMON"); return false;
    }
    return true;
}

static void camera_stop(Camera& cam)
{
    uint32_t type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    ioctl(cam.fd, VIDIOC_STREAMOFF, &type);
}

// 取一幀：回傳 kernel buffer index；失敗回傳 -1
static int camera_dequeue(Camera& cam)
{
    pollfd pfd{ cam.fd, POLLIN, 0 };
    int r = poll(&pfd, 1, POLL_TIMEOUT_MS);
    if (r <= 0) { std::fprintf(stderr, "[v4l2] poll timeout/error\n"); return -1; }

    v4l2_buffer buf{};
    buf.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    buf.memory = V4L2_MEMORY_MMAP;
    if (ioctl(cam.fd, VIDIOC_DQBUF, &buf) < 0) {
        std::perror("VIDIOC_DQBUF"); return -1;
    }
    return static_cast<int>(buf.index);
}

static void camera_requeue(Camera& cam, int idx)
{
    v4l2_buffer buf{};
    buf.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.index  = static_cast<uint32_t>(idx);
    ioctl(cam.fd, VIDIOC_QBUF, &buf);
}

// ── 幀處理：RGA 色彩轉換 → MPP JPEG 編碼 → 存檔 ──────────────────────────────

static bool process_frame(const uint8_t* raw, size_t raw_len,
                          int w, int h, uint32_t v4l2_fmt,
                          mpp_jpeg::MppJpegEncoder& enc,
                          const std::string& out_path)
{
    int rga_fmt = v4l2_to_rga_fmt(v4l2_fmt);
    if (rga_fmt < 0) {
        std::fprintf(stderr, "[proc] 不支援的像素格式\n");
        return false;
    }

    // 若相機輸出已是 NV12，直接給 MPP；否則先用 RGA 轉換成 NV12
    std::vector<uint8_t> nv12;
    if (v4l2_fmt == V4L2_PIX_FMT_NV12) {
        nv12.assign(raw, raw + raw_len);
    } else {
        // RGA：NV16 / BGR24 → NV12（色彩轉換，同解析度）
        nv12.resize(static_cast<size_t>(w * h * 3 / 2));
        std::vector<uint8_t> src(raw, raw + raw_len); // RGA 需要可寫指標
        auto res = rga::rga_cvt_resize(
            src,  w, h, rga_fmt,
            nv12, w, h, rga::fmt::YCB_CR_420_SP);
        if (!res) {
            std::fprintf(stderr, "[rga] 轉換失敗: %s\n", res.message().c_str());
            return false;
        }
        std::printf("[rga] 色彩轉換完成 → NV12\n");
    }

    // MPP JPEG 硬體編碼
    auto jpeg = enc.encode(nv12);
    if (!jpeg) {
        std::fprintf(stderr, "[mpp] JPEG 編碼失敗\n");
        return false;
    }
    std::printf("[mpp] JPEG 編碼完成（%zu bytes）→ %s\n",
                jpeg->size(), out_path.c_str());

    std::ofstream ofs(out_path, std::ios::binary);
    ofs.write(reinterpret_cast<const char*>(jpeg->data()),
              static_cast<std::streamsize>(jpeg->size()));
    return true;
}

// ── main ──────────────────────────────────────────────────────────────────────

static void print_usage(const char* prog)
{
    std::fprintf(stderr,
        "用法: %s <裝置> [寬=%d] [高=%d] [幀數=%d]\n"
        "範例: %s /dev/video0 1920 1080 5\n",
        prog, DEFAULT_W, DEFAULT_H, DEFAULT_FRAMES, prog);
}

int main(int argc, char* argv[])
{
    if (argc < 2) { print_usage(argv[0]); return 1; }

    const char* device = argv[1];
    int want_w  = (argc > 2) ? std::atoi(argv[2]) : DEFAULT_W;
    int want_h  = (argc > 3) ? std::atoi(argv[3]) : DEFAULT_H;
    int n_frames= (argc > 4) ? std::atoi(argv[4]) : DEFAULT_FRAMES;

    std::printf("=== RK3588 硬體影像處理 ===\n");
    std::printf("裝置: %s  目標解析度: %dx%d  擷取幀數: %d\n\n",
                device, want_w, want_h, n_frames);

    // ── 1. 開啟相機 ──────────────────────────────────────────────────────────
    Camera cam;
    if (!camera_open(cam, device, want_w, want_h) ||
        !camera_alloc_bufs(cam)) {
        camera_close(cam);
        return 1;
    }

    // ── 2. 建立 MPP JPEG 編碼器（僅 aarch64 有效）────────────────────────────
    auto enc_opt = mpp_jpeg::MppJpegEncoder::create(cam.width, cam.height, JPEG_QUALITY);
    if (!enc_opt) {
        std::fprintf(stderr, "[mpp] 編碼器建立失敗（非 aarch64 或 MPP 初始化錯誤）\n");
        camera_close(cam);
        return 1;
    }
    auto& enc = *enc_opt;
    std::printf("[mpp] 編碼器就緒（NV12 %dx%d，品質=%d）\n\n",
                cam.width, cam.height, JPEG_QUALITY);

    // ── 3. 開始串流 ──────────────────────────────────────────────────────────
    if (!camera_start(cam)) { camera_close(cam); return 1; }
    std::printf("[v4l2] 串流啟動，開始擷取 %d 幀...\n\n", n_frames);

    // ── 4. 擷取迴圈 ──────────────────────────────────────────────────────────
    for (int frame_idx = 0; frame_idx < n_frames; ++frame_idx) {
        int buf_idx = camera_dequeue(cam);
        if (buf_idx < 0) break;

        const auto& b   = cam.bufs[static_cast<size_t>(buf_idx)];
        const auto* raw = static_cast<const uint8_t*>(b.ptr);

        std::string out_path = "frame_" + std::to_string(frame_idx) + ".jpg";
        std::printf("[幀 %d/%d] buf=%d len=%zu\n",
                    frame_idx + 1, n_frames, buf_idx, b.len);

        process_frame(raw, b.len, cam.width, cam.height, cam.pixfmt, enc, out_path);

        camera_requeue(cam, buf_idx);
    }

    // ── 5. 清理 ──────────────────────────────────────────────────────────────
    camera_stop(cam);
    camera_close(cam);
    std::printf("\n完成。\n");
    return 0;
}
