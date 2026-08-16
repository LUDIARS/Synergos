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

    /// コンフリクト一覧
    ConflictList(Vec<ConflictInfoDto>),

    /// checkout の結果 (取得要求を出したファイル / 既に一致 / 取得元不明)
    CheckoutReport(CheckoutReportDto),

    /// 履歴ノード上の保持版一覧
    HistoryList(Vec<HistoryVersionDto>),

    /// history gc の結果
    HistoryGcReport(HistoryGcReportDto),

    /// イベント購読完了
    Subscribed { subscription_id: String },
}

/// checkout の結果 (IPC 向け DTO)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CheckoutReportDto {
    /// FileWant(version) を出したファイル (rel_path, version)。実体は非同期に届く
    pub requested: Vec<(String, u64)>,
    /// 作業ツリーが既に manifest と一致していたファイル数
    pub up_to_date: usize,
    /// manifest に無いが作業ツリー / 手元台帳にあるファイル (触らない)
    pub extra: Vec<String>,
}

/// 履歴ノードの保持版 1 件 (IPC 向け DTO)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryVersionDto {
    pub rel_path: String,
    pub version: u64,
    pub hash: String,
    pub size: u64,
    pub crc: u32,
    pub stored_at: u64,
    pub publisher: String,
    pub source: String,
}

/// history gc の結果 (IPC 向け DTO)
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HistoryGcReportDto {
    pub removed_versions: Vec<(String, u64)>,
    pub removed_objects: usize,
    pub bytes_freed: u64,
}

/// コンフリクト情報 (IPC 向け DTO)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfoDto {
    pub file_id: String,
    pub file_path: String,
    pub project_id: String,
    pub local_version: u64,
    pub local_author: String,
    pub remote_version: u64,
    pub remote_author: String,
    pub detected_at: u64,
    pub state: String,
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
    /// 相手 daemon の `CARGO_PKG_VERSION` (peer-info 経由で学習)。
    /// 不明な経路の peer (gossip / DHT のみ) は空文字。
    /// 後方互換のため `#[serde(default)]` で旧 daemon にも追従。
    #[serde(default)]
    pub synergos_version: String,
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
