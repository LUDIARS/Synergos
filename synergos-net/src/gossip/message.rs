use serde::{Deserialize, Serialize};

use crate::types::{ChunkId, FileId, MessageId, PeerId};

/// Gossip メッセージの種類
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// ピアのアクティブ状態
    PeerStatus {
        peer_id: PeerId,
        status: PeerActivityStatus,
        /// この情報を最初に発信したピア
        origin: PeerId,
        /// ホップ数（伝播距離）
        hops: u8,
    },
    /// カタログ更新通知
    CatalogUpdate {
        project_id: String,
        root_crc: u32,
        update_count: u64,
        updated_chunks: Vec<ChunkId>,
    },
    /// ファイル更新要求（受信側が「欲しい」と宣言）
    FileWant {
        requester: PeerId,
        file_id: FileId,
        version: u64,
    },
    /// ファイル送信通知（送信側が「送りたい」と宣言）
    FileOffer {
        sender: PeerId,
        file_id: FileId,
        version: u64,
        size: u64,
        crc: u32,
    },
    /// コンフリクト通知
    ConflictAlert {
        file_id: FileId,
        conflicting_nodes: Vec<PeerId>,
        their_versions: Vec<u64>,
    },
}

/// ピアのアクティビティ状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerActivityStatus {
    pub peer_id: PeerId,
    pub display_name: String,
    pub state: ActivityState,
    /// 最終アクティブ時刻 (Unix timestamp ms)
    pub last_active: u64,
    /// 作業中のファイル（任意）
    pub working_on: Vec<FileId>,
}

/// アクティビティ状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityState {
    /// アクティブに作業中
    Active,
    /// アイドル（接続はしている）
    Idle,
    /// 離席
    Away,
    /// オフライン（他ピアのキャッシュ情報）
    Offline,
}

/// Gossipsub 制御メッセージ
#[derive(Debug, Clone)]
pub enum GossipControl {
    /// メッシュへの参加要求
    Graft,
    /// メッシュからの離脱
    Prune,
    /// 保有メッセージの告知（重複排除用）
    IHave { message_ids: Vec<MessageId> },
    /// メッセージの要求
    IWant { message_ids: Vec<MessageId> },
}
