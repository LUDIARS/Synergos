//! Exchange — ファイル転送制御
//!
//! synergos-net の QUIC ストリームを使ってファイルを送受信する。
//! 優先度キュー、帯域制御、チャンク分割・再組立を管理する。
//!
//! TransferLedger と Gossipsub を統合し、Want/Offer ベースの転送制御を行う。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dashmap::DashMap;
use synergos_net::chain::{LedgerEntryState, OfferEntry, TransferLedger, WantEntry, LedgerAction};
use synergos_net::gossip::{GossipMessage, GossipNode};
use synergos_net::types::{Blake3Hash, FileId, PeerId, TopicId, TransferId};

use crate::event_bus::{SharedEventBus, TransferProgressEvent, TransferCompletedEvent};

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
    /// このバージョンを TransferLedger / Gossip へ伝播する。
    /// 呼び出し側（PublishUpdate 経由）が把握していない場合は 1 を使う。
    pub version: u64,
}

/// ファイル取得リクエスト
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub project_id: String,
    pub file_id: FileId,
    /// 取得元ピア（None の場合は Gossipsub で Want をブロードキャスト）
    pub source_peer: Option<PeerId>,
    pub priority: TransferPriority,
    /// 欲しいバージョン。0 は「任意の最新」を意味する。
    pub version: u64,
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
    /// このアクティブ転送が担当するファイルバージョン。
    /// 転送完了時に TransferLedger へ反映される。
    pub version: u64,
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
#[async_trait]
pub trait FileSharing: Send + Sync {
    /// ファイルを共有する（他ピアに送信可能にする / Offer を登録）
    async fn share_file(&self, request: ShareRequest) -> Result<TransferId, FileSharingError>;

    /// ファイルを取得する（他ピアからダウンロード / Want を登録）
    async fn fetch_file(&self, request: FetchRequest) -> Result<TransferId, FileSharingError>;

    /// ローカルファイルの変更をネットワークに公開
    async fn publish_updates(
        &self,
        notifications: Vec<PublishNotification>,
    ) -> Result<(), FileSharingError>;

    /// アクティブ転送の一覧を取得
    async fn list_transfers(&self, project_id: Option<&str>) -> Vec<ActiveTransfer>;

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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// ファイル転送制御サービス
pub struct Exchange {
    event_bus: SharedEventBus,
    /// アクティブ転送のテーブル
    transfers: DashMap<TransferId, ActiveTransfer>,
    /// Want/Offer 転送台帳
    ledger: Arc<TransferLedger>,
    /// Gossipsub ノード（オプション: ネットワーク初期化後にセット）
    gossip: Option<Arc<GossipNode>>,
    /// ローカルピアID
    local_peer_id: PeerId,
}

impl Exchange {
    /// 最小構成のコンストラクタ（テスト・後方互換用）
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self::with_network(event_bus, PeerId::new("local"), None)
    }

    /// ネットワーク依存を注入して構築する本番向けコンストラクタ
    pub fn with_network(
        event_bus: SharedEventBus,
        local_peer_id: PeerId,
        gossip: Option<Arc<GossipNode>>,
    ) -> Self {
        Self {
            event_bus,
            transfers: DashMap::new(),
            ledger: Arc::new(TransferLedger::new()),
            gossip,
            local_peer_id,
        }
    }

    /// 完了済み/キャンセル済み転送をテーブルから除去
    pub fn gc_finished_transfers(&self) -> usize {
        let mut removed = 0;
        self.transfers.retain(|_, t| match &t.state {
            TransferState::Completed
            | TransferState::Cancelled
            | TransferState::Failed(_) => {
                removed += 1;
                false
            }
            _ => true,
        });
        removed
    }

    /// TransferLedger への参照を取得
    pub fn ledger(&self) -> &Arc<TransferLedger> {
        &self.ledger
    }

    /// Gossipsub 経由で FileOffer をブロードキャスト
    fn broadcast_offer(
        &self,
        project_id: &str,
        file_id: &FileId,
        version: u64,
        file_size: u64,
        crc: u32,
    ) {
        if let Some(gossip) = &self.gossip {
            let topic = TopicId::project(project_id);
            gossip.publish(
                &topic,
                GossipMessage::FileOffer {
                    sender: self.local_peer_id.clone(),
                    file_id: file_id.clone(),
                    version,
                    size: file_size,
                    crc,
                },
            );
        }
    }

    /// Gossipsub 経由で FileWant をブロードキャスト
    fn broadcast_want(&self, project_id: &str, file_id: &FileId, version: u64) {
        if let Some(gossip) = &self.gossip {
            let topic = TopicId::project(project_id);
            gossip.publish(
                &topic,
                GossipMessage::FileWant {
                    requester: self.local_peer_id.clone(),
                    file_id: file_id.clone(),
                    version,
                },
            );
        }
    }

    /// Gossipsub 経由で CatalogUpdate をブロードキャスト
    fn broadcast_catalog_update(
        &self,
        project_id: &str,
        root_crc: u32,
        update_count: u64,
    ) {
        if let Some(gossip) = &self.gossip {
            let topic = TopicId::project(project_id);
            gossip.publish(
                &topic,
                GossipMessage::CatalogUpdate {
                    project_id: project_id.to_string(),
                    root_crc,
                    update_count,
                    updated_chunks: vec![],
                },
            );
        }
    }

