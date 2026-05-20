cmake_minimum_required(VERSION 3.22)
project(rk_image_processor C CXX)

set(CMAKE_C_STANDARD 17)
set(CMAKE_CXX_STANDARD 20)

# ── hw_shim.c 單獨編譯成靜態庫（對應 Rust 的 build.rs cc crate）
add_library(hw_shim STATIC src/hw_shim.c)
target_include_directories(hw_shim PUBLIC
    ${CMAKE_SYSROOT}/usr/include   # rga/im2d.h, rockchip/rk_mpi.h
)
target_link_libraries(hw_shim dl)  # dlopen/dlsym 需要 libdl

# ── 主程式（C++）
add_executable(rk3588_demo
    src/hw/rga.cpp
    src/main.cpp
)
target_include_directories(rk3588_demo PRIVATE
    ${CMAKE_SOURCE_DIR}/src
    ${CMAKE_SYSROOT}/usr/include
)
target_link_libraries(rk3588_demo PRIVATE hw_shim)