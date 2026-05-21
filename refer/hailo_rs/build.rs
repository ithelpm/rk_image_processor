// build.rs
use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir_str = env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_dir = PathBuf::from(&manifest_dir_str);

    // workspace root (hyper_mixed/) — 讓 <hailo/hailort.h> 可被找到
    let workspace_root = manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf();
    // RK sysroot 包含 librga.so / librockchip_mpp.so
    let sysroot_lib = manifest_dir.join("aarch64_sysroot/lib");
    let sysroot_inc = manifest_dir.join("aarch64_sysroot/include");

    // 1. 函式庫搜尋路徑 — 先找 sysroot（交叉編譯），再找系統路徑（原生編譯）
    println!("cargo:rustc-link-search=native={}", sysroot_lib.display());
    println!("cargo:rustc-link-search=native=/usr/aarch64-linux-gnu/lib");
    println!("cargo:rustc-link-search=native=/usr/lib/aarch64-linux-gnu");
    println!("cargo:rustc-link-search=native=/usr/lib/");
    // 2. 告訴 Cargo：如果 wrapper.h 或 hw_shim.c 有修改，請重新執行 build.rs
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=hw_shim.c");

    // 3. 以 cc crate 交叉編譯 hw_shim.c（RGA3 + MPP JPEG shim）
    cc::Build
        ::new()
        .file("hw_shim.c")
        .include(&sysroot_inc)
        .include(&workspace_root) // 讓 shim 能找到 <hailo/hailort.h>（若有需要）
        .flag("-std=c11")
        .flag("-O2")
        .compile("hw_shim");

    // 4. RGA / MPP 改由 hw_shim 在 runtime 以 dlopen 載入，避免交叉編譯時
    //    被 vendor .so 的 GLIBC / GLIBCXX 版本綁住。

    // 5. 告訴編譯器連結名為 `hailort` 的函式庫
    println!("cargo:rustc-link-lib=hailort");

    // 5a. 宣告自訂 cfg 旗標（Rust 1.80+ 的 check-cfg 要求）
    println!("cargo::rustc-check-cfg=cfg(opencv_algorithm_hint)");

    // 5b. 偵測 OpenCV 版本，決定 cvt_color 是否接受第 5 個 AlgorithmHint 參數。
    //     該參數在 OpenCV 4.10.0 加入。讀取 OPENCV_INCLUDE_PATHS 指定的版本標頭。
    detect_opencv_algorithm_hint();

    // 6. 設定並執行 bindgen（僅用於 HailoRT；RGA/MPP 已由 hw_shim.c 處理）
    let bindings = bindgen::Builder
        ::default()
        .header("wrapper.h")
        .clang_arg("-I/usr/include")
        .clang_arg("-I/usr/aarch64-linux-gnu/include")
        .clang_arg(format!("-I{}", workspace_root.display())) // <hailo/hailort.h>
        .clang_arg(format!("-I{}", sysroot_inc.display())) // RK sysroot
        .derive_debug(true)
        .derive_default(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("無法生成 HailoRT bindings!");

    // 7. 將生成的 Rust 程式碼寫入 Cargo 的輸出目錄 (OUT_DIR)
    // 這是編譯時的暫存資料夾，我們不會直接把生成的程式碼 commit 進 Git
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("hailort_bindings.rs"))
        .expect("無法將 bindings 寫入檔案！");
}

/// Emit `cargo:rustc-cfg=opencv_algorithm_hint` when the installed OpenCV is
/// >= 4.10.0 (the release that added the AlgorithmHint parameter to
/// cvt_color and friends).  Reads the version from the OpenCV version header
/// found via the OPENCV_INCLUDE_PATHS environment variable that the Dockerfile
/// already sets for cross-compilation.
fn detect_opencv_algorithm_hint() {
    // Build a list of paths to search: OPENCV_INCLUDE_PATHS first, then common fallbacks.
    let env_paths = env::var("OPENCV_INCLUDE_PATHS").unwrap_or_default();
    let mut search: Vec<String> = env_paths
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    // Fallbacks for host (non-cross) builds where OPENCV_INCLUDE_PATHS is not set.
    for p in &["/usr/local/include/opencv4", "/usr/include/opencv4"] {
        if !search.iter().any(|s| s == *p) {
            search.push(p.to_string());
        }
    }

    for base in &search {
        let version_hpp = PathBuf::from(base).join("opencv2/core/version.hpp");
        if let Ok(content) = std::fs::read_to_string(&version_hpp) {
            let major = parse_version_define(&content, "CV_VERSION_MAJOR");
            let minor = parse_version_define(&content, "CV_VERSION_MINOR");
            if major > 4 || (major == 4 && minor >= 10) {
                println!("cargo:rustc-cfg=opencv_algorithm_hint");
            }
            // Found and parsed the header — stop searching.
            return;
        }
    }
}

/// Extract a numeric value from a line like `#define CV_VERSION_MAJOR 4`.
fn parse_version_define(content: &str, name: &str) -> u32 {
    content
        .lines()
        .find(|l| l.contains(name))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
