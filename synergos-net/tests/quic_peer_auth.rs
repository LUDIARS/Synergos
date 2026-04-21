//! S1 QUIC ピア真性認証の E2E テスト
//!
//! ループバックでサーバ / クライアントを立て、connect の成否が
//! `expected_peer_id` と実ピアの鍵一致に厳密に連動することを確認する。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::QuicConfig;
use synergos_net::error::SynergosNetError;
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
    }
}

async fn bind_server(identity: Arc<Identity>) -> (Arc<QuicManager>, SocketAddr) {
    let server = Arc::new(QuicManager::new(qcfg(), identity));
    let addr: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let bound = server.bind(addr).await.expect("server bind");
    (server, bound)
}

async fn build_client(identity: Arc<Identity>) -> Arc<QuicManager> {
    let client = Arc::new(QuicManager::new(qcfg(), identity));
    let addr: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    client.bind(addr).await.expect("client bind");
    client
}

#[tokio::test]
async fn connect_succeeds_with_matching_peer_id() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let (server, server_addr) = bind_server(server_id.clone()).await;
    let client = build_client(client_id).await;

    // サーバが着信を accept するタスクを回す
    let server_task = {
        let server = server.clone();
        tokio::spawn(async move {
            let _ = server.accept().await;
        })
    };

    client
        .connect(server_id.peer_id().clone(), server_addr, "synergos")
        .await
        .expect("connect should succeed with matching peer id");

    assert!(client.get_connection(server_id.peer_id()).is_some());

    // accept タスクが進むのを少し待つ (assert 済みなので強制終了で十分)
    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}

#[tokio::test]
async fn disconnect_removes_connection_entry() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let (server, server_addr) = bind_server(server_id.clone()).await;
    let client = build_client(client_id).await;

    let server_task = {
        let server = server.clone();
        tokio::spawn(async move {
            let _ = server.accept().await;
        })
    };

    client
        .connect(server_id.peer_id().clone(), server_addr, "synergos")
        .await
        .unwrap();

    assert!(client.get_connection(server_id.peer_id()).is_some());
    client.disconnect(server_id.peer_id(), "test").await;
    assert!(client.get_connection(server_id.peer_id()).is_none());
    assert_eq!(client.list_connections().len(), 0);

    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}

#[tokio::test]
async fn open_stream_on_unknown_peer_errors() {
    let id = Arc::new(Identity::generate());
    let client = build_client(id).await;
    use synergos_net::quic::StreamType;
    use synergos_net::types::PeerId;
    let result = client
        .open_stream(&PeerId::new("unknown"), StreamType::Control)
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn connect_rejects_mismatched_peer_id() {
    let server_id = Arc::new(Identity::generate());
    let wrong_id = Identity::generate();
    let client_id = Arc::new(Identity::generate());

    let (server, server_addr) = bind_server(server_id.clone()).await;
    let client = build_client(client_id).await;

    let server_task = {
        let server = server.clone();
        tokio::spawn(async move {
            // 相手の handshake 失敗で accept が Err を返す/None を返すのは
            // 許容する (どちらも "接続確立しなかった" ことの表明)
            let _ = server.accept().await;
        })
    };

    let err = client
        .connect(wrong_id.peer_id().clone(), server_addr, "synergos")
        .await
        .expect_err("connect must be rejected on peer id mismatch");

    match err {
        SynergosNetError::Identity(_) | SynergosNetError::Quic(_) => {}
        other => panic!("unexpected error kind: {other:?}"),
    }

    assert!(client.get_connection(wrong_id.peer_id()).is_none());
    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}
