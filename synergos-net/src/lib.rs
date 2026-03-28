#![allow(dead_code)]
//! synergos-net: P2P network foundation
//!
//! Ars に依存しない汎用ネットワークライブラリ。
//! QUIC, Cloudflare Tunnel, IPv6 Direct/TURN, DHT/Gossipsub メッシュを提供する。

pub mod catalog;
pub mod chain;
pub mod conduit;
pub mod config;
pub mod dht;
pub mod error;
pub mod gossip;
pub mod mesh;
pub mod quic;
pub mod tunnel;
pub mod types;

// Re-export primary types
pub use catalog::{CatalogManager, RootCatalog};
pub use chain::{ChainBlock, ChainPayload, FileChain, TransferLedger};
pub use config::NetConfig;
pub use dht::DhtNode;
pub use error::{Result, SynergosNetError};
pub use gossip::{GossipMessage, GossipNode};
pub use types::*;

use std::sync::Arc;

/// ネットワークイベントハンドラ（上位レイヤが実装する）
#[async_trait::async_trait]
pub trait NetEventHandler: Send + Sync + 'static {
    /// ピアとの接続が確立した
    fn on_peer_connected(&self, peer_id: &PeerId, route: &RouteKind, rtt_ms: u32);
    /// ピアとの接続が切断した
    fn on_peer_disconnected(&self, peer_id: &PeerId, reason: &str);
    /// データ受信
    fn on_data_received(&self, peer_id: &PeerId, stream_id: u64, data: &[u8]);
    /// 接続経路が変更された
    fn on_route_changed(&self, peer_id: &PeerId, old: &RouteKind, new: &RouteKind);
}

/// Network Foundation Layer のエントリポイント
pub struct SynergosNet {
    pub config: NetConfig,
    pub dht: DhtNode,
    pub gossip: GossipNode,
    pub catalog: CatalogManager,
    pub ledger: TransferLedger,
    handler: Arc<dyn NetEventHandler>,
}

impl SynergosNet {
    /// Network Foundation を起動する
    pub fn new(
        config: NetConfig,
        local_peer_id: PeerId,
        project_id: String,
        handler: Arc<dyn NetEventHandler>,
    ) -> Self {
        let dht = DhtNode::new(local_peer_id.clone(), config.dht.clone());
        let gossip = GossipNode::new(local_peer_id, config.gossipsub.clone());
        let catalog = CatalogManager::new(project_id, 256, 10);
        let ledger = TransferLedger::new();

        Self {
            config,
            dht,
            gossip,
            catalog,
            ledger,
            handler,
        }
    }

    /// グレースフルシャットダウン
    pub async fn shutdown(self) -> Result<()> {
        // TODO: 接続のクリーンアップ
        Ok(())
    }
}
