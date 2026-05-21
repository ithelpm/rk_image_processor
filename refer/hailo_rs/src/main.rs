mod app;
mod decoder;

// 引入 Hailo 模組
mod hailo_ffi;
mod hw;
mod infer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run()
}
