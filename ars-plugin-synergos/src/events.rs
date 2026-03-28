//! Ars EventBus イベント定義
//!
//! synergos-core の IPC イベントを Ars EventBus 向けに変換するための型。
//! Synergos が存在しない環境では EventBus::subscribe() が None を返すだけ。

use serde::{Deserialize, Serialize};

/// ピア接続イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConnected {
    pub peer_id: String,
    pub display_name: String,
    pub route: String,
    pub rtt_ms: u32,
}

/// ピア切断イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerDisconnected {
    pub peer_id: String,
    pub reason: DisconnectReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DisconnectReason {
    Graceful,
    Timeout,
    Error(String),
    Remote(String),
}

/// ファイル転送進捗イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferProgress {
    pub transfer_id: String,
    pub peer_id: String,
    pub resource_id: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
}

/// ファイル転送完了イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTransferCompleted {
    pub transfer_id: String,
    pub peer_id: String,
    pub resource_id: String,
    pub file_path: String,
}

/// ネットワーク状態更新イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatusUpdated {
    pub active_connections: u16,
    pub max_connections: u16,
    pub total_bandwidth_bps: u64,
    pub used_bandwidth_bps: u64,
    pub avg_latency_ms: u32,
}

/// コンフリクト検出イベント（Ars EventBus 向け）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictDetected {
    pub file_id: String,
    pub file_path: String,
    pub involved_peers: Vec<String>,
}
