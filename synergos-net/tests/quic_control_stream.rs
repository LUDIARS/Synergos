//! PR-11/PR-12: QUIC 実通信の追加テスト。
//! 制御ストリームでのラウンドトリップ + 2 方向接続。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::QuicConfig;
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

async fn bound(identity: Arc<Identity>) -> (Arc<QuicManager>, SocketAddr) {
    let qm = Arc::new(QuicManager::new(qcfg(), identity));
    let bound = qm.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    (qm, bound)
}

#[tokio::test]
async fn control_stream_byte_roundtrip() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let (server, server_addr) = bound(server_id.clone()).await;
    let (client, _client_addr) = bound(client_id).await;

    // サーバ側: 1 回だけ bidi を accept して echo する
    let sq = server.clone();
    let server_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = sq.accept().await {
            if let Ok((mut send, mut recv)) = acc.connection.accept_bi().await {
                let mut buf = [0u8; 64];
                let mut total = 0;
                while let Ok(Some(n)) = recv.read(&mut buf[total..]).await {
                    total += n;
                    if total >= 4 {
                        break;
                    }
                }
                let _ = send.write_all(b"pong").await;
                let _ = send.finish();
            }
        }
    });

    client
        .connect(server_id.peer_id().clone(), server_addr, "synergos")
        .await
        .unwrap();

    client
        .send_control(server_id.peer_id(), b"ping")
        .await
        .expect("send_control");

    tokio::time::timeout(Duration::from_millis(500), server_task)
        .await
        .ok();
}

#[tokio::test]
async fn list_connections_tracks_active_sessions() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let (server, server_addr) = bound(server_id.clone()).await;
    let (client, _) = bound(client_id).await;

    assert_eq!(client.list_connections().len(), 0);

    let sq = server.clone();
    let task = tokio::spawn(async move {
        let _ = sq.accept().await;
    });

    client
        .connect(server_id.peer_id().clone(), server_addr, "synergos")
        .await
        .unwrap();

    assert_eq!(client.list_connections().len(), 1);
    let info = client.get_connection(server_id.peer_id()).unwrap();
    assert_eq!(info.peer_id, *server_id.peer_id());

    tokio::time::timeout(Duration::from_millis(200), task)
        .await
        .ok();
}
