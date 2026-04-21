//! Exchange::execute_send と handle_incoming_transfer を 2 ノード QUIC 間で
//! 往復させる E2E。S1 認証、`TXFR` magic ディスパッチ、進捗イベント、
//! complete_transfer までが繋がっていることを確認する。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{Exchange, FileSharing, OutPathResolver, ShareRequest};
use synergos_net::config::QuicConfig;
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use synergos_net::transfer::{hash_file, TransferHeader, TRANSFER_STREAM_MAGIC};
use synergos_net::types::{Blake3Hash, FileId, TransferId};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
    }
}

#[tokio::test]
async fn execute_send_and_handle_incoming_roundtrip() {
    use synergos_core::exchange::TransferPriority;

    let _ = tracing_subscriber::fmt::try_init();

    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let server_quic = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let client_quic = Arc::new(QuicManager::new(qcfg(), client_id.clone()));

    let server_addr: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let bound = server_quic.bind(server_addr).await.unwrap();
    client_quic
        .bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();

    // サーバ側 accept + dispatcher を spawn
    let dst_dir = std::env::temp_dir().join(format!("synergos-e2e-dst-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dst_dir).await.unwrap();

    let event_bus_server: SharedEventBus = Arc::new(CoreEventBus::new());
    let mut ex_server =
        Exchange::with_network(event_bus_server.clone(), server_id.peer_id().clone(), None);
    let dst_dir_clone = dst_dir.clone();
    let resolver: OutPathResolver =
        Arc::new(move |_p, file_id: &FileId| Some(dst_dir_clone.join(&file_id.0)));
    ex_server.attach_quic(server_quic.clone(), resolver);
    let ex_server = Arc::new(ex_server);

    let sq = server_quic.clone();
    let ex = ex_server.clone();
    let accept_task = tokio::spawn(async move {
        // accept 1 回 + bidi 1 本 を処理する最小ディスパッチャ
        if let Ok(Some(acc)) = sq.accept().await {
            let sender = acc.peer_id.clone();
            let connection = acc.connection;
            while let Ok((send, mut recv)) = connection.accept_bi().await {
                let _ = send; // 受信側は不要
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == TRANSFER_STREAM_MAGIC {
                    let _ = ex.handle_incoming_transfer(recv, sender.clone()).await;
                }
                break;
            }
        }
    });

    // ソースファイルを用意
    let src_dir = std::env::temp_dir().join(format!("synergos-e2e-src-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&src_dir).await.unwrap();
    let src_file = src_dir.join("sample.bin");
    let payload = (0..200u32)
        .flat_map(|n| (n as u8).to_le_bytes())
        .cycle()
        .take(256 * 1024)
        .collect::<Vec<u8>>();
    tokio::fs::write(&src_file, &payload).await.unwrap();

    // クライアント→サーバへ接続
    client_quic
        .connect(server_id.peer_id().clone(), bound, "synergos")
        .await
        .unwrap();

    // クライアント側 Exchange
    let event_bus_client: SharedEventBus = Arc::new(CoreEventBus::new());
    let mut ex_client =
        Exchange::with_network(event_bus_client.clone(), client_id.peer_id().clone(), None);
    let dummy_resolver: OutPathResolver = Arc::new(|_, _| None);
    ex_client.attach_quic(client_quic.clone(), dummy_resolver);
    let ex_client = Arc::new(ex_client);

    // share_file を直接呼ぶ代わりに execute_send を発射
    let (total_hash, total_size, chunk_count) = hash_file(&src_file).await.unwrap();
    let file_id = FileId::new("sample.bin");
    let transfer_id = TransferId::generate();
    let header = TransferHeader {
        transfer_id: transfer_id.0.clone(),
        project_id: "proj-1".into(),
        file_id: file_id.0.clone(),
        total_size,
        chunk_count,
        total_hash,
    };

    // クライアント側でも ActiveTransfer を用意しておく (execute_send が更新する対象)
    ex_client
        .share_file(ShareRequest {
            project_id: "proj-1".into(),
            file_id: file_id.clone(),
            file_path: src_file.clone(),
            file_size: total_size,
            checksum: Blake3Hash::default(),
            priority: TransferPriority::Interactive,
            target_peer: Some(server_id.peer_id().clone()),
            version: 1,
        })
        .await
        .unwrap();

    // ActiveTransfer の transfer_id が自動生成されるので、新規に作り直して
    // execute_send に渡す — ここでは直接低レベル API をテストする。
    ex_client
        .execute_send(
            transfer_id.clone(),
            server_id.peer_id().clone(),
            src_file.clone(),
            header,
        )
        .await
        .unwrap();

    // 受信側の完了を待つ
    tokio::time::timeout(Duration::from_secs(5), accept_task)
        .await
        .expect("accept task should finish")
        .unwrap();

    // サーバ側が完了ファイルを保存していることを確認
    let received = tokio::fs::read(dst_dir.join("sample.bin")).await.unwrap();
    assert_eq!(received.len(), payload.len());
    assert_eq!(received, payload);

    // クリーンアップ
    let _ = tokio::fs::remove_dir_all(&src_dir).await;
    let _ = tokio::fs::remove_dir_all(&dst_dir).await;
}
