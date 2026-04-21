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
pub mod identity;
pub mod mesh;
pub mod quic;
pub mod transfer;
pub mod tunnel;
pub mod types;

// Re-export primary types
pub use catalog::{CatalogManager, RootCatalog};
pub use chain::{ChainBlock, ChainPayload, FileChain, TransferLedger};
pub use conduit::{Conduit, PeerConnectionStatus};
pub use config::NetConfig;
pub use dht::DhtNode;
pub use error::{Result, SynergosNetError};
pub use gossip::{GossipMessage, GossipNode};
pub use identity::{
    peer_id_from_public_bytes, verify as verify_signature, Identity, IdentityError,
};
pub use mesh::Mesh;
pub use quic::{CalibratedParams, ConnectionCalibrator, QuicManager, SpeedTestResult};
pub use tunnel::{TunnelManager, TunnelState};
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

/// Network Foundation Layer のエントリポイント。
/// `Daemon` は個別コンポーネントを直接配線するためこの集約 struct は
/// 現状インスタンス化されない (将来の組込みホスト向け便宜)。
#[allow(dead_code)]
pub struct SynergosNet {
    pub config: NetConfig,
    pub dht: DhtNode,
    pub gossip: GossipNode,
    pub catalog: CatalogManager,
    pub ledger: TransferLedger,
    pub quic: Arc<QuicManager>,
    pub tunnel: Arc<TunnelManager>,
    pub mesh: Arc<Mesh>,
    pub conduit: Conduit,
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
        let quic = Arc::new(QuicManager::new(config.quic.clone()));
        let tunnel = Arc::new(TunnelManager::new(&config.tunnel));
        let mesh = Arc::new(Mesh::new(config.mesh.clone()));
        let conduit = Conduit::new(
            quic.clone(),
            tunnel.clone(),
            mesh.clone(),
            std::time::Duration::from_secs(15),
        );

        Self {
            config,
            dht,
            gossip,
            catalog,
            ledger,
            quic,
            tunnel,
            mesh,
            conduit,
            handler,
        }
    }

    /// グレースフルシャットダウン
    pub async fn shutdown(self) -> Result<()> {
        tracing::info!("Shutting down synergos-net...");

        // 1. 全ピアとの接続を切断
        self.conduit.shutdown().await;

        // 2. QUIC エンドポイントをシャットダウン
        self.quic.shutdown().await;

        // 3. Tunnel を停止
        let _ = self.tunnel.stop().await;

        // 4. Mesh セッションをクリーンアップ
        self.mesh.cleanup_expired_sessions();

        tracing::info!("synergos-net shutdown complete");
        Ok(())
    }
}
