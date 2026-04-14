//! Presence — ピア状態管理
//!
//! 接続可能なピアの発見と状態管理を行う。
//! DHT + Gossipsub + mDNS をバックエンドとして利用。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use synergos_net::dht::{DhtNode, PeerRecord};
use synergos_net::gossip::{ActivityState, GossipMessage, GossipNode, PeerActivityStatus};
use synergos_net::types::{PeerId, Route, TopicId};

use crate::event_bus::{PeerConnectedEvent, PeerDisconnectedEvent, SharedEventBus};

// ── 型定義 ──

/// ピアの接続状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    /// DHT/mDNS で発見済み（未接続）
    Discovered,
    /// 接続処理中
    Connecting,
    /// 接続済み
    Connected,
    /// アイドル状態（ハートビートは受信中）
    Idle,
    /// 離席中
    Away,
    /// 切断済み
    Disconnected,
}

/// 登録ノード情報
#[derive(Debug, Clone)]
pub struct RegisteredNode {
    pub peer_id: PeerId,
    pub display_name: String,
    pub endpoints: Vec<Route>,
    pub state: PeerState,
    pub rtt_ms: Option<u32>,
    pub project_ids: Vec<String>,
    pub bandwidth_bps: u64,
    pub last_seen: Instant,
}

/// ノード登録リクエスト
#[derive(Debug, Clone)]
pub struct NodeRegistration {
    pub peer_id: PeerId,
    pub display_name: String,
    pub endpoints: Vec<Route>,
    pub project_ids: Vec<String>,
}

/// ノード登録/削除エラー
#[derive(Debug, thiserror::Error)]
pub enum NodeRegistryError {
    #[error("node not found: {0}")]
    NotFound(String),
    #[error("node already registered: {0}")]
    AlreadyRegistered(String),
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("network error: {0}")]
    NetworkError(String),
}

// ── trait 定義 ──

/// ノード登録/削除インターフェース
#[async_trait]
pub trait NodeRegistry: Send + Sync {
    /// 自ノードを DHT に公開する（ネットワーク参加）
    async fn register_self(&self, registration: NodeRegistration) -> Result<(), NodeRegistryError>;

    /// 自ノードを DHT から削除する（ネットワーク離脱）
    async fn unregister_self(&self) -> Result<(), NodeRegistryError>;

    /// リモートノードを登録（接続を確立）
    async fn register_node(
        &self,
        registration: NodeRegistration,
    ) -> Result<RegisteredNode, NodeRegistryError>;

    /// リモートノードを削除（接続を切断し、ルーティングテーブルから除去）
    async fn unregister_node(&self, peer_id: &PeerId) -> Result<(), NodeRegistryError>;

    /// 登録済みノード一覧を取得
    async fn list_nodes(&self, project_id: Option<&str>) -> Vec<RegisteredNode>;

    /// 指定ノードの情報を取得
    async fn get_node(&self, peer_id: &PeerId) -> Option<RegisteredNode>;

    /// 指定ノードの状態を更新
    async fn update_node_state(
        &self,
        peer_id: &PeerId,
        state: PeerState,
    ) -> Result<(), NodeRegistryError>;
}

// ── 実装 ──

/// ピア状態管理サービス
pub struct PresenceService {
    event_bus: SharedEventBus,
    /// ローカルで管理するノード一覧
    nodes: DashMap<PeerId, RegisteredNode>,
    /// 自ノードの情報
    local_node: tokio::sync::RwLock<Option<NodeRegistration>>,
    /// DHT ノード（オプション: ネットワーク初期化後にセット）
    dht: Option<Arc<DhtNode>>,
    /// Gossipsub ノード（オプション）
    gossip: Option<Arc<GossipNode>>,
}

impl PresenceService {
    /// 最小構成のコンストラクタ（テスト・後方互換用）
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self::with_network(event_bus, None, None)
    }

    /// ネットワーク依存を注入して構築する本番向けコンストラクタ
    pub fn with_network(
        event_bus: SharedEventBus,
        dht: Option<Arc<DhtNode>>,
        gossip: Option<Arc<GossipNode>>,
    ) -> Self {
        Self {
            event_bus,
            nodes: DashMap::new(),
            local_node: tokio::sync::RwLock::new(None),
            dht,
            gossip,
        }
    }

    /// Gossipsub 経由でピアステータスをブロードキャスト
    fn broadcast_status(&self, registration: &NodeRegistration, state: ActivityState) {
        if let Some(gossip) = &self.gossip {
            for project_id in &registration.project_ids {
                let topic = TopicId::project(project_id);
                gossip.publish(
                    &topic,
                    GossipMessage::PeerStatus {
                        peer_id: registration.peer_id.clone(),
                        status: PeerActivityStatus {
                            peer_id: registration.peer_id.clone(),
                            display_name: registration.display_name.clone(),
                            state,
                            last_active: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                            working_on: vec![],
                        },
                        origin: registration.peer_id.clone(),
                        hops: 0,
                    },
                );
            }
        }
    }

    /// DHT にピアレコードを announce
    async fn dht_announce(&self, registration: &NodeRegistration) {
        if let Some(dht) = &self.dht {
            let record = PeerRecord {
                peer_id: registration.peer_id.clone(),
                display_name: registration.display_name.clone(),
                endpoints: registration.endpoints.clone(),
                active_projects: registration.project_ids.clone(),
                published_at: Instant::now(),
                ttl: Duration::from_secs(120),
            };
            dht.announce(record).await;
            tracing::debug!("DHT announce completed for {}", registration.peer_id);
        }
    }

    /// DHT からプロジェクトのピアを検索
    pub async fn discover_project_peers(&self, project_id: &str) -> Vec<RegisteredNode> {
        if let Some(dht) = &self.dht {
            let records = dht.find_project_peers(project_id).await;
            records
                .into_iter()
                .map(|r| RegisteredNode {
                    peer_id: r.peer_id,
                    display_name: r.display_name,
                    endpoints: r.endpoints,
                    state: PeerState::Discovered,
                    rtt_ms: None,
                    project_ids: r.active_projects,
                    bandwidth_bps: 0,
                    last_seen: r.published_at,
                })
                .collect()
        } else {
            vec![]
        }
    }

    /// ピアの帯域情報を更新
    pub fn update_bandwidth(&self, peer_id: &PeerId, bandwidth_bps: u64) {
        if let Some(mut entry) = self.nodes.get_mut(peer_id) {
            entry.value_mut().bandwidth_bps = bandwidth_bps;
        }
    }

    /// ピアの RTT を更新
    pub fn update_rtt(&self, peer_id: &PeerId, rtt_ms: u32) {
        if let Some(mut entry) = self.nodes.get_mut(peer_id) {
            entry.value_mut().rtt_ms = Some(rtt_ms);
            entry.value_mut().last_seen = Instant::now();
        }
    }
}

