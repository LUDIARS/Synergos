use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::broadcast;

use super::message::canonical_bytes;
use super::message::*;
use super::transport::OutboundGossip;
use crate::config::GossipsubConfig;
use crate::identity::Identity;
use crate::types::{MessageId, PeerId, TopicId};

/// メッセージキャッシュ（重複排除）
struct MessageCache {
    seen: DashMap<MessageId, Instant>,
    max_size: usize,
}

impl MessageCache {
    fn new(max_size: usize) -> Self {
        Self {
            seen: DashMap::new(),
            max_size,
        }
    }

    /// メッセージが既知かどうかを確認し、未知なら登録する
    fn check_and_insert(&self, id: &MessageId) -> bool {
        if self.seen.contains_key(id) {
            return true; // already seen
        }
        // キャッシュサイズ超過時は古いエントリを削除
        if self.seen.len() >= self.max_size {
            self.gc();
        }
        self.seen.insert(id.clone(), Instant::now());
        false
    }

    fn gc(&self) {
        // 最も古い 25% を削除する。
        // 旧実装は Vec 全体を sort していて O(n log n)。max-heap で
        // 「最古 k 件」のみ保持することで O(n log k) に短縮する。
        let target = (self.max_size / 4).max(1);
        if self.seen.len() <= target {
            self.seen.clear();
            return;
        }
        // MessageId は Ord 未実装なので Instant だけで順序付けする wrapper。
        // (top = 最大時刻 = 最新) で max-heap を作り、走査中に top より古い
        // ものが来たら swap することで結果的に heap には「最古 target 件」が残る。
        use std::cmp::Ordering;
        use std::collections::BinaryHeap;
        struct ByTime(Instant, MessageId);
        impl PartialEq for ByTime {
            fn eq(&self, o: &Self) -> bool {
                self.0 == o.0
            }
        }
        impl Eq for ByTime {}
        impl Ord for ByTime {
            fn cmp(&self, o: &Self) -> Ordering {
                self.0.cmp(&o.0)
            }
        }
        impl PartialOrd for ByTime {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }

        let mut oldest: BinaryHeap<ByTime> = BinaryHeap::with_capacity(target + 1);
        for entry in self.seen.iter() {
            let t = *entry.value();
            if oldest.len() < target {
                oldest.push(ByTime(t, entry.key().clone()));
            } else if let Some(top) = oldest.peek() {
                if t < top.0 {
                    oldest.pop();
                    oldest.push(ByTime(t, entry.key().clone()));
                }
            }
        }
        for ByTime(_t, key) in oldest {
            self.seen.remove(&key);
        }
    }
}

/// Gossipsub ノード
#[allow(dead_code)]
pub struct GossipNode {
    /// ローカルピアID
    local_peer_id: PeerId,
    /// メッシュピア（Topic ごと）
    mesh: DashMap<TopicId, Vec<PeerId>>,
    /// ファンアウトピア（Topic ごと）
    fanout: DashMap<TopicId, Vec<PeerId>>,
    /// メッセージキャッシュ
    message_cache: MessageCache,
    /// 受信メッセージの通知チャンネル
    tx: broadcast::Sender<(TopicId, GossipMessage)>,
    /// 送信 (fan-out) メッセージの通知チャンネル。Daemon の gossip 送信タスクが
    /// これを購読し、QUIC 上で `send_gossip` を使って各メッシュピアへ配信する。
    outbound_tx: broadcast::Sender<OutboundGossip>,
    /// パラメータ
    params: GossipsubConfig,
    /// 自ノードの ed25519 identity。`with_identity` で設定された時のみ
    /// publish 時に SignedGossipMessage に包む (S3 対策)。
    identity: Option<Arc<Identity>>,
    /// 受信検証を厳密化するフラグ。`true` の場合、署名が通らない
    /// メッセージは receive path から脱落する。段階移行のため既定 `false`。
    require_signature: bool,
}

impl GossipNode {
    pub fn new(local_peer_id: PeerId, params: GossipsubConfig) -> Self {
        let (tx, _) = broadcast::channel(256);
        let (outbound_tx, _) = broadcast::channel(256);
        Self {
            local_peer_id,
            mesh: DashMap::new(),
            fanout: DashMap::new(),
            message_cache: MessageCache::new(params.message_cache_size),
            tx,
            outbound_tx,
            params,
            identity: None,
            require_signature: false,
        }
    }

    /// Daemon の送信タスクが購読する outbound チャネルを取得する。
    /// `publish()` のたびに (topic, signed, mesh peers) が流れてくる。
    pub fn outbound_receiver(&self) -> broadcast::Receiver<OutboundGossip> {
        self.outbound_tx.subscribe()
    }

    /// 署名付き送信 + 検証に使う Identity を差し込む。
    pub fn set_identity(&mut self, identity: Arc<Identity>) {
        self.identity = Some(identity);
    }

    /// 受信時に署名検証を必須にするか切り替える。
    pub fn set_require_signature(&mut self, require: bool) {
        self.require_signature = require;
    }

    /// 署名付き送信用に現在のメッセージを封筒化する。
    /// Identity が未設定の場合は `None` を返し、呼び出し側は送信をスキップする。
    pub fn envelope(&self, message: GossipMessage) -> Option<SignedGossipMessage> {
        self.identity
            .as_ref()
            .map(|id| SignedGossipMessage::sign(message, id))
    }

    /// 受信した SignedGossipMessage を検証し、内側の GossipMessage を取り出す。
    /// 検証失敗 / 未署名は `None`。
    pub fn verify_envelope(&self, signed: SignedGossipMessage) -> Option<GossipMessage> {
        if let Err(e) = signed.verify() {
            tracing::warn!("dropped gossip message: {e}");
            return None;
        }
        Some(signed.message)
    }

