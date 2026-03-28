//! Exchange: ファイル転送制御
//!
//! Network Foundation Layer の QUIC ストリームを使ってファイルを送受信する。
//! ファイルサイズ別のストリーム占有率制御と優先度キューを提供する。

use synergos_net::types::{FileId, FileSizeClass, PeerId, TransferId};

/// 転送リクエスト
#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub transfer_id: TransferId,
    pub file_id: FileId,
    pub peer_id: PeerId,
    pub file_path: std::path::PathBuf,
    pub file_size: u64,
    pub size_class: FileSizeClass,
    pub priority: TransferPriority,
}

/// 転送優先度
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferPriority {
    /// ユーザーが明示的に要求した転送
    Interactive = 0,
    /// バックグラウンド同期
    Background = 1,
    /// プリフェッチ
    Prefetch = 2,
}

/// 転送方向
#[derive(Debug, Clone, Copy)]
pub enum TransferDirection {
    Upload,
    Download,
}

/// 転送の状態
#[derive(Debug, Clone)]
pub enum TransferState {
    Queued,
    Active {
        bytes_transferred: u64,
        speed_bps: u64,
    },
    Paused {
        reason: String,
    },
    Completed,
    Failed {
        error: String,
    },
}

/// Exchange マネージャ（スタブ）
pub struct Exchange {
    // TODO: Phase 3 で実装
}

impl Exchange {
    pub fn new() -> Self {
        Self {}
    }
}
