//! CatalogSyncService: `CatalogSyncNeededEvent` を購読し、
//! 発行元ピアから `catalog_cid` を Bitswap で取り寄せて RootCatalog を
//! デシリアライズするところまで自動化する (#25 / #26)。
//!
//! 実ファイル転送 (FileWant → TXFR) は既存の gossip + transfer 経路が
//! 担当するので、本サービスは「ドリフト検知 → full diff に必要な
//! RootCatalog を取り寄せる」部分だけを埋める。取得完了時に
//! `CatalogSyncCompletedEvent` を emit する。
//!
//! 失敗シナリオ (catalog_cid 無し / publisher 不明 / Bitswap NotFound 等) も
//! `CatalogSyncCompletedEvent { error: Some(_) }` として可視化する。

use std::sync::Arc;

use synergos_net::content::{BitswapSession, ContentStore, MemoryContentStore};
use synergos_net::quic::QuicManager;
use synergos_net::types::{Cid, PeerId};
use tokio::sync::broadcast;

use crate::event_bus::{
    CatalogSyncCompletedEvent, CatalogSyncNeededEvent, CoreEventBus, SharedEventBus,
};

/// サービスハンドル。`spawn` が返すので呼び出し側は `abort_handle` などを
/// 保持する必要は無い (shutdown_rx で終端する)。
pub struct CatalogSyncService {
    _phantom: std::marker::PhantomData<()>,
}

impl CatalogSyncService {
    /// EventBus / QUIC / ContentStore を受け取って購読タスクを起動する。
    /// `shutdown_rx` が発火するまでイベントを処理し続ける。
    pub fn spawn(
        event_bus: SharedEventBus,
        quic: Arc<QuicManager>,
        content_store: Arc<MemoryContentStore>,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) -> tokio::task::JoinHandle<()> {
        // `emit` は受信者がいない場合にドロップするので、spawn 側の
        // tokio::spawn 内で subscribe する前に emit されるとイベントを取りこぼす。
        // ここで同期的に subscribe してから tokio::spawn に受信機を渡す。
        let mut rx = event_bus.subscribe::<CatalogSyncNeededEvent>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    evt = rx.recv() => {
                        match evt {
                            Ok(e) => {
                                let eb = event_bus.clone();
                                let q = quic.clone();
                                let cs = content_store.clone();
                                tokio::spawn(async move {
                                    process_sync_event(e, eb, q, cs).await;
                                });
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("catalog_sync subscriber lagged: dropped {n} events");
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            tracing::debug!("catalog_sync service stopped");
        })
    }
}

/// 1 つの CatalogSyncNeededEvent を処理し、完了時に CatalogSyncCompletedEvent を emit する。
async fn process_sync_event(
    evt: CatalogSyncNeededEvent,
    event_bus: SharedEventBus,
    quic: Arc<QuicManager>,
    content_store: Arc<MemoryContentStore>,
) {
    let catalog_cid = match &evt.catalog_cid {
        Some(c) => c.clone(),
        None => {
            tracing::info!(
                "catalog_sync skip: project={} no catalog_cid in gossip (legacy peer?)",
                evt.project_id
            );
            emit_completed(
                &event_bus,
                &evt,
                0,
                Some("catalog_cid not provided by publisher (legacy peer)".into()),
            );
            return;
        }
    };

    let publisher = match &evt.publisher {
        Some(p) if !p.is_empty() => PeerId::new(p.clone()),
        _ => {
            tracing::warn!(
                "catalog_sync skip: project={} publisher not specified in gossip",
                evt.project_id
            );
            emit_completed(
                &event_bus,
                &evt,
                0,
                Some("publisher not specified in gossip".into()),
            );
            return;
        }
    };

    tracing::info!(
        "catalog_sync start: project={} publisher={} catalog_cid={}",
        evt.project_id,
        publisher.short(),
        catalog_cid.0
    );

    match fetch_root_catalog(
        quic.clone(),
        publisher.clone(),
        content_store.clone(),
        &catalog_cid,
    )
    .await
    {
        Ok(fetched) => {
            tracing::info!(
                "catalog_sync done: project={} fetched_blocks={fetched} root_cid={}",
                evt.project_id,
                catalog_cid.0
            );
            emit_completed(&event_bus, &evt, fetched, None);
        }
        Err(err) => {
            tracing::warn!("catalog_sync failed: project={} err={err}", evt.project_id);
            emit_completed(&event_bus, &evt, 0, Some(err));
        }
    }
}

/// 実際の Bitswap fetch。取得した RootCatalog を ContentStore に入れ、
/// 加えてその中身 (chunks 一覧) を返す。
/// 返り値は「取得した block の数」(root 1 件 + (将来的には) 子 chunk 数)。
async fn fetch_root_catalog(
    quic: Arc<QuicManager>,
    publisher: PeerId,
    content_store: Arc<MemoryContentStore>,
    catalog_cid: &Cid,
) -> Result<u32, String> {
    // すでにローカルに同じ CID を持っていれば skip できる
    if content_store.has(catalog_cid).await {
        return Ok(0);
    }

    let session = BitswapSession::new(quic, publisher, content_store.clone());
    let outcomes = session
        .fetch_blocks(std::slice::from_ref(catalog_cid))
        .await
        .map_err(|e| format!("bitswap fetch: {e}"))?;

    let got = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, synergos_net::content::FetchOutcome::Fetched))
        .count();
    if got == 0 {
        return Err("publisher reported NotFound for catalog_cid".into());
    }

    // 取得した block を RootCatalog としてデコードできるかだけ検証する
    let blk = content_store
        .get(catalog_cid)
        .await
        .map_err(|e| format!("content_store get: {e}"))?
        .ok_or_else(|| "content_store missing after fetch".to_string())?;
    let _: synergos_net::catalog::RootCatalog =
        rmp_serde::from_slice(&blk.bytes).map_err(|e| format!("RootCatalog decode: {e}"))?;

    Ok(got as u32)
}

