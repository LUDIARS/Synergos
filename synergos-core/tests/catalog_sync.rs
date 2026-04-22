//! Catalog ドリフト検出: gossip CatalogUpdate が来たとき、ローカル
//! CatalogManager の root_crc と一致しなければ CatalogSyncNeededEvent を
//! emit する (#26)。EventBus 購読でその emit を観測する。

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{CatalogSyncNeededEvent, CoreEventBus, SharedEventBus};
use synergos_core::exchange::Exchange;
use synergos_core::ipc_server::ServiceContext;
use synergos_core::presence::PresenceService;
use synergos_core::project::ProjectManager;
use synergos_net::catalog::CatalogManager;
use synergos_net::gossip::GossipMessage;
use synergos_net::types::{ChunkId, PeerId, TopicId};
use tokio::sync::broadcast;

fn make_ctx() -> Arc<ServiceContext> {
    let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let (shutdown_tx, _) = broadcast::channel(1);
    Arc::new(ServiceContext {
        event_bus: event_bus.clone(),
        project_manager: Arc::new(ProjectManager::new(event_bus.clone())),
        exchange: Arc::new(Exchange::new(event_bus.clone())),
        presence: Arc::new(PresenceService::new(event_bus.clone())),
        conflict_manager: Arc::new(ConflictManager::new(event_bus.clone())),
        shutdown_tx,
        started_at: 0,
        net_config: None,
        catalogs: Arc::new(DashMap::new()),
        content_store: Arc::new(synergos_net::content::MemoryContentStore::new()),
    })
}

/// gossip CatalogUpdate を直接 EventBus 処理系に流し込む単体テスト。
/// Daemon の spawn_gossip_subscriber 内と同じロジックを手書きで呼ぶ。
async fn dispatch_catalog_update(ctx: &ServiceContext, msg: GossipMessage) {
    if let GossipMessage::CatalogUpdate {
        project_id,
        root_crc,
        update_count,
        updated_chunks,
        catalog_cid,
        publisher,
    } = msg
    {
        let local_match = match ctx.catalogs.get(&project_id) {
            Some(cm) => {
                let local = cm.root_catalog().await;
                local.catalog_crc == root_crc && local.update_count == update_count
            }
            None => true,
        };
        if !local_match {
            ctx.event_bus.emit(CatalogSyncNeededEvent {
                project_id,
                remote_root_crc: root_crc,
                remote_update_count: update_count,
                changed_chunks: updated_chunks.iter().map(|c| c.0.clone()).collect(),
                catalog_cid,
                publisher: if publisher.0.is_empty() {
                    None
                } else {
                    Some(publisher.0.clone())
                },
            });
        }
    }
}

#[tokio::test]
async fn catalog_update_matching_local_emits_nothing() {
    let ctx = make_ctx();
    // ローカル catalog を登録
    let cm = Arc::new(CatalogManager::new("proj".into(), 128, 32));
    ctx.catalogs.insert("proj".into(), cm.clone());

    // CatalogUpdate メッセージの root_crc=0 update_count=0 はデフォルトと一致
    let mut rx = ctx.event_bus.subscribe::<CatalogSyncNeededEvent>();

    let root = cm.root_catalog().await;
    dispatch_catalog_update(
        &ctx,
        GossipMessage::CatalogUpdate {
            project_id: "proj".into(),
            root_crc: root.catalog_crc,
            update_count: root.update_count,
            updated_chunks: vec![],
            catalog_cid: None,
            publisher: PeerId::default(),
        },
    )
    .await;

    // 100ms 待って emit が無いことを確認
    let got = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(got.is_err(), "no event should be emitted when in sync");
}

#[tokio::test]
async fn catalog_update_drift_emits_sync_needed_event() {
    let ctx = make_ctx();
    let cm = Arc::new(CatalogManager::new("drift".into(), 128, 32));
    ctx.catalogs.insert("drift".into(), cm.clone());

    let mut rx = ctx.event_bus.subscribe::<CatalogSyncNeededEvent>();

    dispatch_catalog_update(
        &ctx,
        GossipMessage::CatalogUpdate {
            project_id: "drift".into(),
            root_crc: 0xDEAD_BEEF,
            update_count: 99,
            updated_chunks: vec![ChunkId("chunk-1".into()), ChunkId("chunk-2".into())],
            catalog_cid: None,
            publisher: PeerId::default(),
        },
    )
    .await;

    let event = tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .expect("event must arrive")
        .expect("rx open");
    assert_eq!(event.project_id, "drift");
    assert_eq!(event.remote_root_crc, 0xDEAD_BEEF);
    assert_eq!(event.remote_update_count, 99);
    assert_eq!(event.changed_chunks.len(), 2);
}

#[tokio::test]
async fn catalog_update_for_unknown_project_is_ignored() {
    let ctx = make_ctx();
    // catalogs に登録していないプロジェクトの CatalogUpdate は何もしない
    let mut rx = ctx.event_bus.subscribe::<CatalogSyncNeededEvent>();

    dispatch_catalog_update(
        &ctx,
        GossipMessage::CatalogUpdate {
            project_id: "unknown".into(),
            root_crc: 0x1234,
            update_count: 1,
            updated_chunks: vec![],
            catalog_cid: None,
            publisher: PeerId::default(),
        },
    )
    .await;

    let got = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(got.is_err(), "no event for project we're not tracking");
}

/// Smoke test: gossip から publish_updates 経由で CatalogUpdate が流れる
/// ことをローカルに観測する (相手ノード不要、自ノード内の broadcast channel
/// を覗くだけ)。
#[tokio::test]
async fn publish_updates_emits_catalog_update_on_gossip() {
    use synergos_core::exchange::{FileSharing, PublishNotification};
    use synergos_ipc::response::ProjectInfo;

    let bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let gossip_node = {
        let mut g = synergos_net::gossip::GossipNode::new(
            PeerId::new("local"),
            synergos_net::config::GossipsubConfig {
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                heartbeat_interval_ms: 1000,
                message_cache_size: 64,
            },
        );
        g.set_identity(Arc::new(synergos_net::identity::Identity::generate()));
        Arc::new(g)
    };
    let ex = Exchange::with_network(bus, PeerId::new("local"), Some(gossip_node.clone()));
    let mut rx = gossip_node.receiver();

    let topic = TopicId::project("p");
    gossip_node.subscribe(topic);

    ex.publish_updates(vec![PublishNotification {
        project_id: "p".into(),
        file_id: synergos_net::types::FileId::new("a.txt"),
        file_path: std::path::PathBuf::from("/tmp/a.txt"),
        file_size: 10,
        crc: 0x1234,
        version: 1,
    }])
    .await
    .unwrap();

    // FileOffer と CatalogUpdate が local broadcast に流れる
    let mut saw_offer = false;
    let mut saw_catalog = false;
    for _ in 0..3 {
        let Ok(Ok((_topic, msg))) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        else {
            break;
        };
        match msg {
            GossipMessage::FileOffer { .. } => saw_offer = true,
            GossipMessage::CatalogUpdate { .. } => saw_catalog = true,
            _ => {}
        }
    }
    assert!(saw_offer, "FileOffer should be published");
    assert!(saw_catalog, "CatalogUpdate should be published");
    // ProjectInfo は使わないが import を活かすために no-op
    let _: Option<ProjectInfo> = None;
}
