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
        let entry = self.channels.entry(type_id).or_insert_with(|| {
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

/// Catalog ドリフト検出イベント。gossip CatalogUpdate 受信時、ローカルの
/// `root_crc`/`update_count` がリモートと一致しないときに emit される (#26)。
/// 実際のチャンク取得は Bitswap / transfer で別途行う。
#[derive(Debug, Clone)]
pub struct CatalogSyncNeededEvent {
    pub project_id: String,
    pub remote_root_crc: u32,
    pub remote_update_count: u64,
    /// リモート側で変更があったチャンク ID の文字列リスト
    pub changed_chunks: Vec<String>,
}

impl CoreEvent for CatalogSyncNeededEvent {
    fn event_name() -> &'static str {
        "catalog_sync_needed"
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

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;
    use tokio::sync::broadcast::error::TryRecvError;

    // テスト用の独立したイベント型（他テストとの TypeId 衝突を避けるためここだけで定義）

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct AlphaEvent(u32);
    impl CoreEvent for AlphaEvent {
        fn event_name() -> &'static str {
            "alpha"
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct BetaEvent(String);
    impl CoreEvent for BetaEvent {
        fn event_name() -> &'static str {
            "beta"
        }
    }

    /// T-EB-01: emit_without_subscriber_is_noop — 受信者ゼロでも emit がパニックしない
    #[test]
    fn emit_without_subscriber_is_noop() {
        let bus = CoreEventBus::new();
        // チャンネル未作成状態で emit → 何も起きない
        bus.emit(AlphaEvent(1));
        bus.emit(BetaEvent("x".into()));
        // パニックしなければ OK
    }

    /// T-EB-02: subscribe_then_emit_delivers — 購読 → 発行 → 受信できる
    #[tokio::test(flavor = "current_thread")]
    async fn subscribe_then_emit_delivers() {
        let bus = CoreEventBus::new();
        let mut rx = bus.subscribe::<AlphaEvent>();

        bus.emit(AlphaEvent(42));

        let got = rx.recv().await.unwrap();
        assert_eq!(got, AlphaEvent(42));
    }

    /// T-EB-03: multiple_subscribers_all_receive — 複数受信者にブロードキャストされる
    #[tokio::test(flavor = "current_thread")]
    async fn multiple_subscribers_all_receive() {
        let bus = CoreEventBus::new();
        let mut rx1 = bus.subscribe::<AlphaEvent>();
        let mut rx2 = bus.subscribe::<AlphaEvent>();
        let mut rx3 = bus.subscribe::<AlphaEvent>();

        bus.emit(AlphaEvent(7));

        assert_eq!(rx1.recv().await.unwrap(), AlphaEvent(7));
        assert_eq!(rx2.recv().await.unwrap(), AlphaEvent(7));
        assert_eq!(rx3.recv().await.unwrap(), AlphaEvent(7));
    }

    /// T-EB-04: type_isolation — 型が違うイベントは混線しない
    #[tokio::test(flavor = "current_thread")]
    async fn type_isolation() {
        let bus = CoreEventBus::new();
        let mut alpha_rx = bus.subscribe::<AlphaEvent>();
        let mut beta_rx = bus.subscribe::<BetaEvent>();

        bus.emit(AlphaEvent(100));
        bus.emit(BetaEvent("hello".into()));

        assert_eq!(alpha_rx.recv().await.unwrap(), AlphaEvent(100));
        assert_eq!(beta_rx.recv().await.unwrap(), BetaEvent("hello".into()));

        // 交差受信していないことを確認
        assert_matches!(alpha_rx.try_recv(), Err(TryRecvError::Empty));
        assert_matches!(beta_rx.try_recv(), Err(TryRecvError::Empty));
    }

    /// T-EB-05: late_subscriber_misses_old_events — 発行後に購読した受信者は過去分を受け取らない
    #[tokio::test(flavor = "current_thread")]
    async fn late_subscriber_misses_old_events() {
        let bus = CoreEventBus::new();

        // 早期購読者は受信できるが…
        let mut early = bus.subscribe::<AlphaEvent>();
        bus.emit(AlphaEvent(1));

        // 後から購読した受信者は過去の 1 を受け取らない
        let mut late = bus.subscribe::<AlphaEvent>();

        // 新しい発行は両方受け取る
        bus.emit(AlphaEvent(2));

        assert_eq!(early.recv().await.unwrap(), AlphaEvent(1));
        assert_eq!(early.recv().await.unwrap(), AlphaEvent(2));

        assert_eq!(late.recv().await.unwrap(), AlphaEvent(2));
        // late がもう受信できる値を持っていないことを確認
        assert_matches!(late.try_recv(), Err(TryRecvError::Empty));
    }
}
