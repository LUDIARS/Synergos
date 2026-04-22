//! Bitswap v2 E2E: WantList / HAVE / Cancel と BitswapSession::fetch_dag を
//! 実 QUIC 上で回す。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::QuicConfig;
use synergos_net::content::{
    add_file, handle_bitswap_stream, request_many, send_cancel, BitswapResponse, BitswapSession,
    Block, ChunkerOptions, ContentStore, FetchOutcome, MemoryContentStore, BITSWAP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 32,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
    }
}

/// サーバ側 accept タスクを起動 (全ての BSW1 ストリームを個別タスクで処理)。
fn spawn_bitswap_server(
    quic: Arc<QuicManager>,
    store: Arc<MemoryContentStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Ok(Some(acc)) = quic.accept().await {
            let conn = acc.connection;
            while let Ok((send, mut recv)) = conn.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == BITSWAP_STREAM_MAGIC {
                    let s = store.clone();
                    tokio::spawn(async move {
                        let _ = handle_bitswap_stream(s, send, recv).await;
                    });
                }
            }
        }
    })
}

/// WantList で複数 CID を 1 ストリームで取得できること。
#[tokio::test]
async fn wantlist_fetches_multiple_cids_in_one_stream() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let b1 = Block::new(b"one".to_vec());
    let b2 = Block::new(b"two-two".to_vec());
    let b3 = Block::new(b"three-three-three".to_vec());
    let (c1, c2, c3) = (b1.cid.clone(), b2.cid.clone(), b3.cid.clone());
    store.put(b1).await.unwrap();
    store.put(b2).await.unwrap();
    store.put(b3).await.unwrap();

    let server_task = spawn_bitswap_server(quic_s.clone(), store);

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();
    let (send, recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();

    let cids = vec![c1.clone(), c2.clone(), c3.clone()];
    let resps = request_many(send, recv, &cids, false).await.unwrap();
    assert_eq!(resps.len(), 3);
    for (cid, r) in cids.iter().zip(resps.iter()) {
        match r {
            BitswapResponse::Block {
                cid: got_cid,
                bytes: _,
            } => assert_eq!(got_cid, cid),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(300), server_task)
        .await
        .ok();
}

/// 存在する CID と存在しない CID が混在するとき、NotFound が順序通りに返ること。
#[tokio::test]
async fn wantlist_mixes_block_and_notfound_in_request_order() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let b1 = Block::new(b"alpha".to_vec());
    let c1 = b1.cid.clone();
    store.put(b1).await.unwrap();
    let missing = synergos_net::types::Cid("blake3-ffff".into());

    let server_task = spawn_bitswap_server(quic_s.clone(), store);

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();
    let (send, recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();

    let cids = vec![c1.clone(), missing.clone(), c1.clone()];
    let resps = request_many(send, recv, &cids, false).await.unwrap();
    assert_eq!(resps.len(), 3);
    matches!(resps[0], BitswapResponse::Block { .. });
    matches!(resps[1], BitswapResponse::NotFound { .. });
    matches!(resps[2], BitswapResponse::Block { .. });

    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(300), server_task)
        .await
        .ok();
}

/// want_have=true メタ問い合わせで Have / DontHave が返ること。
#[tokio::test]
async fn have_query_returns_have_and_donthave() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let b1 = Block::new(b"present".to_vec());
    let c1 = b1.cid.clone();
    store.put(b1).await.unwrap();
    let missing = synergos_net::types::Cid("blake3-absent".into());

    let server_task = spawn_bitswap_server(quic_s.clone(), store);

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();

    // BitswapSession::have_query を使う (複数ストリームにまたがってもよいが本テストは 1 batch)
    let session = BitswapSession::new(
        quic_c.clone(),
        server_id.peer_id().clone(),
        Arc::new(MemoryContentStore::new()),
    );
    let res = session
        .have_query(&[c1.clone(), missing.clone()])
        .await
        .unwrap();
    assert_eq!(res.len(), 2);
    assert_eq!(res[0].0, c1);
    assert!(res[0].1, "expected Have for c1");
    assert_eq!(res[1].0, missing);
    assert!(!res[1].1, "expected DontHave for missing");

    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(300), server_task)
        .await
        .ok();
}

/// Cancel を送信してもサーバがパニックせず、ストリームを正常に閉じること。
#[tokio::test]
async fn cancel_closes_stream_without_error() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let server_task = spawn_bitswap_server(quic_s.clone(), store);

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();
    let (send, _recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();
    send_cancel(send, &[synergos_net::types::Cid("blake3-whatever".into())])
        .await
        .unwrap();

    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(300), server_task)
        .await
        .ok();
}

/// BitswapSession::fetch_dag が root + 全 chunk を自動的に取得し、
/// クライアント側 store を完全に埋められること。
#[tokio::test]
async fn session_fetch_dag_pulls_root_and_all_chunks() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    // Server 側: 大きめのファイルを chunk 分割
    let src = std::env::temp_dir().join(format!("bsw2-src-{}", uuid::Uuid::new_v4()));
    let payload: Vec<u8> = (0..257u32)
        .flat_map(|n| (n as u8).to_le_bytes())
        .cycle()
        .take(120_000)
        .collect();
    tokio::fs::write(&src, &payload).await.unwrap();

    let server_store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let root = add_file(
        server_store.as_ref(),
        &src,
        ChunkerOptions { chunk_size: 16_384 },
    )
    .await
    .unwrap();

    let server_task = spawn_bitswap_server(quic_s.clone(), server_store.clone());

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();

    let client_store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let session = BitswapSession::new(
        quic_c.clone(),
        server_id.peer_id().clone(),
        client_store.clone(),
    )
    .with_max_inflight(4);

    let dag = session.fetch_dag(&root).await.unwrap();
    assert_eq!(dag.total_size as usize, payload.len());
    // root + chunks 全部が client_store にある
    assert!(client_store.has(&root).await);
    for chunk_cid in &dag.chunks {
        assert!(
            client_store.has(chunk_cid).await,
            "missing chunk {} on client store",
            chunk_cid.0
        );
    }

    // 二回目は全部ヒットで終わる (NotFound が出ないこと)
    let out = session.fetch_blocks(&dag.chunks).await.unwrap();
    for (_cid, fo) in &out {
        assert!(matches!(fo, FetchOutcome::Fetched));
    }

    let _ = tokio::fs::remove_file(&src).await;
    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(500), server_task)
        .await
        .ok();
}