fn emit_completed(
    event_bus: &Arc<CoreEventBus>,
    evt: &CatalogSyncNeededEvent,
    fetched: u32,
    error: Option<String>,
) {
    event_bus.emit(CatalogSyncCompletedEvent {
        project_id: evt.project_id.clone(),
        remote_root_crc: evt.remote_root_crc,
        remote_update_count: evt.remote_update_count,
        fetched_blocks: fetched,
        error,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use synergos_net::identity::Identity;

    fn qcfg() -> synergos_net::config::QuicConfig {
        synergos_net::config::QuicConfig {
            max_concurrent_streams: 8,
            idle_timeout_ms: 5_000,
            max_udp_payload_size: 1350,
            enable_0rtt: false,
        }
    }

    /// catalog_cid が無い event は "legacy" として Completed(error) になり、
    /// Bitswap 経路には行かないこと。
    #[tokio::test]
    async fn skips_when_catalog_cid_missing() {
        let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
        let id = Arc::new(Identity::generate());
        let quic = Arc::new(QuicManager::new(qcfg(), id));
        let _ = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
        let cs = Arc::new(MemoryContentStore::new());
        let (tx, rx) = broadcast::channel(1);

        let mut done_rx = event_bus.subscribe::<CatalogSyncCompletedEvent>();
        let _task = CatalogSyncService::spawn(event_bus.clone(), quic, cs, rx);

        // emit event with no catalog_cid
        event_bus.emit(CatalogSyncNeededEvent {
            project_id: "p".into(),
            remote_root_crc: 0x1,
            remote_update_count: 1,
            changed_chunks: vec![],
            catalog_cid: None,
            publisher: None,
        });

        let got = tokio::time::timeout(std::time::Duration::from_millis(500), done_rx.recv())
            .await
            .expect("completed event not received")
            .unwrap();
        assert_eq!(got.fetched_blocks, 0);
        assert!(got.error.is_some(), "should report error when cid missing");

        let _ = tx.send(());
    }
}
