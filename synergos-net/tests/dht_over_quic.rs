//! PR-12: 2 ノードで QuicDhtTransport 経由の FIND_NODE 往復を確認する。
//! サーバ側は accept → dispatch_peer_streams 相当を手動で組む。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::{DhtConfig, QuicConfig};
use synergos_net::dht::{
    handle_dht_stream, DhtNode, DhtTransport, FindNodeOptions, PeerRecord, QuicDhtTransport,
};
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use synergos_net::types::{PeerId, Route};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

fn dcfg() -> DhtConfig {
    DhtConfig {
        k_bucket_size: 8,
        routing_refresh_secs: 300,
        peer_ttl_secs: 300,
    }
}

async fn bound(identity: Arc<Identity>) -> (Arc<QuicManager>, SocketAddr) {
    let qm = Arc::new(QuicManager::new(qcfg(), identity));
    let bound = qm.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    (qm, bound)
}

#[tokio::test]
async fn find_node_over_quic_resolves_target_via_server_announce() {
    let server_id_entity = Arc::new(Identity::generate());
    let client_id_entity = Arc::new(Identity::generate());

    // target_peer はサーバ側のローカル store に入れる。
    let target = PeerId::new("target");

    // ── サーバ側 ──
    let (server_quic, server_addr) = bound(server_id_entity.clone()).await;
    let server_dht = Arc::new(DhtNode::new(server_id_entity.peer_id().clone(), dcfg()));
    server_dht
        .announce(PeerRecord {
            peer_id: target.clone(),
            display_name: "t".into(),
            endpoints: vec![Route::Tunnel {
                tunnel_id: "t1".into(),
                hostname: "example.com".into(),
            }],
            active_projects: vec!["p".into()],
            published_at: std::time::Instant::now(),
            ttl: Duration::from_secs(300),
        })
        .await;

    // サーバ accept ループ (1 接続ぶんだけ処理する)
    let sq = server_quic.clone();
    let sdht = server_dht.clone();
    let server_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = sq.accept().await {
            let connection = acc.connection;
            while let Ok((send, mut recv)) = connection.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_ok() && &magic == b"DHT1" {
                    let _ = handle_dht_stream(sdht.clone(), send, recv).await;
                }
            }
        }
    });

    // ── クライアント側 ──
    let (client_quic, _) = bound(client_id_entity.clone()).await;
    client_quic
        .connect(server_id_entity.peer_id().clone(), server_addr, "synergos")
        .await
        .unwrap();

    let client_dht = DhtNode::new(client_id_entity.peer_id().clone(), dcfg());
    // ルーティングテーブルに server を入れる
    client_dht
        .add_peer(server_id_entity.peer_id().clone(), vec![], None)
        .await;

    let transport: Arc<dyn DhtTransport> = Arc::new(QuicDhtTransport::new(client_quic.clone()));
    let opts = FindNodeOptions {
        per_request_timeout: Duration::from_secs(3),
        max_hops: 2,
        alpha: 2,
        k: 20,
    };
    let found = client_dht
        .find_peer_iterative(&target, transport.as_ref(), opts)
        .await;
    assert!(found.is_some(), "target must be resolvable via server");
    assert_eq!(found.unwrap().peer_id, target);

    client_quic
        .disconnect(server_id_entity.peer_id(), "done")
        .await;
    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}
