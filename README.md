# A Sample Code for RK3588 from NanoPC T6 Hardware Encode/Decode


## How to build

```bash
cmake -B build \
  -DCMAKE_TOOLCHAIN_FILE=cmake/aarch64-linux-gnu.cmake \
  -DCMAKE_BUILD_TYPE=Release

cmake --build build -j$(nproc)
```