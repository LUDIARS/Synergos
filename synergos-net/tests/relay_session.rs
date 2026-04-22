//! WebSocket Relay セッションの単体テスト。
//! in-process で echo-style のリレーサーバを起動し、2 クライアント間で
//! バイナリ frame が流れるかを確認する。

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use synergos_net::relay::{RelayData, RelaySession};
use synergos_net::types::PeerId;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// 簡易 relay サーバ: 1 room のみ想定、最初の join メッセージは捨てて、
/// 以降の binary フレームを他の接続に bcast する。
async fn run_echo_relay(listener: TcpListener) {
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let clients: Arc<RwLock<Vec<tokio::sync::mpsc::Sender<Vec<u8>>>>> =
        Arc::new(RwLock::new(Vec::new()));

    while let Ok((stream, _)) = listener.accept().await {
        let ws = match accept_async(stream).await {
            Ok(w) => w,
            Err(_) => continue,
        };
        let (mut writer, mut reader) = ws.split();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        clients.write().await.push(tx);

        // writer ループ
        tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if writer.send(Message::binary(bytes)).await.is_err() {
                    break;
                }
            }
        });

        // reader ループ: join を無視し、以降の binary を他クライアントに bcast
        let clients_clone = clients.clone();
        tokio::spawn(async move {
            let mut joined = false;
            while let Some(Ok(msg)) = reader.next().await {
                match msg {
                    Message::Text(_) if !joined => {
                        joined = true;
                    }
                    Message::Binary(bytes) => {
                        let senders = clients_clone.read().await.clone();
                        for s in senders {
                            let _ = s.try_send(bytes.to_vec());
                        }
                    }
                    _ => {}
                }
            }
        });
    }
}

#[tokio::test]
async fn relay_roundtrip_between_two_clients() {
    // relay サーバを bind
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let url = format!("ws://{}/", addr);
    tokio::spawn(run_echo_relay(listener));

    let peer_a = PeerId::new("a");
    let peer_b = PeerId::new("b");

    let session_a = RelaySession::connect(&url, "room-x", peer_a.clone())
        .await
        .unwrap();
    let session_b = RelaySession::connect(&url, "room-x", peer_b.clone())
        .await
        .unwrap();

    // クライアント接続が echo relay に登録されるまで少し待つ
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A → サーバ bcast → A 自身と B が受け取る。
    // 最小 echo relay では sender 自身にも返るので、B の受信のみ検証する。
    session_a
        .send(RelayData {
            from_peer_id: peer_a.clone(),
            payload: b"hello".to_vec(),
        })
        .await
        .unwrap();

    // B 側が受信するかを確認 (自分が出した echo も混在する可能性があるので
    // peer_id で区別する)
    let mut got = None;
    for _ in 0..5 {
        let data = session_b.recv(Duration::from_millis(500)).await;
        match data {
            Ok(d) if d.from_peer_id == peer_a => {
                got = Some(d);
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    let data = got.expect("B should receive A's frame within 2.5s");
    assert_eq!(data.payload, b"hello");
}
