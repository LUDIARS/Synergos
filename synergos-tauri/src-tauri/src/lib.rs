//! Synergos Tauri GUI — IPC bridge layer.
//!
//! Tauri command として React フロントエンドに公開する関数群。
//! - `daemon_*` / `project_*` / `peer_*` / `network_*`: synergos-core daemon の
//!   IPC ソケットに接続して `IpcCommand` を投げる
//! - `config_*`: ローカルの `synergos-net.toml` を直接 read/write する
//!
//! daemon が落ちていれば Bridge エラー (`daemon_not_running`) を返す。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use synergos_ipc::{
    command::IpcCommand,
    response::{DaemonStatus, IpcResponse, NetworkStatusInfo, PeerInfo, ProjectInfo, TransferInfo},
    transport::IpcError,
    IpcClient,
};

mod ipc;
use ipc::{call_daemon, BridgeError};

#[derive(Debug, Serialize, Deserialize)]
pub struct InviteToken {
    pub token: String,
    pub expires_at: Option<u64>,
}

// ── daemon ──

#[tauri::command]
async fn daemon_status() -> Result<DaemonStatus, BridgeError> {
    match call_daemon(IpcCommand::Status).await? {
        IpcResponse::Status(s) => Ok(s),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn daemon_ping() -> Result<bool, BridgeError> {
    // daemon が動いていれば true、connect 失敗で false (ユーザに「起動しろ」と言いたい)
    match IpcClient::connect().await {
        Ok(mut c) => match c.send(IpcCommand::Ping).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        },
        Err(IpcError::DaemonNotRunning) => Ok(false),
        Err(e) => Err(BridgeError::Io(e.to_string())),
    }
}

// ── project ──

#[tauri::command]
async fn project_list() -> Result<Vec<ProjectInfo>, BridgeError> {
    match call_daemon(IpcCommand::ProjectList).await? {
        IpcResponse::ProjectList(v) => Ok(v),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn project_open(
    project_id: String,
    root_path: String,
    display_name: Option<String>,
) -> Result<(), BridgeError> {
    let cmd = IpcCommand::ProjectOpen {
        project_id,
        root_path: PathBuf::from(root_path),
        display_name,
    };
    match call_daemon(cmd).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn project_close(project_id: String) -> Result<(), BridgeError> {
    let cmd = IpcCommand::ProjectClose { project_id };
    match call_daemon(cmd).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn project_publish(project_id: String, files: Vec<String>) -> Result<(), BridgeError> {
    let cmd = IpcCommand::PublishUpdate {
        project_id,
        file_paths: files.into_iter().map(PathBuf::from).collect(),
    };
    match call_daemon(cmd).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn project_invite(
    project_id: String,
    expires_in_secs: Option<u64>,
) -> Result<InviteToken, BridgeError> {
    let cmd = IpcCommand::ProjectCreateInvite {
        project_id,
        expires_in_secs,
    };
    match call_daemon(cmd).await? {
        IpcResponse::InviteToken { token, expires_at } => Ok(InviteToken { token, expires_at }),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

// ── peer ──

#[tauri::command]
async fn peer_list(project_id: String) -> Result<Vec<PeerInfo>, BridgeError> {
    let cmd = IpcCommand::PeerList { project_id };
    match call_daemon(cmd).await? {
        IpcResponse::PeerList(v) => Ok(v),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn peer_add_url(project_id: String, url: String) -> Result<(), BridgeError> {
    let cmd = IpcCommand::PeerAddByUrl { project_id, url };
    match call_daemon(cmd).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn peer_disconnect(peer_id: String) -> Result<(), BridgeError> {
    let cmd = IpcCommand::PeerDisconnect { peer_id };
    match call_daemon(cmd).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error { message, .. } => Err(BridgeError::Daemon(message)),
        other => Err(BridgeError::unexpected(other)),
    }
}

// ── network ──

#[tauri::command]
async fn network_status() -> Result<NetworkStatusInfo, BridgeError> {
    match call_daemon(IpcCommand::NetworkStatus).await? {
        IpcResponse::NetworkStatus(s) => Ok(s),
        other => Err(BridgeError::unexpected(other)),
    }
}

#[tauri::command]
async fn transfer_list(project_id: Option<String>) -> Result<Vec<TransferInfo>, BridgeError> {
    let cmd = IpcCommand::TransferList { project_id };
    match call_daemon(cmd).await? {
        IpcResponse::TransferList(v) => Ok(v),
        other => Err(BridgeError::unexpected(other)),
    }
}

// ── config (toml read/write) ──

mod config;
use config::{ConfigFile, ConfigPaths};

#[tauri::command]
async fn config_get() -> Result<ConfigFile, BridgeError> {
    config::load().map_err(|e| BridgeError::Config(e.to_string()))
}

#[tauri::command]
async fn config_set(cfg: ConfigFile) -> Result<(), BridgeError> {
    config::save(&cfg).map_err(|e| BridgeError::Config(e.to_string()))
}

#[tauri::command]
fn config_paths() -> ConfigPaths {
    config::paths()
}

// ── App entry ──

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("synergos_tauri=info")),
        )
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            daemon_ping,
            project_list,
            project_open,
            project_close,
            project_publish,
            project_invite,
            peer_list,
            peer_add_url,
            peer_disconnect,
            network_status,
            transfer_list,
            config_get,
            config_set,
            config_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
