#!/usr/bin/env bash
set -euo pipefail

BUILD_DIR="build"
TOOLCHAIN="cmake/aarch64-linux-gnu.cmake"
BUILD_TYPE="${1:-Release}"

# ── 前置條件檢查 ──────────────────────────────────────────────────────────────
need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "[build] 缺少工具: $1"
        echo "        安裝方式: sudo apt install ${2:-$1}"
        exit 1
    fi
}

need cmake    cmake
need ninja    ninja-build
need clang    clang
need aarch64-linux-gnu-gcc gcc-aarch64-linux-gnu

# ── CMake configure（僅在 build/ 不存在或 CMakeLists.txt 有變動時需要重跑）──
if [ ! -f "${BUILD_DIR}/build.ninja" ]; then
    echo "[build] CMake configure（${BUILD_TYPE}）..."
    cmake -B "${BUILD_DIR}" \
          -DCMAKE_TOOLCHAIN_FILE="${TOOLCHAIN}" \
          -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" \
          -G Ninja
fi

# ── 編譯 ──────────────────────────────────────────────────────────────────────
echo "[build] 編譯中..."
cmake --build "${BUILD_DIR}" -j"$(nproc)"

echo ""
echo "[build] 完成 → ${BUILD_DIR}/rk3588_demo"
echo "        傳板指令範例: scp ${BUILD_DIR}/rk3588_demo user@board:~/"
