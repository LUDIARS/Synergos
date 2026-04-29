//! Tauri command → synergos-core daemon IPC client wrapper.
//!
//! IpcClient::connect() を毎呼び出しごとに張る (短命 RPC)。
//! daemon が起動していなければ `daemon_not_running` を返す。
//!
//! 共有 IpcClient を持たないのは:
//! - Tauri command は async で複数同時に呼ばれる → mutex 競合で順序問題が出る
//! - daemon は再起動される可能性があり connection 持ち越しは fragile
//! - 1 回の RPC は ms オーダーなので reconnect cost は許容

use serde::Serialize;
use synergos_ipc::{command::IpcCommand, response::IpcResponse, transport::IpcError, IpcClient};

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "snake_case")]
pub enum BridgeError {
    #[error("daemon is not running")]
    DaemonNotRunning,
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("unexpected daemon response: {0}")]
    Unexpected(String),
}

impl BridgeError {
    pub fn unexpected(resp: IpcResponse) -> Self {
        Self::Unexpected(format!("{:?}", resp))
    }
}

pub async fn call_daemon(cmd: IpcCommand) -> Result<IpcResponse, BridgeError> {
    let mut client = IpcClient::connect().await.map_err(map_ipc_err)?;
    client.send(cmd).await.map_err(map_ipc_err)
}

fn map_ipc_err(e: IpcError) -> BridgeError {
    match e {
        IpcError::DaemonNotRunning => BridgeError::DaemonNotRunning,
        other => BridgeError::Io(other.to_string()),
    }
}
