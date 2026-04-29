//! CatalogSyncService の E2E:
//! 2 ノード (publisher / subscriber) を QUIC で繋ぎ、publisher が
//! ContentStore に RootCatalog を put した状態で subscriber に
//! `CatalogSyncNeededEvent` を emit → BitswapSession が走って
//! `CatalogSyncCompletedEvent` に成功が乗ることを確認する (#25 + #26)。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_core::catalog_sync::CatalogSyncService;
use synergos_core::event_bus::{
    CatalogSyncCompletedEvent, CatalogSyncNeededEvent, CoreEventBus, SharedEventBus,
};
use synergos_net::catalog::{ChunkIndex, FileEntry, FileState, RootCatalog};
use synergos_net::config::QuicConfig;
use synergos_net::content::{
    handle_bitswap_stream, Block, ContentStore, MemoryContentStore, BITSWAP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use synergos_net::types::{Blake3Hash, ChunkId, FileId};
use tokio::sync::broadcast;

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 32,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

fn sample_root_catalog() -> RootCatalog {
    RootCatalog {
        project_id: "p".into(),
        update_count: 7,
        chunks: vec![ChunkIndex {
            chunk_id: ChunkId("chunk-1".into()),
            crc: 0xDEAD,
            content_hash: Blake3Hash::default(),
            last_updated: 1_700_000_000_000,
        }],
        catalog_crc: 0xBEEF,
        last_updated: 1_700_000_000_000,
    }
}

fn _force_file_entry_use() -> FileEntry {
    FileEntry {
        file_id: FileId::new("x"),
        path: "x".into(),
        crc: 0,
        content_hash: Blake3Hash::default(),
        state: FileState::Synced,
        size: 0,
    }
}

/// publisher 側の受け入れループを起動。BSW1 ストリームを個別タスクで処理。
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

#[tokio::test]
async fn catalog_sync_e2e_fetches_root_catalog_over_bitswap() {
    // ── publisher (server) ──
    let pub_id = Arc::new(Identity::generate());
    let sub_id = Arc::new(Identity::generate());
    let pub_quic = Arc::new(QuicManager::new(qcfg(), pub_id.clone()));
    let sub_quic = Arc::new(QuicManager::new(qcfg(), sub_id));
    let pub_addr = pub_quic
        .bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let _ = sub_quic
        .bind((Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();

    // publisher 側で RootCatalog を ContentStore に put する
    let pub_store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let snapshot = sample_root_catalog();
    let bytes = rmp_serde::to_vec(&snapshot).unwrap();
    let block = Block::new(bytes);
    let catalog_cid = block.cid.clone();
    pub_store.put(block).await.unwrap();

    let _pub_task = spawn_bitswap_server(pub_quic.clone(), pub_store.clone());

    // ── subscriber ──
    sub_quic
        .connect(pub_id.peer_id().clone(), pub_addr, "synergos")
        .await
        .unwrap();

    let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let sub_store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let service_task = CatalogSyncService::spawn(
        event_bus.clone(),
        sub_quic.clone(),
        sub_store.clone(),
        shutdown_rx,
    );

    // Completed event を待ち受ける
    let mut done_rx = event_bus.subscribe::<CatalogSyncCompletedEvent>();

    event_bus.emit(CatalogSyncNeededEvent {
        project_id: "p".into(),
        remote_root_crc: snapshot.catalog_crc,
        remote_update_count: snapshot.update_count,
        changed_chunks: vec!["chunk-1".into()],
        catalog_cid: Some(catalog_cid.clone()),
        publisher: Some(pub_id.peer_id().0.clone()),
    });

    let completed = tokio::time::timeout(Duration::from_secs(3), done_rx.recv())
        .await
        .expect("CatalogSyncCompletedEvent timeout")
        .expect("rx open");
    assert_eq!(completed.project_id, "p");
    assert_eq!(completed.remote_root_crc, snapshot.catalog_crc);
    assert_eq!(completed.remote_update_count, snapshot.update_count);
    assert!(
        completed.error.is_none(),
        "expected success, got error: {:?}",
        completed.error
    );
    assert_eq!(completed.fetched_blocks, 1);

    // subscriber 側 store に catalog_cid が入っていること
    assert!(sub_store.has(&catalog_cid).await);

    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_millis(300), service_task).await;
    drop(sub_quic);
}