    /// Topic を購読（プロジェクト参加時）
    pub fn subscribe(&self, topic: TopicId) {
        self.mesh.entry(topic).or_default();
    }

    /// Topic から退出
    pub fn unsubscribe(&self, topic: &TopicId) {
        self.mesh.remove(topic);
        self.fanout.remove(topic);
    }

    /// メッセージをメッシュに配信
    pub fn publish(&self, topic: &TopicId, message: GossipMessage) -> Vec<PeerId> {
        // MessageId を正規バイト列から派生 (S21 対策: Debug 出力依存をやめる)
        let msg_bytes = canonical_bytes(&message);
        let msg_id = MessageId::from_content(&msg_bytes);

        // 重複チェック
        if self.message_cache.check_and_insert(&msg_id) {
            return vec![]; // already published
        }

        // ローカルの broadcast channel に通知 (自ノード内の購読者向け)
        let _ = self.tx.send((topic.clone(), message.clone()));

        let peers = self
            .mesh
            .get(topic)
            .map(|peers| peers.clone())
            .unwrap_or_default();

        // Identity を持っていれば署名して outbound にも流す。
        // Daemon の送信タスクがこれを拾って QUIC で各メッシュピアに配る。
        if let Some(signed) = self.envelope(message) {
            let _ = self.outbound_tx.send(OutboundGossip {
                topic: topic.clone(),
                signed,
                peers: peers.clone(),
            });
        }

        peers
    }

    /// 署名付き受信メッセージの処理。検証成功時はキャッシュチェック + 配信する。
    pub fn on_signed_message_received(
        &self,
        topic: &TopicId,
        signed: SignedGossipMessage,
        _from: &PeerId,
    ) -> bool {
        match self.verify_envelope(signed) {
            Some(msg) => self.deliver(topic, msg),
            None => false,
        }
    }

    /// 受信メッセージを処理 (無署名経路)。
    /// `require_signature=true` の場合は即 drop。
    pub fn on_message_received(
        &self,
        topic: &TopicId,
        message: GossipMessage,
        _from: &PeerId,
    ) -> bool {
        if self.require_signature {
            tracing::debug!("dropping unsigned gossip message (require_signature=true)");
            return false;
        }
        self.deliver(topic, message)
    }

    /// MessageCache 重複チェック + broadcast 配信の共通処理。
    fn deliver(&self, topic: &TopicId, message: GossipMessage) -> bool {
        let msg_bytes = canonical_bytes(&message);
        let msg_id = MessageId::from_content(&msg_bytes);
        if self.message_cache.check_and_insert(&msg_id) {
            return false; // duplicate
        }
        let _ = self.tx.send((topic.clone(), message));
        true
    }

    /// メッセージ受信を購読
    pub fn receiver(&self) -> broadcast::Receiver<(TopicId, GossipMessage)> {
        self.tx.subscribe()
    }

    /// メッシュにピアを追加 (GRAFT)
    pub fn graft(&self, topic: &TopicId, peer: PeerId) {
        self.mesh.entry(topic.clone()).or_default().push(peer);
        self.enforce_mesh_bounds(topic);
    }

    /// メッシュからピアを削除 (PRUNE)
    pub fn prune(&self, topic: &TopicId, peer: &PeerId) {
        if let Some(mut peers) = self.mesh.get_mut(topic) {
            peers.retain(|p| p != peer);
        }
    }

    /// ハートビート処理
    pub fn heartbeat(&self) {
        for entry in self.mesh.iter() {
            let topic = entry.key();
            self.enforce_mesh_bounds(topic);
        }
    }

    /// メッシュの現在のピア一覧を取得
    pub fn mesh_peers(&self, topic: &TopicId) -> Vec<PeerId> {
        self.mesh
            .get(topic)
            .map(|peers| peers.clone())
            .unwrap_or_default()
    }

    /// 購読中の Topic 一覧
    pub fn subscribed_topics(&self) -> Vec<TopicId> {
        self.mesh.iter().map(|e| e.key().clone()).collect()
    }

    // --- internal ---

    fn enforce_mesh_bounds(&self, topic: &TopicId) {
        if let Some(mut peers) = self.mesh.get_mut(topic) {
            // mesh_n_high を超えたら刈り込み
            while peers.len() > self.params.mesh_n_high {
                peers.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscribe_and_publish() {
        let node = GossipNode::new(
            PeerId::new("local"),
            GossipsubConfig {
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                heartbeat_interval_ms: 1000,
                message_cache_size: 100,
            },
        );

        let topic = TopicId::project("proj-1");
        node.subscribe(topic.clone());

        // Add peers to mesh
        node.graft(&topic, PeerId::new("peer-a"));
        node.graft(&topic, PeerId::new("peer-b"));

        let msg = GossipMessage::CatalogUpdate {
            project_id: "proj-1".into(),
            root_crc: 0x1234,
            update_count: 1,
            updated_chunks: vec![],
        };

        let targets = node.publish(&topic, msg);
        assert_eq!(targets.len(), 2);
    }

    #[test]
    fn test_duplicate_message_blocked() {
        let node = GossipNode::new(
            PeerId::new("local"),
            GossipsubConfig {
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                heartbeat_interval_ms: 1000,
                message_cache_size: 100,
            },
        );

        let topic = TopicId::project("proj-1");
        node.subscribe(topic.clone());

        let msg = GossipMessage::FileWant {
            requester: PeerId::new("peer-a"),
            file_id: crate::types::FileId::new("f1"),
            version: 1,
        };

        // First publish succeeds
        node.publish(&topic, msg.clone());
        // Second publish returns empty (duplicate)
        let targets = node.publish(&topic, msg);
        assert!(targets.is_empty());
    }
}
