//! 内部 EventBus
//!
//! Ars EventBus とは独立した、synergos-core 内部のイベント配信機構。
//! 型安全なブロードキャストチャンネルベースのパブサブを提供する。

use dashmap::DashMap;
use std::any::{Any, TypeId};
use std::sync::Arc;
use tokio::sync::broadcast;

/// コアイベント trait
///
/// EventBus で配信可能なイベント型が実装する。
pub trait CoreEvent: Clone + Send + Sync + 'static {
    /// イベント名（ログ・デバッグ用）
    fn event_name() -> &'static str;
}

/// チャンネルサイズ（バッファ上限）
const CHANNEL_CAPACITY: usize = 256;

/// 内部 EventBus
///
/// 型ごとにブロードキャストチャンネルを管理する。
/// `emit()` で発行されたイベントは、対応する `subscribe()` の
/// 全受信者にブロードキャストされる。
pub struct CoreEventBus {
    channels: DashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl CoreEventBus {
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
        }
    }

    /// イベントを発行する
    ///
    /// 受信者がいない場合は何も起こらない（ドロップ）。
    pub fn emit<E: CoreEvent>(&self, event: E) {
        let type_id = TypeId::of::<E>();
        if let Some(entry) = self.channels.get(&type_id) {
            if let Some(tx) = entry.downcast_ref::<broadcast::Sender<E>>() {
                // 受信者がいない場合のエラーは無視
                let _ = tx.send(event);
            }
        }
    }

    /// イベントを購読する
    ///
    /// 初回購読時にチャンネルが自動作成される。
    pub fn subscribe<E: CoreEvent>(&self) -> broadcast::Receiver<E> {
        let type_id = TypeId::of::<E>();
        let entry = self
            .channels
            .entry(type_id)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel::<E>(CHANNEL_CAPACITY);
                Box::new(tx)
            });
        entry
            .downcast_ref::<broadcast::Sender<E>>()
            .expect("type mismatch in EventBus")
            .subscribe()
    }
}

impl Default for CoreEventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ── 内部イベント型定義 ──

/// ピア接続イベント
#[derive(Debug, Clone)]
pub struct PeerConnectedEvent {
    pub project_id: String,
    pub peer_id: String,
    pub display_name: String,
    pub route: String,
    pub rtt_ms: u32,
}

impl CoreEvent for PeerConnectedEvent {
    fn event_name() -> &'static str {
        "peer_connected"
    }
}

/// ピア切断イベント
#[derive(Debug, Clone)]
pub struct PeerDisconnectedEvent {
    pub project_id: String,
    pub peer_id: String,
    pub reason: String,
}

impl CoreEvent for PeerDisconnectedEvent {
    fn event_name() -> &'static str {
        "peer_disconnected"
    }
}

/// 転送進捗イベント
#[derive(Debug, Clone)]
pub struct TransferProgressEvent {
    pub transfer_id: String,
    pub file_name: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
}

impl CoreEvent for TransferProgressEvent {
    fn event_name() -> &'static str {
        "transfer_progress"
    }
}

/// 転送完了イベント
#[derive(Debug, Clone)]
pub struct TransferCompletedEvent {
    pub transfer_id: String,
    pub file_name: String,
    pub file_path: String,
}

impl CoreEvent for TransferCompletedEvent {
    fn event_name() -> &'static str {
        "transfer_completed"
    }
}

/// コンフリクト検出イベント
#[derive(Debug, Clone)]
pub struct ConflictDetectedEvent {
    pub project_id: String,
    pub file_id: String,
    pub file_path: String,
    pub involved_peers: Vec<String>,
}

impl CoreEvent for ConflictDetectedEvent {
    fn event_name() -> &'static str {
        "conflict_detected"
    }
}

/// ネットワーク状態更新イベント
#[derive(Debug, Clone)]
pub struct NetworkStatusEvent {
    pub active_connections: u16,
    pub total_bandwidth_bps: u64,
    pub used_bandwidth_bps: u64,
    pub avg_latency_ms: u32,
}

impl CoreEvent for NetworkStatusEvent {
    fn event_name() -> &'static str {
        "network_status_updated"
    }
}

/// EventBus の共有参照型
pub type SharedEventBus = Arc<CoreEventBus>;
