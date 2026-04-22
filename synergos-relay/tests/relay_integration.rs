//! synergos-relay サーバと synergos-net::relay::RelaySession を繋いで
//! wire protocol 互換性 + 2 クライアント間 broadcast を検証する。

use std::time::Duration;

use synergos_net::relay::{RelayData, RelaySession};
use synergos_net::types::PeerId;
use synergos_relay::{run, RelayConfig};

#[tokio::test]
async fn two_clients_exchange_binary_frames_via_relay_server() {
    let cfg = RelayConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    };
    let handle = run(cfg).await.unwrap();
    let url = format!("ws://{}/", handle.local_addr);

    let peer_a = PeerId::new("peer-a");
    let peer_b = PeerId::new("peer-b");

    let sess_a = RelaySession::connect(&url, "room-1", peer_a.clone())
        .await
        .unwrap();
    let sess_b = RelaySession::connect(&url, "room-1", peer_b.clone())
        .await
        .unwrap();

    // サーバ登録の反映を待つ (join は非同期で処理される)
    tokio::time::sleep(Duration::from_millis(150)).await;

    // A → B
    sess_a
        .send(RelayData {
            from_peer_id: peer_a.clone(),
            payload: b"alpha".to_vec(),
        })
        .await
        .unwrap();

    let recv = sess_b.recv(Duration::from_secs(2)).await.unwrap();
    assert_eq!(recv.from_peer_id, peer_a);
    assert_eq!(recv.payload, b"alpha");

    // B → A (逆方向)
    sess_b
        .send(RelayData {
            from_peer_id: peer_b.clone(),
            payload: b"beta".to_vec(),
        })
        .await
        .unwrap();

    let recv = sess_a.recv(Duration::from_secs(2)).await.unwrap();
    assert_eq!(recv.from_peer_id, peer_b);
    assert_eq!(recv.payload, b"beta");

    handle.shutdown().await;
}

#[tokio::test]
async fn sender_does_not_receive_own_broadcast() {
    let cfg = RelayConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    };
    let handle = run(cfg).await.unwrap();
    let url = format!("ws://{}/", handle.local_addr);

    let peer = PeerId::new("solo");
    let sess = RelaySession::connect(&url, "room-solo", peer.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    sess.send(RelayData {
        from_peer_id: peer.clone(),
        payload: b"me".to_vec(),
    })
    .await
    .unwrap();

    // 他にクライアントが居ないので 300ms 以内に受信は無い
    let got = sess.recv(Duration::from_millis(300)).await;
    assert!(got.is_err(), "sender must not receive its own broadcast");

    handle.shutdown().await;
}

#[tokio::test]
async fn different_rooms_are_isolated() {
    let cfg = RelayConfig {
        listen_addr: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    };
    let handle = run(cfg).await.unwrap();
    let url = format!("ws://{}/", handle.local_addr);

    let a = PeerId::new("a");
    let b = PeerId::new("b");

    let sess_a = RelaySession::connect(&url, "room-A", a.clone())
        .await
        .unwrap();
    let sess_b = RelaySession::connect(&url, "room-B", b.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    sess_a
        .send(RelayData {
            from_peer_id: a,
            payload: b"isolate".to_vec(),
        })
        .await
        .unwrap();

    // 別 room なので B には届かない
    let got = sess_b.recv(Duration::from_millis(300)).await;
    assert!(got.is_err(), "different rooms must be isolated");

    handle.shutdown().await;
}
