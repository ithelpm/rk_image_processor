# cmake/aarch64-linux-gnu.cmake
#
# 交叉編譯 toolchain 設定，目標：aarch64 Linux（如 NanoPC-T6 / RK3588）。
#
# 使用方式：
#   cmake -B build \
#         -DCMAKE_TOOLCHAIN_FILE=cmake/aarch64-linux-gnu.cmake \
#         -DCMAKE_BUILD_TYPE=Release
#   cmake --build build -j$(nproc)
#
# 前置條件：
#   - 主機已安裝 clang（支援 --target aarch64-linux-gnu）
#   - 主機已安裝 gcc-aarch64-linux-gnu（提供 libgcc、crtbegin 等執行期支援）
#   - 專案根目錄下有 aarch64_sysroot/（含目標板 /usr/lib, /usr/include）

set(CMAKE_SYSTEM_NAME      Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)

# ── 編譯器 ───────────────────────────────────────────────────────────────────
# 使用 clang 進行交叉編譯；透過 --target 指定目標三元組，
# 而非維護獨立的 aarch64-linux-gnu-gcc 安裝路徑。
set(CMAKE_C_COMPILER   clang)
set(CMAKE_CXX_COMPILER clang++)

set(CMAKE_C_COMPILER_TARGET   aarch64-linux-gnu)
set(CMAKE_CXX_COMPILER_TARGET aarch64-linux-gnu)

# ── GCC 工具鏈支援 ────────────────────────────────────────────────────────────
# clang 本身不附帶 libgcc / crtbegin 等低階執行期支援檔案；
# --gcc-toolchain=/usr 讓 clang 從系統安裝的 gcc-aarch64-linux-gnu 取得這些檔案。
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} --gcc-toolchain=/usr")
set(CMAKE_CXX_FLAGS        "${CMAKE_CXX_FLAGS}        --gcc-toolchain=/usr")

# ── Sysroot（目標板的根檔案系統子集）────────────────────────────────────────
# aarch64_sysroot 應包含目標板的 /usr/include 與 /usr/lib，
# 使 CMake 在此目錄而非主機路徑搜尋標頭檔與函式庫。
set(CMAKE_SYSROOT        ${CMAKE_CURRENT_LIST_DIR}/../aarch64_sysroot)
set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT})

# 主機工具（如 pkg-config）仍在主機路徑搜尋；
# 函式庫與標頭檔只在 sysroot 內搜尋，避免意外連結到主機的 x86 版本。
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
