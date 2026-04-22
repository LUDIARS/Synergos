use std::time::{Duration, Instant};

use crate::types::{NodeId, PeerId, Route};

/// バケット最古エントリが stale (応答確認が必要) と判断するまでの閾値。
/// Kademlia の本来の運用 (ping→タイムアウトで evict) に合わせて長めに取る。
const STALE_THRESHOLD: Duration = Duration::from_secs(15 * 60);

/// Kademlia k-bucket 内のエントリ
#[derive(Debug, Clone)]
pub struct BucketEntry {
    pub node_id: NodeId,
    pub peer_id: PeerId,
    pub endpoints: Vec<Route>,
    pub last_seen: Instant,
    pub rtt_ms: Option<u32>,
}

/// Kademlia k-bucket
pub struct KBucket {
    pub entries: Vec<BucketEntry>,
    pub last_refreshed: Instant,
    k: usize,
}

impl KBucket {
    pub fn new(k: usize) -> Self {
        Self {
            entries: Vec::with_capacity(k),
            last_refreshed: Instant::now(),
            k,
        }
    }

    /// エントリを追加または更新
    ///
    /// Kademlia の eviction policy に近づけた挙動:
    /// - 既存 node_id → 末尾へ move (LRU 更新)
    /// - bucket 空きあり → 末尾に append
    /// - bucket 満杯 + 最古エントリが stale (≥ STALE_THRESHOLD) → 最古を置換
    /// - bucket 満杯 + 最古も fresh → 新規候補を drop
    ///
    /// これにより「KBucket eviction flooding」(S22) — 古いだけで生きている
    /// ノードが短命 peer によって無差別に追い出される問題を抑える。
    /// (本来は最古ノードに ping を投げて応答確認後に判断するが、その
    /// スケジューリングは上位の RoutingTable::maintenance に委ねる。)
    pub fn upsert(&mut self, entry: BucketEntry) {
        if let Some(pos) = self.entries.iter().position(|e| e.node_id == entry.node_id) {
            // 既存エントリを末尾へ move しつつ last_seen / endpoints を更新 (LRU)
            let mut existing = self.entries.remove(pos);
            existing.endpoints = entry.endpoints;
            existing.last_seen = entry.last_seen;
            existing.rtt_ms = entry.rtt_ms;
            self.entries.push(existing);
        } else if self.entries.len() < self.k {
            self.entries.push(entry);
        } else if let Some((oldest_idx, oldest)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.last_seen)
        {
            if oldest.last_seen.elapsed() >= STALE_THRESHOLD {
                self.entries[oldest_idx] = entry;
            } else {
                tracing::debug!(
                    "kbucket full and oldest entry fresh; dropping new peer {:?}",
                    entry.peer_id
                );
                return;
            }
        }
        self.last_refreshed = Instant::now();
    }

    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.k
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Kademlia ルーティングテーブル
pub struct RoutingTable {
    /// 自身のノードID
    local_id: NodeId,
    /// 256 個の k-bucket (距離 0〜255)
    buckets: Vec<KBucket>,
}

impl RoutingTable {
    pub fn new(local_id: NodeId, k: usize) -> Self {
        let buckets = (0..256).map(|_| KBucket::new(k)).collect();
        Self { local_id, buckets }
    }

    /// ピアをルーティングテーブルに追加
    pub fn insert(&mut self, entry: BucketEntry) {
        let bucket_idx = self.bucket_index(&entry.node_id);
        self.buckets[bucket_idx].upsert(entry);
    }

    /// 指定 NodeId に最も近い k 個のノードを返す
    pub fn closest(&self, target: &NodeId, count: usize) -> Vec<&BucketEntry> {
        let mut all_entries: Vec<&BucketEntry> =
            self.buckets.iter().flat_map(|b| b.entries.iter()).collect();

        all_entries.sort_by(|a, b| {
            let dist_a = target.xor_distance(&a.node_id);
            let dist_b = target.xor_distance(&b.node_id);
            dist_a.cmp(&dist_b)
        });

        all_entries.into_iter().take(count).collect()
    }

    /// ルーティングテーブル内の全ノード数
    pub fn total_nodes(&self) -> usize {
        self.buckets.iter().map(|b| b.len()).sum()
    }

    fn bucket_index(&self, node_id: &NodeId) -> usize {
        let zeros = self.local_id.leading_zeros(node_id) as usize;
        // leading_zeros が 256 (= 完全一致) の場合は bucket 0
        zeros.min(255)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PeerId;

    fn make_node_id(byte: u8) -> NodeId {
        let mut id = [0u8; 32];
        id[0] = byte;
        NodeId(id)
    }

    #[test]
    fn test_bucket_upsert() {
        let mut bucket = KBucket::new(3);
        let entry = BucketEntry {
            node_id: make_node_id(1),
            peer_id: PeerId::new("p1"),
            endpoints: vec![],
            last_seen: Instant::now(),
            rtt_ms: Some(10),
        };
        bucket.upsert(entry);
        assert_eq!(bucket.len(), 1);
    }

    #[test]
    fn test_routing_closest() {
        let local = make_node_id(0);
        let mut table = RoutingTable::new(local, 20);

        for i in 1..=10u8 {
            table.insert(BucketEntry {
                node_id: make_node_id(i),
                peer_id: PeerId::new(format!("p{}", i)),
                endpoints: vec![],
                last_seen: Instant::now(),
                rtt_ms: None,
            });
        }

        let target = make_node_id(3);
        let closest = table.closest(&target, 3);
        assert_eq!(closest.len(), 3);
        // first result should be exact match
        assert_eq!(closest[0].node_id, make_node_id(3));
    }
}
