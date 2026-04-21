//! IPC レスポンス定義
//!
//! synergos-core デーモン → クライアントへの応答。

use serde::{Deserialize, Serialize};

/// Core デーモンからクライアントへのレスポンス
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    /// 成功（データなし）
    Ok,

    /// エラー
    Error { code: u32, message: String },

    /// Ping 応答
    Pong,

    /// デーモン状態
    Status(DaemonStatus),

    /// プロジェクト一覧
    ProjectList(Vec<ProjectInfo>),

    /// プロジェクト詳細
    ProjectDetail(ProjectDetail),

    /// 招待トークン
    InviteToken {
        token: String,
        expires_at: Option<u64>,
    },

    /// ピア一覧
    PeerList(Vec<PeerInfo>),

    /// 転送一覧
    TransferList(Vec<TransferInfo>),

    /// ネットワーク状態
    NetworkStatus(NetworkStatusInfo),

    /// イベント購読完了
    Subscribed { subscription_id: String },
}

/// デーモン状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    /// プロセスID
    pub pid: u32,
    /// 起動時刻（Unix epoch 秒）
    pub started_at: u64,
    /// 管理プロジェクト数
    pub project_count: usize,
    /// 総アクティブ接続数
    pub active_connections: usize,
    /// 総アクティブ転送数
    pub active_transfers: usize,
}

/// プロジェクト情報（一覧用サマリ）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub project_id: String,
    pub display_name: String,
    pub root_path: String,
    pub peer_count: usize,
    pub active_transfers: usize,
}

/// プロジェクト詳細情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub project_id: String,
    pub display_name: String,
    pub description: String,
    pub root_path: String,
    pub sync_mode: String,
    pub max_peers: u16,
    pub peer_count: usize,
    pub active_transfers: usize,
    pub created_at: u64,
    /// 接続中のピア一覧（ID のみ）
    pub connected_peer_ids: Vec<String>,
}

/// ピア情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub display_name: String,
    pub route: String,
    pub rtt_ms: u32,
    pub bandwidth_bps: u64,
    pub state: String,
}

/// 転送情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferInfo {
    pub transfer_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_transferred: u64,
    pub speed_bps: u64,
    pub direction: String,
    pub peer_id: String,
    pub state: String,
}

/// ネットワーク状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatusInfo {
    pub primary_route: String,
    pub total_bandwidth_bps: u64,
    pub used_bandwidth_bps: u64,
    pub active_connections: u16,
    pub max_connections: u16,
    pub avg_latency_ms: u32,
}
