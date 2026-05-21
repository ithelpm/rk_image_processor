# ── Stage 1：建立編譯環境並編譯 ───────────────────────────────────────────────
FROM ubuntu:22.04 AS builder

# clang：交叉編譯器（--target aarch64-linux-gnu）
# gcc-aarch64-linux-gnu：提供 libgcc / crtbegin 等 aarch64 執行期支援檔案
# cmake + ninja-build：建置系統
RUN apt-get update && apt-get install -y --no-install-recommends \
        clang \
        gcc-aarch64-linux-gnu \
        cmake \
        ninja-build \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN cmake -B build \
          -DCMAKE_TOOLCHAIN_FILE=cmake/aarch64-linux-gnu.cmake \
          -DCMAKE_BUILD_TYPE=Release \
          -G Ninja \
    && cmake --build build -j$(nproc)

# ── Stage 2：僅輸出 binary（配合 docker build -o 使用）────────────────────────
FROM scratch AS artifact
COPY --from=builder /src/build/rk3588_demo /rk3588_demo