#[async_trait]
impl NodeRegistry for PresenceService {
    async fn register_self(&self, registration: NodeRegistration) -> Result<(), NodeRegistryError> {
        tracing::info!(
            "Registering self as '{}' (peer_id={})",
            registration.display_name,
            registration.peer_id
        );

        // ローカルノード情報を保存
        let mut local = self.local_node.write().await;
        *local = Some(registration.clone());

        // 自分自身も nodes テーブルに登録
        let node = RegisteredNode {
            peer_id: registration.peer_id.clone(),
            display_name: registration.display_name.clone(),
            endpoints: registration.endpoints.clone(),
            state: PeerState::Connected,
            rtt_ms: Some(0),
            project_ids: registration.project_ids.clone(),
            bandwidth_bps: 0,
            last_seen: Instant::now(),
        };
        self.nodes.insert(registration.peer_id.clone(), node);

        // DHT に announce
        self.dht_announce(&registration).await;

        // Gossipsub でステータスをブロードキャスト
        self.broadcast_status(&registration, ActivityState::Active);

        Ok(())
    }

    async fn unregister_self(&self) -> Result<(), NodeRegistryError> {
        let mut local = self.local_node.write().await;
        if let Some(reg) = local.take() {
            tracing::info!("Unregistering self (peer_id={})", reg.peer_id);
            self.nodes.remove(&reg.peer_id);

            // Gossipsub でオフラインステータスをブロードキャスト
            self.broadcast_status(&reg, ActivityState::Offline);
        }
        Ok(())
    }

    async fn register_node(
        &self,
        registration: NodeRegistration,
    ) -> Result<RegisteredNode, NodeRegistryError> {
        let peer_id = &registration.peer_id;

        // 重複チェック
        if self.nodes.contains_key(peer_id) {
            return Err(NodeRegistryError::AlreadyRegistered(peer_id.to_string()));
        }

        tracing::info!(
            "Registering remote node: '{}' (peer_id={})",
            registration.display_name,
            peer_id
        );

        let node = RegisteredNode {
            peer_id: registration.peer_id.clone(),
            display_name: registration.display_name.clone(),
            endpoints: registration.endpoints.clone(),
            state: PeerState::Discovered,
            rtt_ms: None,
            project_ids: registration.project_ids.clone(),
            bandwidth_bps: 0,
            last_seen: Instant::now(),
        };

        self.nodes.insert(peer_id.clone(), node.clone());

        // DHT にピア情報を追加
        if let Some(dht) = &self.dht {
            dht.add_peer(
                registration.peer_id.clone(),
                registration.endpoints.clone(),
                None,
            )
            .await;
        }

        // EventBus にピア接続イベントを発行
        self.event_bus.emit(PeerConnectedEvent {
            project_id: registration
                .project_ids
                .first()
                .cloned()
                .unwrap_or_default(),
            peer_id: peer_id.to_string(),
            display_name: registration.display_name.clone(),
            route: format!("{:?}", registration.endpoints.first()),
            rtt_ms: 0,
        });

        Ok(node)
    }

    async fn unregister_node(&self, peer_id: &PeerId) -> Result<(), NodeRegistryError> {
        let removed = self.nodes.remove(peer_id);

        match removed {
            Some((_, node)) => {
                tracing::info!(
                    "Unregistered node: '{}' (peer_id={})",
                    node.display_name,
                    peer_id
                );

                // EventBus にピア切断イベントを発行
                self.event_bus.emit(PeerDisconnectedEvent {
                    project_id: node.project_ids.first().cloned().unwrap_or_default(),
                    peer_id: peer_id.to_string(),
                    reason: "unregistered".to_string(),
                });

                Ok(())
            }
            None => Err(NodeRegistryError::NotFound(peer_id.to_string())),
        }
    }

    async fn list_nodes(&self, project_id: Option<&str>) -> Vec<RegisteredNode> {
        self.nodes
            .iter()
            .filter(|entry| match project_id {
                Some(pid) => entry.value().project_ids.contains(&pid.to_string()),
                None => true,
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    async fn get_node(&self, peer_id: &PeerId) -> Option<RegisteredNode> {
        self.nodes.get(peer_id).map(|entry| entry.value().clone())
    }

    async fn update_node_state(
        &self,
        peer_id: &PeerId,
        state: PeerState,
    ) -> Result<(), NodeRegistryError> {
        match self.nodes.get_mut(peer_id) {
            Some(mut entry) => {
                entry.value_mut().state = state;
                entry.value_mut().last_seen = Instant::now();
                Ok(())
            }
            None => Err(NodeRegistryError::NotFound(peer_id.to_string())),
        }
    }
}
