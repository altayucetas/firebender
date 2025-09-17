use axum::{
    extract::{Path, State, ws::WebSocketUpgrade},
    response::{IntoResponse, Json},
    http::StatusCode,
};

use crate::{
    AppState,
    CreateWorkstationPayload,
    Workstation,
};

use tracing::{
    info,
    error,
};

use uuid::Uuid;

use crate::terminal;
use crate::helpers::{
    connect_vms_to_network,
    spawn_firecracker_process,
    configure_vm,
    shutdown_vm,
};

/*----------------------------------------------------------HANDLERS----------------------------------------------------------*/

pub async fn root_handler() -> &'static str {
    info!("Request received at root endpoint.");
    
    "Hello, this is the Firebender API!\n"
}

pub async fn get_workstations_handler(
    State(state): State<AppState>
) -> impl IntoResponse {
    info!("Get workstations request received.");

    let workstations_map = state.workstations.lock().unwrap();
    let workstations = workstations_map.values().cloned().collect::<Vec<_>>();

    Json(workstations)
}

pub async fn create_workstation_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkstationPayload>,
) -> impl IntoResponse {
    info!("Create workstation request received.");

    if payload.vcpu_count <= 0 || payload.mem_size_mib <= 0 {
        let error_response = serde_json::json!({ "error": "Invalid vCPU or Memory values. vCPU must be > 0 and Memory must be > 0." });
        return (StatusCode::BAD_REQUEST, Json(error_response)).into_response();
    }

    let mut vm_counter = state.vm_counter.lock().unwrap();
    let current_vm_counter = *vm_counter;
    *vm_counter += 1;

    let vm_id = Uuid::new_v4().to_string();

    let workstation = Workstation {
        id: vm_id.clone(),
        ip_address: format!("172.16.0.{}", current_vm_counter),
        order: current_vm_counter,
        vcpu_count: payload.vcpu_count,
        mem_size_mib: payload.mem_size_mib,
        smt_enabled: payload.smt_enabled,
        read_only: payload.read_only,
        bandwidth: payload.bandwidth,
    };

    info!("Creating workstation with ID: {}, IP: {}", workstation.id, workstation.ip_address);

    if let Err(e) = connect_vms_to_network(current_vm_counter) {
        error!("Error connecting VM to network: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR, 
            Json(serde_json::json!({"error": e}))
        ).into_response();
    }

    let socket_path = match spawn_firecracker_process(&vm_id) {
        Ok(path) => path,
        Err(e) => {
            error!("Error spawning Firecracker process: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    if let Err(e) = configure_vm(&socket_path, &vm_id, &workstation.ip_address, current_vm_counter, 
        payload.vcpu_count, payload.mem_size_mib, payload.smt_enabled, payload.read_only, payload.bandwidth) {
        error!("Error configuring VM: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response();
    }

    let mut workstations_map = state.workstations.lock().unwrap();
    workstations_map.insert(workstation.id.clone(), workstation.clone());

    (StatusCode::CREATED, Json(workstation)).into_response()
}

pub async fn delete_workstation_handler(
    State(state): State<AppState>,
    Path(workstation_id): Path<String>,
) -> impl IntoResponse {
    info!("Delete workstation request received for ID: {}", workstation_id);

    let mut workstations_map = state.workstations.lock().unwrap();

    if let Some(workstation) = workstations_map.remove(&workstation_id) {
        if let Err(e) = shutdown_vm(&workstation) {
            error!("Error shutting down VM: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }

        (StatusCode::OK, Json(serde_json::json!({"status": format!("Workstation with ID {} deleted", workstation.id)}))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": format!("Workstation with ID {} not found", workstation_id)}))).into_response()
    }
}

pub async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        terminal::terminal_ws_handler(socket, Path(id), State(state)).await
    })
}