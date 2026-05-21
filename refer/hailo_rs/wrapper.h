// HailoRT — used by bindgen to generate Rust FFI bindings.
// Headers are searched via:
//   -I/usr/include             (系統安裝路徑)
//   -I<workspace_root>         (從 build.rs 動態加入，讓 <hailo/hailort.h> 被找到)
//   -I<manifest>/aarch64_sysroot/include  (RK sysroot，供 RGA/MPP 標頭使用)
#include <hailo/hailort.h>

// RGA3 和 MPP 透過 hw_shim.c 呼叫，不在此處引入 bindgen，
// 因為這些標頭含有 C++ 預設參數，無法以純 C 模式解析。