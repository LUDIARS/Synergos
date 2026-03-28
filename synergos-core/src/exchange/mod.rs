//! Exchange — ファイル転送制御
//!
//! synergos-net の QUIC ストリームを使ってファイルを送受信する。
//! 優先度キュー、帯域制御、チャンク分割・再組立を管理する。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use std::path::PathBuf;

use async_trait::async_trait;
use dashmap::DashMap;
use synergos_net::types::{Blake3Hash, FileId, PeerId, TransferId};

use crate::event_bus::SharedEventBus;

// ── 型定義 ──

/// 転送の優先度
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferPriority {
    /// ユーザーが明示的に要求した転送
    Interactive = 2,
    /// バックグラウンド同期
    Background = 1,
    /// プリフェッチ（帯域に余裕がある場合のみ）
    Prefetch = 0,
}

/// 転送方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Send,
    Receive,
}

/// 転送状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    /// キュー待ち
    Queued,
    /// 転送中
    Running,
    /// 一時停止
    Paused,
    /// 完了
    Completed,
    /// 失敗
    Failed(String),
    /// キャンセル
    Cancelled,
}

/// ファイル共有リクエスト
#[derive(Debug, Clone)]
pub struct ShareRequest {
    pub project_id: String,
    pub file_id: FileId,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub checksum: Blake3Hash,
    pub priority: TransferPriority,
    /// 送信先ピア（None の場合は Gossipsub で全ピアにブロードキャスト）
    pub target_peer: Option<PeerId>,
}

/// ファイル取得リクエスト
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub project_id: String,
    pub file_id: FileId,
    /// 取得元ピア（None の場合は Gossipsub で Want をブロードキャスト）
    pub source_peer: Option<PeerId>,
    pub priority: TransferPriority,
}

/// アクティブ転送の情報
#[derive(Debug, Clone)]
pub struct ActiveTransfer {
    pub transfer_id: TransferId,
    pub project_id: String,
    pub file_id: FileId,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_transferred: u64,
    pub speed_bps: u64,
    pub direction: TransferDirection,
    pub peer_id: PeerId,
    pub state: TransferState,
    pub priority: TransferPriority,
}

/// ファイル公開通知（ローカル変更をネットワークに公開）
#[derive(Debug, Clone)]
pub struct PublishNotification {
    pub project_id: String,
    pub file_id: FileId,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub crc: u32,
    pub version: u64,
}

/// ファイル共有エラー
#[derive(Debug, thiserror::Error)]
pub enum FileSharingError {
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("transfer not found: {0}")]
    TransferNotFound(String),
    #[error("peer not found: {0}")]
    PeerNotFound(String),
    #[error("transfer already in progress for file: {0}")]
    AlreadyInProgress(String),
    #[error("network error: {0}")]
    NetworkError(String),
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

// ── trait 定義 ──

/// ファイル共有インターフェース
///
/// ファイルの送受信・公開・キャンセルを管理する。
/// - Want/Offer ベースの TransferLedger と連携
/// - Gossipsub 経由で FileWant/FileOffer をブロードキャスト
/// - QUIC ストリームで実データを転送
#[async_trait]
pub trait FileSharing: Send + Sync {
    /// ファイルを共有する（他ピアに送信可能にする / Offer を登録）
    async fn share_file(&self, request: ShareRequest) -> Result<TransferId, FileSharingError>;

    /// ファイルを取得する（他ピアからダウンロード / Want を登録）
    async fn fetch_file(&self, request: FetchRequest) -> Result<TransferId, FileSharingError>;

    /// ローカルファイルの変更をネットワークに公開
    /// （CatalogUpdate + FileOffer を Gossipsub でブロードキャスト）
    async fn publish_updates(
        &self,
        notifications: Vec<PublishNotification>,
    ) -> Result<(), FileSharingError>;

    /// アクティブ転送の一覧を取得
    async fn list_transfers(
        &self,
        project_id: Option<&str>,
    ) -> Vec<ActiveTransfer>;

    /// 転送の詳細を取得
    async fn get_transfer(&self, transfer_id: &TransferId) -> Option<ActiveTransfer>;

    /// 転送をキャンセル
    async fn cancel_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError>;

    /// 転送を一時停止
    async fn pause_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError>;

    /// 転送を再開
    async fn resume_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError>;
}

// ── 実装 ──

/// ファイル転送制御サービス
pub struct Exchange {
    event_bus: SharedEventBus,
    /// アクティブ転送のテーブル
    transfers: DashMap<TransferId, ActiveTransfer>,
}

impl Exchange {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            event_bus,
            transfers: DashMap::new(),
        }
    }
}

