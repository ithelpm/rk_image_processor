# rk_image_processor

A minimal C++ sample demonstrating **hardware-accelerated image processing** on the
RK3588 SoC (NanoPC-T6 and compatible boards) using:

- **RGA3** — single-pass colour-space conversion and scaling (no CPU involvement)
- **MPP** — hardware JPEG encoding (NV12/NV24 → JPEG)
- **V4L2** — camera capture with format negotiation

Both RGA and MPP are loaded at runtime via `dlopen`, so the binary can be built on
a standard x86 host and deployed to the target without compile-time library
dependencies.

---

## Prerequisites

| Requirement | Notes |
|---|---|
| clang (host) | Must support `--target aarch64-linux-gnu` |
| gcc-aarch64-linux-gnu (host) | Provides `libgcc`, `crtbegin`, etc. for the linker |
| `aarch64_sysroot/` | Target board's `/usr` tree (headers + libs); place at project root |

Install the cross-toolchain on Ubuntu/Debian:
```bash
sudo apt install clang gcc-aarch64-linux-gnu
```

Populate `aarch64_sysroot/` by copying `/usr` from the target board, or by
extracting the relevant packages from the board's package repository.

---

## Build

```bash
cmake -B build \
      -DCMAKE_TOOLCHAIN_FILE=cmake/aarch64-linux-gnu.cmake \
      -DCMAKE_BUILD_TYPE=Release

cmake --build build -j$(nproc)
```

The output binary is `build/rk3588_demo`.

---

## Usage

```bash
# Capture 10 frames from /dev/video0 at 1920×1080 (defaults)
./rk3588_demo /dev/video0

# Specify resolution and frame count
./rk3588_demo /dev/video0 1280 720 5
```

Each captured frame is saved as `frame_0.jpg`, `frame_1.jpg`, …

The program negotiates the camera pixel format in this order:
**NV12 → NV16 → BGR24**. If none is available the program exits with an error.
Formats not natively supported by RGA (e.g. YUYV) are intentionally excluded.

---

## Project structure

```
.
├── hw_shim.c / hw_shim.h     C shim that dlopen-loads librga and librockchip_mpp
├── src/
│   ├── main.cpp              V4L2 capture loop → RGA → MPP JPEG
│   └── hw/
│       ├── rga.hpp / .cpp    RGA3 colour-conversion + scaling wrapper (C++20)
│       └── mpp_jpeg.hpp / .cpp  MPP JPEG encoder wrapper (RAII, move-only)
├── cmake/
│   └── aarch64-linux-gnu.cmake  Cross-compilation toolchain file
└── aarch64_sysroot/          Target sysroot (not tracked in git)
```

---

## Hardware pipeline

```
V4L2 frame (NV12 / NV16 / BGR24)
        │
        │  [if not NV12] RGA colour conversion → NV12
        ▼
   MPP JPEG encode
        │
        ▼
   frame_N.jpg
```

For AI preprocessing (resize + colour-space in one pass):
```
NV12 frame (e.g. 1920×1080)
        │
        │  RGA resize + NV12→BGR in a single pass
        ▼
   BGR frame (e.g. 960×544) ready for inference
```

---

## Runtime library loading

`hw_shim.c` uses `dlopen` to load `librga.so` and `librockchip_mpp.so` at
runtime.  This means:

- The binary has **no hard link-time dependency** on these libraries.
- If the libraries are absent (e.g. on a development host), `create()` and
  `rga_cvt_resize()` return failure gracefully; callers can fall back to a
  software path.
- No `pkg-config` or sysroot `.so` stubs are required for the build.
