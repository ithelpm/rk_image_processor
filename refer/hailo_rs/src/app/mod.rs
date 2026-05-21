mod config;
mod image;
mod options;
mod routes;
mod runtime;
mod state;
mod trigger_engine;
mod types;

use opencv::prelude::MatTraitConst;
use std::{ net::SocketAddr, sync::{ Arc, Mutex } };
use tokio::{ net::TcpListener, runtime::Builder, sync::{ watch, Mutex as AsyncMutex } };

use config::{ load_postprocess_config, postprocess_config_path };
use image::{ build_bootstrap_frame, encode_jpeg };
use kmmacro::load_rule_store;
use options::ServerOptions;
use routes::build_router;
use runtime::{ spawn_mock_runtime, start_live_runtime };
use state::{ BackendState, MppJpegState };
use trigger_engine::run_trigger_engine;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = ServerOptions::parse()?;
    let config_path = postprocess_config_path()?;
    let initial_config = load_postprocess_config(&config_path).unwrap_or_default();

    let bootstrap_frame = build_bootstrap_frame()?;
    let (frame_width, frame_height) = (bootstrap_frame.cols(), bootstrap_frame.rows());
    let (frame_tx, _frame_rx) = watch::channel(encode_jpeg(&bootstrap_frame)?);
    let (camera_restart_tx, _camera_restart_rx) = watch::channel::<u64>(0);
    let (annotations_tx, _annotations_rx) = watch::channel::<Vec<String>>(Vec::new());
    let mpp_jpeg = Arc::new(Mutex::new(MppJpegState::from_env()));

    let trigger_rules = Arc::new(Mutex::new(load_rule_store(None).unwrap_or_default().rules));

    let state = BackendState {
        camera_source: Arc::new(Mutex::new(options.camera_source.clone())),
        camera_restart_tx,
        mbus_port: options.mbus_port.clone(),
        mbus_lock: Arc::new(AsyncMutex::new(())),
        options,
        config_path,
        config: Arc::new(Mutex::new(initial_config)),
        manual_rois: Arc::new(Mutex::new(Vec::new())),
        latest_annotations: Arc::new(Mutex::new(Vec::new())),
        latest_frame_size: Arc::new(Mutex::new((frame_width, frame_height))),
        last_error: Arc::new(Mutex::new(None)),
        frame_tx,
        mpp_jpeg,
        annotations_tx,
        trigger_rules,
    };

    if state.options.mock {
        spawn_mock_runtime(state.clone());
    } else {
        start_live_runtime(state.clone())?;
    }

    let runtime = Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(serve(state))?;

    Ok(())
}

async fn serve(state: BackendState) -> Result<(), Box<dyn std::error::Error>> {
    let address: SocketAddr = format!("{}:{}", state.options.host, state.options.port).parse()?;
    let listener = TcpListener::bind(address).await?;

    // 觸發引擎背景任務
    tokio::spawn(run_trigger_engine(state.clone()));

    let app = build_router(state.clone());
    axum::serve(listener, app).await?;
    Ok(())
}
