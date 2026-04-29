//! Bitswap E2E: 2 ノードで片方の ContentStore に block を入れ、もう片方が
//! QUIC `BSW1` ストリームで WANT → BLOCK を往復させる。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::QuicConfig;
use synergos_net::content::{
    add_file, get_file, handle_bitswap_stream, request_block, BitswapResponse, Block,
    ChunkerOptions, ContentStore, MemoryContentStore, BITSWAP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

#[tokio::test]
async fn bitswap_fetch_single_block_over_quic() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());

    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    // Server store: 1 block を事前に積む
    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let block = Block::new(b"hello-bitswap".to_vec());
    let cid = block.cid.clone();
    store.put(block).await.unwrap();

    // Server accept → BSW1 dispatch
    let store_s = store.clone();
    let qs = quic_s.clone();
    let server_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = qs.accept().await {
            let conn = acc.connection;
            while let Ok((send, mut recv)) = conn.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == BITSWAP_STREAM_MAGIC {
                    let _ = handle_bitswap_stream(store_s.clone(), send, recv).await;
                    break;
                }
            }
        }
    });

    // Client connect + WANT
    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();
    let (send, recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();

    let resp = request_block(send, recv, &cid).await.unwrap();
    match resp {
        BitswapResponse::Block {
            cid: got_cid,
            bytes,
        } => {
            assert_eq!(got_cid, cid);
            assert_eq!(bytes, b"hello-bitswap");
        }
        other => panic!("expected Block, got {other:?}"),
    }

    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}

#[tokio::test]
async fn bitswap_not_found_for_unknown_cid() {
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let store_s = store.clone();
    let qs = quic_s.clone();
    let server_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = qs.accept().await {
            let conn = acc.connection;
            while let Ok((send, mut recv)) = conn.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == BITSWAP_STREAM_MAGIC {
                    let _ = handle_bitswap_stream(store_s.clone(), send, recv).await;
                    break;
                }
            }
        }
    });

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();
    let (send, recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();

    let cid = synergos_net::types::Cid("blake3-unknown".into());
    let resp = request_block(send, recv, &cid).await.unwrap();
    matches!(resp, BitswapResponse::NotFound { .. });

    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}

#[tokio::test]
async fn chunker_plus_bitswap_reassembles_file() {
    // Peer A (server) が大きなファイルを add_file → store に多数の block
    // Peer B (client) が root CID を知っている前提で、DAG を引き子 CID を順に引いて
    // 再組立。実運用では (1) root CID を gossip 経由で知らせる + (2) 各 chunk を
    // bitswap で取りに行く、という組合わせになる。
    let server_id = Arc::new(Identity::generate());
    let client_id = Arc::new(Identity::generate());
    let quic_s = Arc::new(QuicManager::new(qcfg(), server_id.clone()));
    let quic_c = Arc::new(QuicManager::new(qcfg(), client_id));
    let addr_s = quic_s.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let _ = quic_c.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let src = std::env::temp_dir().join(format!("ipfs-src-{}", uuid::Uuid::new_v4()));
    let payload: Vec<u8> = (0..257u32)
        .flat_map(|n| (n as u8).to_le_bytes())
        .cycle()
        .take(200_000)
        .collect();
    tokio::fs::write(&src, &payload).await.unwrap();

    let server_store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let root = add_file(
        server_store.as_ref(),
        &src,
        ChunkerOptions { chunk_size: 32_768 },
    )
    .await
    .unwrap();

    // Server accept loop: 複数の bitswap stream を連続で処理
    let store_s = server_store.clone();
    let qs = quic_s.clone();
    let server_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = qs.accept().await {
            let conn = acc.connection;
            while let Ok((send, mut recv)) = conn.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == BITSWAP_STREAM_MAGIC {
                    let s = store_s.clone();
                    tokio::spawn(async move {
                        let _ = handle_bitswap_stream(s, send, recv).await;
                    });
                }
            }
        }
    });

    quic_c
        .connect(server_id.peer_id().clone(), addr_s, "synergos")
        .await
        .unwrap();

    // client-side store: bitswap で引いた block を積んでいく
    let client_store = MemoryContentStore::new();

    // まず root DAG を引く
    let (send, recv) = quic_c
        .open_stream(server_id.peer_id(), StreamType::Control)
        .await
        .unwrap();
    match request_block(send, recv, &root).await.unwrap() {
        BitswapResponse::Block { cid, bytes } => {
            client_store.put(Block { cid, bytes }).await.unwrap();
        }
        other => panic!("expected root block, got {other:?}"),
    }

    // root block を client store から引いて DAG を decode、各 chunk を fetch
    let root_block = client_store.get(&root).await.unwrap().unwrap();
    let dag: synergos_net::content::ChunkDag = rmp_serde::from_slice(&root_block.bytes).unwrap();
    for chunk_cid in &dag.chunks {
        let (send, recv) = quic_c
            .open_stream(server_id.peer_id(), StreamType::Control)
            .await
            .unwrap();
        match request_block(send, recv, chunk_cid).await.unwrap() {
            BitswapResponse::Block { cid, bytes } => {
                client_store.put(Block { cid, bytes }).await.unwrap();
            }
            other => panic!("expected chunk block, got {other:?}"),
        }
    }

    // local store から get_file で元ファイルを再生成
    let dst = std::env::temp_dir().join(format!("ipfs-dst-{}", uuid::Uuid::new_v4()));
    let size = get_file(&client_store, &root, &dst).await.unwrap();
    assert_eq!(size as usize, payload.len());
    let got = tokio::fs::read(&dst).await.unwrap();
    assert_eq!(got, payload);

    let _ = tokio::fs::remove_file(&src).await;
    let _ = tokio::fs::remove_file(&dst).await;
    drop(quic_c);
    tokio::time::timeout(Duration::from_millis(200), server_task)
        .await
        .ok();
}

/// 変数が使われないので警告抑制用の dummy 型アクセス
#[allow(dead_code)]
fn _type_cover() -> Option<SocketAddr> {
    None
}
