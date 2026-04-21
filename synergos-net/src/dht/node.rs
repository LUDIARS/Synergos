use std::collections::HashSet;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::routing::{BucketEntry, RoutingTable};
use super::rpc::{DhtRequest, DhtResponse, DhtTransport, PeerRecordDto};
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

    /// DTO への変換 (送信用)
    pub fn to_dto(&self) -> PeerRecordDto {
        PeerRecordDto {
            peer_id: self.peer_id.clone(),
            display_name: self.display_name.clone(),
            endpoints: self.endpoints.iter().map(Into::into).collect(),
            active_projects: self.active_projects.clone(),
            ttl_secs: self.ttl.as_secs(),
        }
    }

    /// DTO → PeerRecord の復元。published_at は受信側の now で近似する。
    pub fn from_dto(dto: PeerRecordDto) -> Self {
        Self {
            peer_id: dto.peer_id,
            display_name: dto.display_name,
            endpoints: dto.endpoints.iter().map(Into::into).collect(),
            active_projects: dto.active_projects,
            published_at: Instant::now(),
            ttl: Duration::from_secs(dto.ttl_secs),
        }
    }
}

/// `find_peer_iterative` の動作パラメータ。
#[derive(Debug, Clone)]
pub struct FindNodeOptions {
    /// 1 ラウンド (= 1 peer への問合せ) あたりのタイムアウト
    pub per_request_timeout: Duration,
    /// 最大ホップ数 (再帰深さ)
    pub max_hops: u8,
    /// 1 ラウンドあたりに並行問合せる α (Kademlia でいう α = 3 が標準)
    pub alpha: usize,
    /// 最終的に束ねる closest k 件
    pub k: usize,
}

impl Default for FindNodeOptions {
    fn default() -> Self {
        Self {
            per_request_timeout: Duration::from_secs(3),
            max_hops: 4,
            alpha: 3,
            k: 20,
        }
    }
}

/// DHT ノード情報ストアの上限。S8 対策: 悪意ある peer が announce を
/// 大量に送ってきても 10k 件で頭打ちになる。超過時は最も古いものから
/// evict する。
const DHT_STORE_MAX: usize = 10_000;