#[async_trait]
impl FileSharing for Exchange {
    async fn share_file(&self, request: ShareRequest) -> Result<TransferId, FileSharingError> {
        let transfer_id = TransferId::generate();
        let file_name = request
            .file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| request.file_id.to_string());

        tracing::info!(
            "Sharing file '{}' (id={}, size={})",
            file_name,
            request.file_id,
            request.file_size
        );

        let peer_id = request
            .target_peer
            .clone()
            .unwrap_or_else(|| PeerId::new("broadcast"));

        let transfer = ActiveTransfer {
            transfer_id: transfer_id.clone(),
            project_id: request.project_id,
            file_id: request.file_id,
            file_name,
            file_size: request.file_size,
            bytes_transferred: 0,
            speed_bps: 0,
            direction: TransferDirection::Send,
            peer_id,
            state: TransferState::Queued,
            priority: request.priority,
        };

        self.transfers.insert(transfer_id.clone(), transfer);

        // TODO: TransferLedger に Offer を登録
        // TODO: Gossipsub で FileOffer をブロードキャスト
        // TODO: QUIC ストリームで実データ転送を開始

        Ok(transfer_id)
    }

    async fn fetch_file(&self, request: FetchRequest) -> Result<TransferId, FileSharingError> {
        let transfer_id = TransferId::generate();

        tracing::info!(
            "Fetching file (id={}, project={})",
            request.file_id,
            request.project_id
        );

        let peer_id = request
            .source_peer
            .clone()
            .unwrap_or_else(|| PeerId::new("any"));

        let transfer = ActiveTransfer {
            transfer_id: transfer_id.clone(),
            project_id: request.project_id,
            file_id: request.file_id.clone(),
            file_name: request.file_id.to_string(),
            file_size: 0, // 不明（Offer 受信時に更新）
            bytes_transferred: 0,
            speed_bps: 0,
            direction: TransferDirection::Receive,
            peer_id,
            state: TransferState::Queued,
            priority: request.priority,
        };

        self.transfers.insert(transfer_id.clone(), transfer);

        // TODO: TransferLedger に Want を登録
        // TODO: Gossipsub で FileWant をブロードキャスト

        Ok(transfer_id)
    }

    async fn publish_updates(
        &self,
        notifications: Vec<PublishNotification>,
    ) -> Result<(), FileSharingError> {
        tracing::info!("Publishing {} file update(s)", notifications.len());

        for notif in &notifications {
            tracing::debug!(
                "  - file_id={}, path={}, version={}",
                notif.file_id,
                notif.file_path.display(),
                notif.version
            );
        }

        // TODO: CatalogManager に更新を記録
        // TODO: Gossipsub で CatalogUpdate をブロードキャスト
        // TODO: 各ファイルの FileOffer を TransferLedger に登録

        Ok(())
    }

    async fn list_transfers(
        &self,
        project_id: Option<&str>,
    ) -> Vec<ActiveTransfer> {
        self.transfers
            .iter()
            .filter(|entry| match project_id {
                Some(pid) => entry.value().project_id == pid,
                None => true,
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    async fn get_transfer(&self, transfer_id: &TransferId) -> Option<ActiveTransfer> {
        self.transfers
            .get(transfer_id)
            .map(|entry| entry.value().clone())
    }

    async fn cancel_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError> {
        match self.transfers.get_mut(transfer_id) {
            Some(mut entry) => {
                tracing::info!("Cancelling transfer: {:?}", transfer_id);
                entry.value_mut().state = TransferState::Cancelled;

                // TODO: QUIC ストリームを閉じる
                // TODO: TransferLedger で cancel_want

                Ok(())
            }
            None => Err(FileSharingError::TransferNotFound(format!(
                "{:?}",
                transfer_id
            ))),
        }
    }

    async fn pause_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError> {
        match self.transfers.get_mut(transfer_id) {
            Some(mut entry) => {
                if entry.value().state == TransferState::Running {
                    entry.value_mut().state = TransferState::Paused;
                    tracing::info!("Paused transfer: {:?}", transfer_id);
                }
                Ok(())
            }
            None => Err(FileSharingError::TransferNotFound(format!(
                "{:?}",
                transfer_id
            ))),
        }
    }

    async fn resume_transfer(&self, transfer_id: &TransferId) -> Result<(), FileSharingError> {
        match self.transfers.get_mut(transfer_id) {
            Some(mut entry) => {
                if entry.value().state == TransferState::Paused {
                    entry.value_mut().state = TransferState::Running;
                    tracing::info!("Resumed transfer: {:?}", transfer_id);
                }
                Ok(())
            }
            None => Err(FileSharingError::TransferNotFound(format!(
                "{:?}",
                transfer_id
            ))),
        }
    }
}
