cmake_minimum_required(VERSION 3.22)
project(rk_image_processor C CXX)

set(CMAKE_C_STANDARD   17)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ── hw_shim：C 層，以 dlopen 在執行期載入 librga / librockchip_mpp ────────────
# 編譯為靜態庫，讓 C++ 程式碼不需直接依賴這兩個 .so 的標頭細節。
# hw_shim.c 需要 sysroot 內的 <rga/im2d.h> 和 <rockchip/rk_mpi.h>，
# 因此將 sysroot include 設為 PUBLIC，主程式目標也會繼承。
add_library(hw_shim STATIC hw_shim.c)
target_include_directories(hw_shim PUBLIC
    ${CMAKE_SYSROOT}/include
)
target_link_libraries(hw_shim PUBLIC dl)

# ── rk3588_demo：主程式 ──────────────────────────────────────────────────────
add_executable(rk3588_demo
    src/main.cpp
    src/hw/rga.cpp
    src/hw/mpp_jpeg.cpp
)
target_include_directories(rk3588_demo PRIVATE
    ${CMAKE_SOURCE_DIR}/src       # 讓 #include "hw/rga.hpp" 等路徑生效
)
target_link_libraries(rk3588_demo PRIVATE hw_shim)
