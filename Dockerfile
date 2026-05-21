# Cross-compile toolchain image for rk_image_processor (target: aarch64-linux-gnu / RK3588)
#
# Build the image once:
#   docker build -t rk3588-builder .
#
# Then cross-compile (source + aarch64_sysroot are mounted at runtime):
#   docker run --rm -v "$(pwd)":/workspace rk3588-builder
#
# Or use the convenience script:
#   ./build.sh
#
# Output binary: build-aarch64/rk3588_demo

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

# clang        — cross-compiler (built with multi-target support, includes aarch64)
# gcc-aarch64-linux-gnu  — provides libgcc, crtbegin.o and AArch64 glibc headers
# g++-aarch64-linux-gnu  — provides AArch64 C++ STL headers
# cmake + ninja-build    — build system
RUN apt-get update && apt-get install -y --no-install-recommends \
        clang \
        gcc-aarch64-linux-gnu \
        g++-aarch64-linux-gnu \
        cmake \
        ninja-build \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Default command: configure (into a dedicated aarch64 build dir) then build.
# Keeping this as CMD (not ENTRYPOINT) so users can override with a shell for debugging:
#   docker run --rm -v "$(pwd)":/workspace rk3588-builder bash
CMD ["bash", "-c", \
     "cmake -B build-aarch64 \
            -DCMAKE_TOOLCHAIN_FILE=cmake/aarch64-linux-gnu.cmake \
            -DCMAKE_BUILD_TYPE=Release \
            -G Ninja \
     && cmake --build build-aarch64 -j$(nproc)"]
