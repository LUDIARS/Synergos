//! IPC イベント定義
//!
//! synergos-core デーモン → クライアントへのプッシュ通知。
//! Subscribe コマンドで購読を開始すると、該当イベントがリアルタイムに配信される。

use serde::{Deserialize, Serialize};

/// Core デーモンからクライアントへのプッシュイベント
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcEvent {
    /// ピア接続
    PeerConnected {
        project_id: String,
        peer_id: String,
        display_name: String,
        route: String,
        rtt_ms: u32,
    },

    /// ピア切断
    PeerDisconnected {
        project_id: String,
        peer_id: String,
        reason: String,
    },

    /// 転送進捗
    TransferProgress {
        transfer_id: String,
        file_name: String,
        bytes_transferred: u64,
        total_bytes: u64,
        speed_bps: u64,
    },

    /// 転送完了
    TransferCompleted {
        transfer_id: String,
        file_name: String,
        file_path: String,
    },

    /// 転送失敗
    TransferFailed {
        transfer_id: String,
        file_name: String,
        error: String,
    },

    /// コンフリクト検出
    ConflictDetected {
        project_id: String,
        file_id: String,
        file_path: String,
        involved_peers: Vec<String>,
    },

    /// コンフリクト解決
    ConflictResolved {
        project_id: String,
        file_id: String,
        resolution: String,
    },

    /// ネットワーク状態更新
    NetworkStatusUpdated {
        active_connections: u16,
        total_bandwidth_bps: u64,
        used_bandwidth_bps: u64,
        avg_latency_ms: u32,
    },
}

/// イベントフィルタ（購読時に指定）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventFilter {
    /// 全イベントを購読
    All,
    /// 指定プロジェクトのイベントのみ
    Project(String),
    /// 特定カテゴリのみ
    Category(EventCategory),
}

/// イベントカテゴリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventCategory {
    Peer,
    Transfer,
    Conflict,
    Network,
}
