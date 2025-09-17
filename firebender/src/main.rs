use axum::{
    routing::{get, post, delete},
    Router
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
    thread,
};
use tracing::{
    info,
    error,
};
use tokio::net::TcpListener;
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};

mod terminal;

mod handlers;
use handlers::{
    root_handler,
    get_workstations_handler,
    create_workstation_handler,
    delete_workstation_handler,
    terminal_ws_handler,
};

mod helpers;
use helpers::{
    create_bridge,
    cleanup_taps,
};

/*----------------------------------------------------------DEFINES----------------------------------------------------------*/

pub const IMAGE_PATH: &str = "/root/projects/firecrack-project/";
pub const KERNEL_IMAGE_PATH: &str = "/root/projects/firecrack-project/kernel.bin";
pub const ROOTFS_IMAGE_PATH: &str = "/root/projects/firecrack-project/rootfs.ext4";

/*----------------------------------------------------------STRUCTS----------------------------------------------------------*/

#[derive(serde::Deserialize)]
struct CreateWorkstationPayload {
    vcpu_count: u64,
    mem_size_mib: u32,
    smt_enabled: bool,
    read_only: bool,
    bandwidth: u64,
}

#[derive(Serialize, Clone)]
struct Workstation {
    id: String,
    ip_address: String,
    order: u32,
    vcpu_count: u64,
    mem_size_mib: u32,
    smt_enabled: bool,
    read_only: bool,
    bandwidth: u64,
}

#[derive(Clone)]
struct AppState {
    workstations: Arc<Mutex<HashMap<String, Workstation>>>,
    vm_counter: Arc<Mutex<u32>>,
}

/*----------------------------------------------------------MAIN----------------------------------------------------------*/

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    info!("Starting Firebender API server...");

    if let Err(e) = create_bridge() {
        error!("Error creating network bridge: {}", e);
        return;
    }

    let app_state = AppState {
        workstations: Arc::new(Mutex::new(HashMap::new())),
        vm_counter: Arc::new(Mutex::new(2)),
    };

    let cors_layer = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
    .route("/", get(root_handler))
    .route("/workstations", get(get_workstations_handler))
    .route("/workstations", post(create_workstation_handler))
    .route("/workstations/{id}", delete(delete_workstation_handler))
    .route("/ws/workstations/{id}/terminal", get(terminal_ws_handler))
    .with_state(app_state.clone())
    .layer(cors_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    info!("Server listening on http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();

    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal)
        .await
        .unwrap();

    info!("Server shutting down. Cleaning up tap devices...");

    thread::sleep(Duration::from_millis(50));

    if let Err(e) = cleanup_taps(&app_state) {
        error!("Error during cleanup: {}", e);
    }

    info!("Cleanup complete. Exiting.");
}