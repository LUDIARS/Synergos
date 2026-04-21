use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::routing::{BucketEntry, RoutingTable};
use crate::config::DhtConfig;
use crate::types::{NodeId, PeerId, Route};

/// DHT に保存するピアレコード
#[derive(Debug, Clone)]
pub struct PeerRecord {
    pub peer_id: PeerId,
    pub display_name: String,
    pub endpoints: Vec<Route>,
    /// 参加中のプロジェクト一覧
    pub active_projects: Vec<String>,
    /// レコード発行時刻
    pub published_at: Instant,
    /// TTL
    pub ttl: Duration,
}

impl PeerRecord {
    pub fn is_expired(&self) -> bool {
        self.published_at.elapsed() > self.ttl
    }
}

/// DHT ノード情報ストアの上限。S8 対策: 悪意ある peer が announce を
/// 大量に送ってきても 10k 件で頭打ちになる。超過時は最も古いものから
/// evict する。
const DHT_STORE_MAX: usize = 10_000;

/// DHT ノード（Kademlia ベース）
pub struct DhtNode {
    /// 自身のピアID
    pub local_peer_id: PeerId,
    /// 自身のノードID
    pub node_id: NodeId,
    /// Kademlia ルーティングテーブル
    routing_table: RwLock<RoutingTable>,
    /// ノード情報ストア
    store: DashMap<NodeId, PeerRecord>,
    /// 設定
    config: DhtConfig,
}

impl DhtNode {
    pub fn new(local_peer_id: PeerId, config: DhtConfig) -> Self {
        let node_id = NodeId::from_peer_id(&local_peer_id);
        let routing_table = RoutingTable::new(node_id.clone(), config.k_bucket_size);
        Self {
            local_peer_id,
            node_id,
            routing_table: RwLock::new(routing_table),
            store: DashMap::new(),
            config,
        }
    }

    /// ピアを検索（Kademlia FIND_NODE）
    pub async fn find_peer(&self, peer_id: &PeerId) -> Option<PeerRecord> {
        let target_node_id = NodeId::from_peer_id(peer_id);

        // まずローカルストアを確認
        if let Some(record) = self.store.get(&target_node_id) {
            if !record.is_expired() {
                return Some(record.clone());
            }
        }

        // ルーティングテーブルから最も近いノードを取得
        // （実際のネットワーク問い合わせは上位レイヤが行う）
        None
    }

    /// 特定プロジェクトに参加しているピアを検索
    pub async fn find_project_peers(&self, project_id: &str) -> Vec<PeerRecord> {
        self.store
            .iter()
            .filter(|entry| {
                let record = entry.value();
                !record.is_expired() && record.active_projects.contains(&project_id.to_string())
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// 自身のレコードを DHT に公開
    pub async fn announce(&self, record: PeerRecord) {
        let node_id = NodeId::from_peer_id(&record.peer_id);
        // 上限を超えた場合は一度 GC、それでも収まらなければ最古を捨てる (S8)
        if self.store.len() >= DHT_STORE_MAX && !self.store.contains_key(&node_id) {
            self.gc_expired();
            if self.store.len() >= DHT_STORE_MAX {
                if let Some(oldest_key) = self
                    .store
                    .iter()
                    .min_by_key(|e| e.value().published_at)
                    .map(|e| e.key().clone())
                {
                    self.store.remove(&oldest_key);
                }
            }
        }
        self.store.insert(node_id, record);
    }

    /// 外部ピアの情報をルーティングテーブルに追加
    pub async fn add_peer(&self, peer_id: PeerId, endpoints: Vec<Route>, rtt_ms: Option<u32>) {
        let node_id = NodeId::from_peer_id(&peer_id);
        let entry = BucketEntry {
            node_id,
            peer_id,
            endpoints,
            last_seen: Instant::now(),
            rtt_ms,
        };
        self.routing_table.write().await.insert(entry);
    }

    /// 指定 NodeId に最も近い k 個のノードを返す
    pub async fn closest_peers(&self, target: &NodeId, count: usize) -> Vec<PeerId> {
        self.routing_table
            .read()
            .await
            .closest(target, count)
            .into_iter()
            .map(|e| e.peer_id.clone())
            .collect()
    }

    /// ルーティングテーブルの総ノード数
    pub async fn total_known_peers(&self) -> usize {
        self.routing_table.read().await.total_nodes()
    }

    /// 期限切れレコードを削除
    pub fn gc_expired(&self) {
        self.store.retain(|_, record| !record.is_expired());
    }
}
