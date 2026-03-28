//! Synergos 固有の ArsEvent 定義
//!
//! これらのイベントは Ars の EventBus を通じて他モジュールに通知される。
//! Synergos が存在しない環境では EventBus::subscribe() が None を返すだけ。

use synergos_net::types::{FileId, PeerId, RouteKind, TransferId, Blake3Hash};

/// ピアが接続した
#[derive(Debug, Clone)]
pub struct PeerConnected {
    pub peer_id: PeerId,
    pub display_name: String,
    pub route: RouteKind,
    pub rtt_ms: u32,
}

/// ピアが切断した
#[derive(Debug, Clone)]
pub struct PeerDisconnected {
    pub peer_id: PeerId,
    pub reason: DisconnectReason,
}

#[derive(Debug, Clone)]
pub enum DisconnectReason {
    Graceful,
    Timeout,
    Error(String),
    Remote(String),
}

/// ファイル転送の進捗
#[derive(Debug, Clone)]
pub struct FileTransferProgress {
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub resource_id: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
}

/// ファイル転送が完了した
#[derive(Debug, Clone)]
pub struct FileTransferCompleted {
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub resource_id: String,
    pub file_path: std::path::PathBuf,
    pub checksum: Blake3Hash,
}

/// ネットワーク状況が更新された
#[derive(Debug, Clone)]
pub struct NetworkStatusUpdated {
    pub active_connections: u16,
    pub max_connections: u16,
    pub total_bandwidth_bps: u64,
    pub used_bandwidth_bps: u64,
    pub avg_latency_ms: u32,
}

/// コンフリクトが検出された
#[derive(Debug, Clone)]
pub struct ConflictDetected {
    pub file_id: FileId,
    pub involved_peers: Vec<PeerId>,
}
