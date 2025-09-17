use axum::{
    extract::{ws::{WebSocket, Message}, State, Path},
};
use futures::{StreamExt, SinkExt};
use crate::AppState;
use tracing::{info, error};
use tokio::process::Command;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{sleep, Duration};

pub async fn terminal_ws_handler(
    ws: WebSocket,
    Path(id): Path<String>,
    State(state): State<AppState>,
) {
    info!("Websocket connection has been made, VM ID: {}", id);

    let ip_address = {
        let workstations = state.workstations.lock().unwrap();
        match workstations.get(&id) {
            Some(w) => Some(w.ip_address.clone()),
            None => None,
        }
    };

    let ip_address = match ip_address {
        Some(ip) => ip,
        None => {
            error!("Workstation not found: {}", id);
            let mut ws = ws;
            if let Err(e) = ws.send(Message::Text("Error: VM not found".to_string().into())).await {
                error!("WebSocket error: {}", e);
            }
            return;
        }
    };
    
    info!("VM IP address: {}", ip_address);
    
    let (mut ws_sender, mut ws_receiver) = ws.split();
    
    info!("Starting SSH connection: {}", ip_address);
    let mut ssh_process = match Command::new("sshpass")
        .arg("-p")
        .arg("root")
        .arg("ssh")
        .arg("-tt")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg("-o")
        .arg("LogLevel=QUIET") 
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg(format!("root@{}", ip_address))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn() {
            Ok(process) => {
                info!("SSH process has been successfully started");
                process
            },
            Err(e) => {
                error!("SSH process could not be started: {}", e);
                if let Err(e) = ws_sender.send(Message::Text(format!("SSH connection could not be established: {}", e).into())).await {
                    error!("WebSocket error: {}", e);
                }
                return;
            }
        };
    
    let stdin = ssh_process.stdin.take().unwrap();
    let mut stdout = ssh_process.stdout.take().unwrap();
    let mut stderr = ssh_process.stderr.take().unwrap();
    
    let ws_sender = Arc::new(TokioMutex::new(ws_sender));
    let stdin = Arc::new(TokioMutex::new(stdin));

    let mut sender = ws_sender.lock().await;
    if let Err(e) = sender.send(Message::Text("SSH connection is being established...\n".into())).await {
        error!("WebSocket response error: {}", e);
        return;
    }
    drop(sender);

    sleep(Duration::from_millis(500)).await;

    // 1. SSH stdout -> WebSocket sender
    let ws_sender_clone = Arc::clone(&ws_sender);
    let stdout_task = tokio::spawn(async move {
        let mut buffer = [0; 1024];
        loop {
            match stdout.read(&mut buffer).await {
                Ok(0) => {
                    info!("SSH stdout EOF received");
                    break;
                },
                Ok(n) => {
                    let raw_output = String::from_utf8_lossy(&buffer[..n]).to_string();
                    info!("SSH stdout output received ({} bytes)", n);

                    let output = raw_output;
                    
                    let mut sender = ws_sender_clone.lock().await;
                    if let Err(e) = sender.send(Message::Text(output.into())).await {
                        error!("WebSocket data sending error: {}", e);
                        break;
                    }
                },
                Err(e) => {
                    error!("SSH stdout read error: {}", e);
                    break;
                }
            }
        }
        info!("SSH stdout reader task completed");
    });

    // 2. SSH stderr -> WebSocket sender
    let ws_sender_clone = Arc::clone(&ws_sender);
    let stderr_task = tokio::spawn(async move {
        let mut buffer = [0; 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => {
                    info!("SSH stderr EOF received");
                    break;
                },
                Ok(n) => {
                    let raw_output = String::from_utf8_lossy(&buffer[..n]).to_string();
                    info!("SSH stderr output received: {}", raw_output);

                    let output = raw_output;
                    
                    let mut sender = ws_sender_clone.lock().await;
                    if let Err(e) = sender.send(Message::Text(output.into())).await {
                        error!("WebSocket data sending error: {}", e);
                        break;
                    }
                },
                Err(e) => {
                    error!("SSH stderr read error: {}", e);
                    break;
                }
            }
        }
        info!("SSH stderr reader task completed");
    });

    // 3. WebSocket -> SSH stdin sender
    let stdin_clone = Arc::clone(&stdin);
    let ws_to_ssh_task = tokio::spawn(async move {

        sleep(Duration::from_secs(1)).await;
        
        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    let command_str = text.to_string();
                    info!("WebSocket command received: {:?}", command_str);
                    
                    let mut stdin = stdin_clone.lock().await;
                    if let Err(e) = stdin.write_all(command_str.as_bytes()).await {
                        error!("SSH stdin write error: {}", e);
                        continue;
                    }
                    
                    if let Err(e) = stdin.flush().await {
                        error!("SSH stdin flush error: {}", e);
                        continue;
                    }
                },
                Ok(Message::Close(_)) => {
                    info!("WebSocket connection closed");
                    break;
                },
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                },
                _ => {}
            }
        }
        info!("WebSocket -> SSH task completed");
    });

    tokio::select! {
        _ = stdout_task => {
            info!("SSH stdout reader task completed");
        },
        _ = stderr_task => {
            info!("SSH stderr reader task completed");
        },
        _ = ws_to_ssh_task => {
            info!("WebSocket -> SSH task completed");
        }
    }
    
    info!("Terminal session terminated, SSH process is being closed");
    let _ = ssh_process.kill().await;
}