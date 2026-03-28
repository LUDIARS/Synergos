//! Presence — ピア状態管理
//!
//! 接続可能なピアの発見と状態管理を行う。
//! DHT + Gossipsub + mDNS をバックエンドとして利用。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use async_trait::async_trait;
use dashmap::DashMap;
use synergos_net::types::{PeerId, Route};

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
///
/// ピア（ノード）のライフサイクルを管理する。
/// - 自ノードの DHT 公開（announce）
/// - リモートノードの登録・切断・削除
/// - プロジェクト単位でのピア検索
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
}

impl PresenceService {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            event_bus,
            nodes: DashMap::new(),
            local_node: tokio::sync::RwLock::new(None),
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
        };
        self.nodes.insert(registration.peer_id.clone(), node);

        // TODO: DHT announce を synergos-net 経由で実行
        Ok(())
    }

    async fn unregister_self(&self) -> Result<(), NodeRegistryError> {
        let mut local = self.local_node.write().await;
        if let Some(reg) = local.take() {
            tracing::info!("Unregistering self (peer_id={})", reg.peer_id);
            self.nodes.remove(&reg.peer_id);
            // TODO: DHT からレコードを削除
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
        };

        self.nodes.insert(peer_id.clone(), node.clone());

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
                Ok(())
            }
            None => Err(NodeRegistryError::NotFound(peer_id.to_string())),
        }
    }
}
