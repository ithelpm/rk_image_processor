set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)

# 交叉編譯器
set(CMAKE_C_COMPILER   clang)
set(CMAKE_CXX_COMPILER clang++)

set(CMAKE_C_COMPILER_TARGET   aarch64-linux-gnu)
set(CMAKE_CXX_COMPILER_TARGET aarch64-linux-gnu)

set(CMAKE_EXE_LINKER_FLAGS    "--gcc-toolchain=/usr")
set(CMAKE_CXX_FLAGS           "--gcc-toolchain=/usr")

# 對應 Rust build.rs 的 aarch64_sysroot
set(CMAKE_SYSROOT ${CMAKE_CURRENT_LIST_DIR}/../aarch64_sysroot)
set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)