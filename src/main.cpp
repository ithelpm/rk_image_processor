#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <vector>
#include <string>
#include <fstream>
#include <optional>
#include <chrono>
#include <thread>
#include <mutex>
#include <condition_variable>
#include <limits>

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

using Clock = std::chrono::steady_clock;

static double to_ms(Clock::duration d)
{
    return std::chrono::duration<double, std::milli>(d).count();
}

// 統一錯誤格式：[tag] op: strerror(errno)
static void sys_err(const char* tag, const char* op)
{
    std::fprintf(stderr, "%s %s: %s\n", tag, op, std::strerror(errno));
}

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
    if (cam.fd < 0) { sys_err("[v4l2]", "open"); return false; }

    v4l2_capability cap{};
    if (ioctl(cam.fd, VIDIOC_QUERYCAP, &cap) < 0) {
        sys_err("[v4l2]", "VIDIOC_QUERYCAP"); return false;
    }
    if (!(cap.capabilities & V4L2_CAP_VIDEO_CAPTURE_MPLANE) ||
        !(cap.capabilities & V4L2_CAP_STREAMING)) {
        std::fprintf(stderr, "[v4l2] 裝置不支援 CAPTURE_MPLANE + STREAMING\n");
        return false;
    }

    // 依序嘗試 RGA 可處理的格式（MPLANE 介面）
    static const uint32_t fmt_prefs[] = {
        V4L2_PIX_FMT_NV12,
        V4L2_PIX_FMT_NV16,
        V4L2_PIX_FMT_BGR24,
    };

    for (auto pref : fmt_prefs) {
        v4l2_format fmt{};
        fmt.type                   = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        fmt.fmt.pix_mp.width       = static_cast<uint32_t>(want_w);
        fmt.fmt.pix_mp.height      = static_cast<uint32_t>(want_h);
        fmt.fmt.pix_mp.pixelformat = pref;
        fmt.fmt.pix_mp.field       = V4L2_FIELD_ANY;

        if (ioctl(cam.fd, VIDIOC_S_FMT, &fmt) == 0) {
            cam.pixfmt = fmt.fmt.pix_mp.pixelformat;
            cam.width  = static_cast<int>(fmt.fmt.pix_mp.width);
            cam.height = static_cast<int>(fmt.fmt.pix_mp.height);
            char fcc[5]{};
            std::memcpy(fcc, &cam.pixfmt, 4);
            std::printf("[v4l2] 格式: %s  %dx%d (MPLANE)\n", fcc, cam.width, cam.height);
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
    req.type   = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    req.memory = V4L2_MEMORY_MMAP;
    if (ioctl(cam.fd, VIDIOC_REQBUFS, &req) < 0) {
        sys_err("[v4l2]", "VIDIOC_REQBUFS"); return false;
    }

    cam.bufs.resize(req.count);
    for (uint32_t i = 0; i < req.count; ++i) {
        v4l2_plane  planes[1]{};
        v4l2_buffer buf{};
        buf.type     = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        buf.memory   = V4L2_MEMORY_MMAP;
        buf.index    = i;
        buf.m.planes = planes;
        buf.length   = 1;   // 平面數量
        if (ioctl(cam.fd, VIDIOC_QUERYBUF, &buf) < 0) {
            sys_err("[v4l2]", "VIDIOC_QUERYBUF"); return false;
        }
        cam.bufs[i].len = planes[0].length;
        cam.bufs[i].ptr = mmap(nullptr, planes[0].length,
                               PROT_READ | PROT_WRITE,
                               MAP_SHARED, cam.fd,
                               static_cast<off_t>(planes[0].m.mem_offset));
        if (cam.bufs[i].ptr == MAP_FAILED) {
            sys_err("[v4l2]", "mmap"); return false;
        }
    }
    return true;
}

static bool camera_start(Camera& cam)
{
    for (uint32_t i = 0; i < cam.bufs.size(); ++i) {
        v4l2_plane  planes[1]{};
        v4l2_buffer buf{};
        buf.type     = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
        buf.memory   = V4L2_MEMORY_MMAP;
        buf.index    = i;
        buf.m.planes = planes;
        buf.length   = 1;
        if (ioctl(cam.fd, VIDIOC_QBUF, &buf) < 0) {
            sys_err("[v4l2]", "VIDIOC_QBUF"); return false;
        }
    }
    uint32_t type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    if (ioctl(cam.fd, VIDIOC_STREAMON, &type) < 0) {
        sys_err("[v4l2]", "VIDIOC_STREAMON"); return false;
    }
    return true;
}

static void camera_stop(Camera& cam)
{
    uint32_t type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    ioctl(cam.fd, VIDIOC_STREAMOFF, &type);
}

struct DequeueResult { int idx = -1; size_t bytesused = 0; };

// 取一幀；失敗時 idx == -1
static DequeueResult camera_dequeue(Camera& cam)
{
    pollfd pfd{ cam.fd, POLLIN, 0 };
    int r = poll(&pfd, 1, POLL_TIMEOUT_MS);
    if (r <= 0) { std::fprintf(stderr, "[v4l2] poll timeout/error\n"); return {}; }

    v4l2_plane  planes[1]{};
    v4l2_buffer buf{};
    buf.type     = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    buf.memory   = V4L2_MEMORY_MMAP;
    buf.m.planes = planes;
    buf.length   = 1;
    if (ioctl(cam.fd, VIDIOC_DQBUF, &buf) < 0) {
        sys_err("[v4l2]", "VIDIOC_DQBUF"); return {};
    }
    return { static_cast<int>(buf.index), planes[0].bytesused };
}

static void camera_requeue(Camera& cam, int idx)
{
    v4l2_plane  planes[1]{};
    v4l2_buffer buf{};
    buf.type     = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
    buf.memory   = V4L2_MEMORY_MMAP;
    buf.index    = static_cast<uint32_t>(idx);
    buf.m.planes = planes;
    buf.length   = 1;
    ioctl(cam.fd, VIDIOC_QBUF, &buf);
}

// ── Pipeline 資料結構 ─────────────────────────────────────────────────────────

// Capture thread 複製到 user-space 後傳給 process thread 的幀資料
struct FrameBuffer
{
    std::vector<uint8_t> data;
    int                  frame_idx   = 0;
    double               dequeue_ms  = 0;
    Clock::time_point    captured_at;   // 幀開始時間點，用於計算端對端延遲
};

// 容量為 1 的有界 slot：確保 process thread 跟不上時 capture thread 自然回壓
// 等同於 Rust 參考實作中的 sync_channel(1)
struct BoundedSlot
{
    std::mutex              mtx;
    std::condition_variable cv;
    std::optional<FrameBuffer> item;
    bool closed = false;

    // 推入一幀；若 slot 已滿則阻塞直到 process thread 取走；closed 時回傳 false
    bool push(FrameBuffer&& fb)
    {
        std::unique_lock lk(mtx);
        cv.wait(lk, [&]{ return !item.has_value() || closed; });
        if (closed) return false;
        item = std::move(fb);
        cv.notify_all();
        return true;
    }

    // 取出一幀；若 slot 為空則阻塞直到 capture thread 推入；closed 且空時回傳 false
    bool pop(FrameBuffer& out)
    {
        std::unique_lock lk(mtx);
        cv.wait(lk, [&]{ return item.has_value() || closed; });
        if (!item.has_value()) return false;
        out = std::move(*item);
        item.reset();
        cv.notify_all();
        return true;
    }

    void close()
    {
        std::unique_lock lk(mtx);
        closed = true;
        cv.notify_all();
    }
};

// ── 幀計時 ────────────────────────────────────────────────────────────────────

struct FrameTiming { double rga_ms = 0, encode_ms = 0, write_ms = 0; };
struct FrameStats  { double dequeue_ms, process_ms, total_ms,
                            rga_ms, encode_ms, write_ms; };

// ── 幀處理：RGA 色彩轉換 → MPP JPEG 編碼 → 存檔 ──────────────────────────────

static bool process_frame(const uint8_t* raw, size_t raw_len,
                          int w, int h, uint32_t v4l2_fmt,
                          mpp_jpeg::MppJpegEncoder& enc,
                          const std::string& out_path,
                          FrameTiming* timing)
{
    int rga_fmt = v4l2_to_rga_fmt(v4l2_fmt);
    if (rga_fmt < 0) {
        std::fprintf(stderr, "[proc] 不支援的像素格式\n");
        return false;
    }

    // 若相機輸出已是 NV12，直接給 MPP；否則先用 RGA 轉換成 NV12
    std::vector<uint8_t> nv12;
    auto t0 = Clock::now();
    if (v4l2_fmt == V4L2_PIX_FMT_NV12) {
        nv12.assign(raw, raw + raw_len);
    } else {
        nv12.resize(static_cast<size_t>(w * h * 3 / 2));
        std::vector<uint8_t> src(raw, raw + raw_len);
        auto res = rga::rga_cvt_resize(
            src,  w, h, rga_fmt,
            nv12, w, h, rga::fmt::YCB_CR_420_SP);
        if (!res) {
            std::fprintf(stderr, "[rga] 轉換失敗: %s\n", res.message().c_str());
            return false;
        }
    }
    auto t1 = Clock::now();
    if (timing) timing->rga_ms = to_ms(t1 - t0);

    // MPP JPEG 硬體編碼
    auto jpeg = enc.encode(nv12);
    auto t2 = Clock::now();
    if (timing) timing->encode_ms = to_ms(t2 - t1);
    if (!jpeg) {
        std::fprintf(stderr, "[mpp] JPEG 編碼失敗\n");
        return false;
    }

    std::ofstream ofs(out_path, std::ios::binary);
    ofs.write(reinterpret_cast<const char*>(jpeg->data()),
              static_cast<std::streamsize>(jpeg->size()));
    auto t3 = Clock::now();
    if (timing) timing->write_ms = to_ms(t3 - t2);

    return true;
}

// ── 延遲統計輸出 ──────────────────────────────────────────────────────────────

static void print_stats(const std::vector<FrameStats>& stats)
{
    if (stats.empty()) return;

    struct Col { double avg = 0, min = 0, max = 0; };
    auto gather = [&](auto fn) {
        Col c{ 0, std::numeric_limits<double>::max(), 0 };
        for (auto& s : stats) {
            double v = fn(s);
            c.avg += v;
            if (v < c.min) c.min = v;
            if (v > c.max) c.max = v;
        }
        c.avg /= static_cast<double>(stats.size());
        return c;
    };
    auto pr = [](const char* name, Col c) {
        std::printf(" %-8s: avg=%5.1f  min=%5.1f  max=%5.1f ms\n",
                    name, c.avg, c.min, c.max);
    };

    std::printf("\n=== 延遲統計（%zu 幀）===\n", stats.size());
    pr("dequeue", gather([](auto& s){ return s.dequeue_ms; }));
    pr("rga",     gather([](auto& s){ return s.rga_ms;     }));
    pr("encode",  gather([](auto& s){ return s.encode_ms;  }));
    pr("write",   gather([](auto& s){ return s.write_ms;   }));
    pr("process", gather([](auto& s){ return s.process_ms; }));
    pr("total",   gather([](auto& s){ return s.total_ms;   }));
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

    const char* device  = argv[1];
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

    // ── 4. 雙執行緒 pipeline ──────────────────────────────────────────────────
    //
    //  Capture thread                    Process thread
    //  ─────────────────────────────     ────────────────────────────
    //  camera_dequeue                →   RGA color convert
    //  memcpy to user buffer         →   MPP JPEG encode
    //  camera_requeue (立刻歸還)      →   write to disk
    //  BoundedSlot::push ──────────────► BoundedSlot::pop
    //
    // BoundedSlot 容量為 1，process 跟不上時 capture 自然阻塞（回壓）。

    BoundedSlot           slot;
    std::vector<FrameStats> stats;
    stats.reserve(static_cast<size_t>(n_frames));

    // ── Capture thread ────────────────────────────────────────────────────────
    std::thread capture_th([&] {
        for (int i = 0; i < n_frames; ++i) {
            auto t_start = Clock::now();

            auto dq = camera_dequeue(cam);
            auto t_after_dq = Clock::now();
            if (dq.idx < 0) break;

            const auto& b  = cam.bufs[static_cast<size_t>(dq.idx)];
            size_t raw_len = dq.bytesused ? dq.bytesused : b.len;

            FrameBuffer fb;
            fb.frame_idx  = i;
            fb.dequeue_ms = to_ms(t_after_dq - t_start);
            fb.captured_at = t_start;
            fb.data.assign(static_cast<const uint8_t*>(b.ptr),
                           static_cast<const uint8_t*>(b.ptr) + raw_len);

            // kernel buffer 在複製後立刻歸還，讓驅動程式填充下一幀
            camera_requeue(cam, dq.idx);

            if (!slot.push(std::move(fb))) break;
        }
        slot.close();
    });

    // ── Process thread ────────────────────────────────────────────────────────
    std::thread process_th([&] {
        FrameBuffer fb;
        while (slot.pop(fb)) {
            std::string out_path = "frame_" + std::to_string(fb.frame_idx) + ".jpg";

            FrameTiming ft{};
            process_frame(fb.data.data(), fb.data.size(),
                          cam.width, cam.height, cam.pixfmt,
                          enc, out_path, &ft);

            auto t_end = Clock::now();
            double proc_ms = ft.rga_ms + ft.encode_ms + ft.write_ms;
            double tot_ms  = to_ms(t_end - fb.captured_at);

            std::printf("[幀 %d/%d] dequeue=%.1fms  rga=%.1fms  encode=%.1fms"
                        "  write=%.1fms  total=%.1fms\n",
                        fb.frame_idx + 1, n_frames,
                        fb.dequeue_ms, ft.rga_ms, ft.encode_ms, ft.write_ms, tot_ms);

            stats.push_back({ fb.dequeue_ms, proc_ms, tot_ms,
                              ft.rga_ms, ft.encode_ms, ft.write_ms });
        }
    });

    capture_th.join();
    process_th.join();

    // ── 5. 清理 ──────────────────────────────────────────────────────────────
    camera_stop(cam);
    camera_close(cam);

    print_stats(stats);
    std::printf("\n完成。\n");
    return 0;
}