    /// 転送完了を処理
    pub fn complete_transfer(&self, transfer_id: &TransferId) {
        if let Some(mut entry) = self.transfers.get_mut(transfer_id) {
            let transfer = entry.value_mut();
            transfer.state = TransferState::Completed;
            transfer.bytes_transferred = transfer.file_size;

            // EventBus に完了イベントを発行
            self.event_bus.emit(TransferCompletedEvent {
                transfer_id: transfer_id.0.clone(),
                file_name: transfer.file_name.clone(),
                file_path: String::new(),
            });

            // TransferLedger で fulfilled をマーク
            self.ledger.mark_fulfilled(
                &transfer.file_id,
                transfer.version,
                &transfer.peer_id,
            );
        }
    }

    /// 転��進捗を更新
    pub fn update_progress(
        &self,
        transfer_id: &TransferId,
        bytes_transferred: u64,
        speed_bps: u64,
    ) {
        if let Some(mut entry) = self.transfers.get_mut(transfer_id) {
            let transfer = entry.value_mut();
            transfer.bytes_transferred = bytes_transferred;
            transfer.speed_bps = speed_bps;

            if transfer.state == TransferState::Queued {
                transfer.state = TransferState::Running;
            }

            // EventBus に進捗イベントを発行
            self.event_bus.emit(TransferProgressEvent {
                transfer_id: transfer_id.0.clone(),
                file_name: transfer.file_name.clone(),
                bytes_transferred,
                total_bytes: transfer.file_size,
                speed_bps,
            });
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
            project_id: request.project_id.clone(),
            file_id: request.file_id.clone(),
            file_name,
            file_size: request.file_size,
            bytes_transferred: 0,
            speed_bps: 0,
            direction: TransferDirection::Send,
            peer_id,
            state: TransferState::Queued,
            priority: request.priority,
            version: request.version,
        };

        self.transfers.insert(transfer_id.clone(), transfer);

        // TransferLedger に Offer を登録
        let offer = OfferEntry {
            sender: self.local_peer_id.clone(),
            file_id: request.file_id.clone(),
            version: request.version,
            file_size: request.file_size,
            crc: crc32fast::hash(&request.checksum.0),
            offered_at: now_ms(),
            state: LedgerEntryState::Pending,
        };
        let actions = self.ledger.register_offer(offer);
        tracing::debug!("Ledger offer registered, actions: {}", actions.len());

        // Gossipsub で FileOffer をブロードキャスト
        self.broadcast_offer(
            &request.project_id,
            &request.file_id,
            request.version,
            request.file_size,
            crc32fast::hash(&request.checksum.0),
        );

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
            project_id: request.project_id.clone(),
            file_id: request.file_id.clone(),
            file_name: request.file_id.to_string(),
            file_size: 0,
            bytes_transferred: 0,
            speed_bps: 0,
            direction: TransferDirection::Receive,
            peer_id,
            state: TransferState::Queued,
            priority: request.priority,
            version: request.version,
        };

        self.transfers.insert(transfer_id.clone(), transfer);

        // TransferLedger に Want を登録
        let want = WantEntry {
            requester: self.local_peer_id.clone(),
            file_id: request.file_id.clone(),
            version: request.version,
            requested_at: now_ms(),
            state: LedgerEntryState::Pending,
        };
        let action = self.ledger.register_want(want);

        match &action {
            LedgerAction::Match { sender, file_size } => {
                tracing::info!(
                    "Immediate match found: sender={}, size={}",
                    sender,
                    file_size
                );
                // マッチ成立 → 転送を開始状態にする
                if let Some(mut entry) = self.transfers.get_mut(&transfer_id) {
                    entry.value_mut().state = TransferState::Running;
                    entry.value_mut().file_size = *file_size;
                    entry.value_mut().peer_id = sender.clone();
                }
            }
            LedgerAction::Duplicate => {
                tracing::debug!("Duplicate want for file {}", request.file_id);
            }
            LedgerAction::Queued => {
                tracing::debug!("Want queued for file {}", request.file_id);
            }
        }

        // Gossipsub で FileWant をブロードキャスト
        self.broadcast_want(&request.project_id, &request.file_id, 0);

        Ok(transfer_id)
    }

    async fn publish_updates(
        &self,
        notifications: Vec<PublishNotification>,
    ) -> Result<(), FileSharingError> {
        tracing::info!("Publishing {} file update(s)", notifications.len());

        let mut total_crc: u32 = 0;

        for notif in &notifications {
            tracing::debug!(
                "  - file_id={}, path={}, version={}",
                notif.file_id,
                notif.file_path.display(),
                notif.version
            );

            // TransferLedger に各ファ��ルの Offer を登録
            let offer = OfferEntry {
                sender: self.local_peer_id.clone(),
                file_id: notif.file_id.clone(),
                version: notif.version,
                file_size: notif.file_size,
                crc: notif.crc,
                offered_at: now_ms(),
                state: LedgerEntryState::Pending,
            };
            self.ledger.register_offer(offer);

            // Gossipsub で FileOffer をブロードキャスト
            self.broadcast_offer(
                &notif.project_id,
                &notif.file_id,
                notif.version,
                notif.file_size,
                notif.crc,
            );

            total_crc = crc32fast::hash(&total_crc.to_le_bytes());
        }

        // Gossipsub で CatalogUpdate をブロードキャスト
        if let Some(first) = notifications.first() {
            self.broadcast_catalog_update(
                &first.project_id,
                total_crc,
                notifications.len() as u64,
            );
        }

        Ok(())
    }

    async fn list_transfers(&self, project_id: Option<&str>) -> Vec<ActiveTransfer> {
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
                let transfer = entry.value_mut();
                transfer.state = TransferState::Cancelled;

                // TransferLedger で Want をキャンセル
                if transfer.direction == TransferDirection::Receive {
                    self.ledger.cancel_want(
                        &transfer.file_id,
                        0,
                        &self.local_peer_id,
                    );
                }

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