/// DHT ノード（Kademlia ベース）
#[allow(dead_code)]
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

    /// ローカルストアのみから peer_id を探す。ネットワーク lookup は含まない。
    pub async fn find_peer_local(&self, peer_id: &PeerId) -> Option<PeerRecord> {
        let target_node_id = NodeId::from_peer_id(peer_id);
        if let Some(record) = self.store.get(&target_node_id) {
            if !record.is_expired() {
                return Some(record.clone());
            }
        }
        None
    }

    /// ピアを検索 (ローカルストア優先)。トランスポートが無い環境では
    /// この `find_peer` (= `find_peer_local`) を使う。
    pub async fn find_peer(&self, peer_id: &PeerId) -> Option<PeerRecord> {
        self.find_peer_local(peer_id).await
    }

    /// Kademlia iterative FIND_NODE。ネットワーク経由で target PeerId を探す。
    ///
    /// アルゴリズム:
    /// 1. ローカル store に hit すれば即座に返す
    /// 2. ルーティングテーブルから target に近い α 件を候補として取り、問合せキューへ
    /// 3. 並行に `FindNode` を投げ、返ってきた候補をキューへ足す
    /// 4. hop 数が `max_hops` に達するか、新たに追加できる近接ノードが尽きたら終了
    ///
    /// `alpha`, `max_hops`, `per_request_timeout` で暴走を防止する。
    pub async fn find_peer_iterative(
        &self,
        target: &PeerId,
        transport: &dyn DhtTransport,
        opts: FindNodeOptions,
    ) -> Option<PeerRecord> {
        if let Some(local) = self.find_peer_local(target).await {
            return Some(local);
        }

        let target_node_id = NodeId::from_peer_id(target);

        // ルーティングテーブルから初期 α 件
        let mut queried: HashSet<PeerId> = HashSet::new();
        queried.insert(self.local_peer_id.clone());

        let mut candidates: Vec<PeerId> = self
            .closest_peers(&target_node_id, opts.alpha.max(1))
            .await
            .into_iter()
            .filter(|p| !queried.contains(p))
            .collect();

        for hop in 0..opts.max_hops {
            if candidates.is_empty() {
                tracing::debug!(
                    "find_peer_iterative: no more candidates at hop {hop} for target {}",
                    target.short()
                );
                break;
            }

            // 今回 hop でまわす α 件をピック。queried に追加しておくことで重複を防ぐ。
            let round: Vec<PeerId> = candidates.drain(..).take(opts.alpha).collect();
            for p in &round {
                queried.insert(p.clone());
            }

            // α 件を並列発射
            let mut futures = Vec::with_capacity(round.len());
            for peer in &round {
                let peer = peer.clone();
                let req = DhtRequest::FindNode {
                    target: target.clone(),
                };
                let timeout = opts.per_request_timeout;
                futures.push(async move {
                    let fut = transport.request(&peer, &req);
                    match tokio::time::timeout(timeout, fut).await {
                        Ok(Ok(resp)) => Some((peer, resp)),
                        Ok(Err(e)) => {
                            tracing::debug!(
                                "find_peer_iterative: transport error from {}: {e}",
                                peer.short()
                            );
                            None
                        }
                        Err(_) => {
                            tracing::debug!(
                                "find_peer_iterative: timeout from {}",
                                peer.short()
                            );
                            None
                        }
                    }
                });
            }
            let round_results = futures::future::join_all(futures).await;

            for (_responder, response) in round_results.into_iter().flatten() {
                match response {
                    DhtResponse::Nodes { exact, closest } => {
                        if let Some(dto) = exact {
                            let rec = PeerRecord::from_dto(dto);
                            // 受信側でも local store に kick back しておく
                            self.announce(rec.clone()).await;
                            return Some(rec);
                        }
                        for dto in closest {
                            let rec = PeerRecord::from_dto(dto);
                            let pid = rec.peer_id.clone();
                            // routing table にも反映
                            self.add_peer(pid.clone(), rec.endpoints.clone(), None).await;
                            // 未問合せなら候補に入れる
                            if !queried.contains(&pid) && !candidates.contains(&pid) {
                                candidates.push(pid);
                            }
                        }
                    }
                    DhtResponse::Error { message } => {
                        tracing::debug!("find_peer_iterative: peer error: {message}");
                    }
                    _ => {
                        tracing::debug!("find_peer_iterative: unexpected response variant");
                    }
                }
            }

            // 候補を target に近い順へソートして次 hop へ
            candidates.sort_by(|a, b| {
                let da = NodeId::from_peer_id(a).xor_distance(&target_node_id);
                let db = NodeId::from_peer_id(b).xor_distance(&target_node_id);
                da.cmp(&db)
            });
            candidates.truncate(opts.k);
        }

        // 全 hop 消化した後にもう一度ローカル (受信側で announce された分) を確認
        self.find_peer_local(target).await
    }

    /// 受信したリクエストをディスパッチする (サーバ側ハンドラ)。
    /// 受信する型を既に判別済みの前提 — トランスポート層から呼ばれる。
    pub async fn handle_request(&self, req: DhtRequest) -> DhtResponse {
        match req {
            DhtRequest::Ping => DhtResponse::Pong,
            DhtRequest::FindNode { target } => {
                let target_node_id = NodeId::from_peer_id(&target);
                let exact = self.find_peer_local(&target).await.map(|r| r.to_dto());
                let closest_ids = self.closest_peers(&target_node_id, 20).await;
                let closest = closest_ids
                    .into_iter()
                    .filter_map(|pid| {
                        let node_id = NodeId::from_peer_id(&pid);
                        self.store.get(&node_id).map(|e| e.value().to_dto())
                    })
                    .collect();
                DhtResponse::Nodes { exact, closest }
            }
            DhtRequest::FindProjectPeers { project_id } => {
                let peers = self
                    .find_project_peers(&project_id)
                    .await
                    .iter()
                    .map(|r| r.to_dto())
                    .collect();
                DhtResponse::ProjectPeers { peers }
            }
            DhtRequest::Announce { record } => {
                let peer_id = record.peer_id.clone();
                let endpoints = record.endpoints.iter().map(Into::into).collect::<Vec<_>>();
                self.announce(PeerRecord::from_dto(record)).await;
                self.add_peer(peer_id, endpoints, None).await;
                DhtResponse::AnnounceAck
            }
        }
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
