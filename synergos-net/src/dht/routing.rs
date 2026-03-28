use std::time::Instant;

use crate::types::{NodeId, PeerId, Route};

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
    pub fn upsert(&mut self, entry: BucketEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.node_id == entry.node_id) {
            // 既存エントリを更新して末尾に移動
            existing.endpoints = entry.endpoints;
            existing.last_seen = entry.last_seen;
            existing.rtt_ms = entry.rtt_ms;
        } else if self.entries.len() < self.k {
            self.entries.push(entry);
        } else {
            // bucket がフル → 最も古いエントリが応答しない場合のみ置換
            // （Kademlia の eviction policy）
            // ここでは簡易的に最古を置換
            if let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_seen)
                .map(|(i, _)| i)
            {
                self.entries[oldest] = entry;
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
    /// k-bucket サイズ
    k: usize,
}

impl RoutingTable {
    pub fn new(local_id: NodeId, k: usize) -> Self {
        let buckets = (0..256).map(|_| KBucket::new(k)).collect();
        Self {
            local_id,
            buckets,
            k,
        }
    }

    /// ピアをルーティングテーブルに追加
    pub fn insert(&mut self, entry: BucketEntry) {
        let bucket_idx = self.bucket_index(&entry.node_id);
        self.buckets[bucket_idx].upsert(entry);
    }

    /// 指定 NodeId に最も近い k 個のノードを返す
    pub fn closest(&self, target: &NodeId, count: usize) -> Vec<&BucketEntry> {
        let mut all_entries: Vec<&BucketEntry> = self
            .buckets
            .iter()
            .flat_map(|b| b.entries.iter())
            .collect();

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
