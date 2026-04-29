//! PR-9: IPC Subscribe event push loop のテスト。
//! handle_client_generic を duplex でドライブして、Subscribe 後に
//! EventBus へ emit したイベントがクライアント側に届くことを確認する。

use std::sync::Arc;
use std::time::Duration;

use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{
    CoreEventBus, PeerConnectedEvent, SharedEventBus, TransferProgressEvent,
};
use synergos_core::exchange::Exchange;
use synergos_core::ipc_server::{handle_client_generic, ServiceContext};
use synergos_core::presence::PresenceService;
use synergos_core::project::ProjectManager;
use synergos_ipc::command::IpcCommand;
use synergos_ipc::event::{EventCategory, EventFilter, IpcEvent};
use synergos_ipc::response::IpcResponse;
use synergos_ipc::transport::{IpcTransport, ServerMessage};
use synergos_net::config::QuicConfig;
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use tokio::io::duplex;
use tokio::sync::{broadcast, Mutex};

fn make_ctx() -> Arc<ServiceContext> {
    let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let (shutdown_tx, _) = broadcast::channel(1);
    let quic = Arc::new(QuicManager::new(
        QuicConfig {
            max_concurrent_streams: 8,
            idle_timeout_ms: 5_000,
            max_udp_payload_size: 1350,
            enable_0rtt: false,
            listen_addr: None,
        },
        Arc::new(Identity::generate()),
    ));
    Arc::new(ServiceContext {
        event_bus: event_bus.clone(),
        project_manager: Arc::new(ProjectManager::new(event_bus.clone())),
        exchange: Arc::new(Exchange::new(event_bus.clone())),
        presence: Arc::new(PresenceService::new(event_bus.clone())),
        conflict_manager: Arc::new(ConflictManager::new(event_bus.clone())),
        shutdown_tx,
        started_at: 0,
        net_config: None,
        catalogs: Arc::new(dashmap::DashMap::new()),
        content_store: Arc::new(synergos_net::content::MemoryContentStore::new()),
        quic,
    })
}

#[tokio::test]
async fn subscribe_all_delivers_peer_connected_event() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, mut client_rx) = duplex(64 * 1024);
    let writer = Arc::new(Mutex::new(server_tx));

    let server_task = {
        let ctx = ctx.clone();
        let writer = writer.clone();
        tokio::spawn(async move {
            let _ = handle_client_generic(server_rx, writer, ctx).await;
        })
    };

    // Subscribe { events: [] } = All
    IpcTransport::write_message(&mut client_tx, &IpcCommand::Subscribe { events: vec![] })
        .await
        .unwrap();

    // Subscribed 応答を読む
    let first: ServerMessage = IpcTransport::read_message(&mut client_rx).await.unwrap();
    match first {
        ServerMessage::Response(IpcResponse::Subscribed { .. }) => (),
        other => panic!("expected Subscribed, got {other:?}"),
    }

    // EventBus に Peer イベントを投げる
    ctx.event_bus.emit(PeerConnectedEvent {
        project_id: "p".into(),
        peer_id: "peer-x".into(),
        display_name: "x".into(),
        route: "Direct".into(),
        rtt_ms: 10,
    });

    // クライアントが Event として受信できるはず
    let msg = tokio::time::timeout(
        Duration::from_secs(2),
        IpcTransport::read_message::<_, ServerMessage>(&mut client_rx),
    )
    .await
    .expect("event must arrive within timeout")
    .expect("event decodes");
    match msg {
        ServerMessage::Event(IpcEvent::PeerConnected { peer_id, .. }) => {
            assert_eq!(peer_id, "peer-x");
        }
        other => panic!("expected PeerConnected, got {other:?}"),
    }

    drop(client_tx);
    let _ = server_task.await;
}

#[tokio::test]
async fn unsubscribe_stops_event_push() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, mut client_rx) = duplex(64 * 1024);
    let writer = Arc::new(Mutex::new(server_tx));

    let server_task = {
        let ctx = ctx.clone();
        let writer = writer.clone();
        tokio::spawn(async move {
            let _ = handle_client_generic(server_rx, writer, ctx).await;
        })
    };

    IpcTransport::write_message(&mut client_tx, &IpcCommand::Subscribe { events: vec![] })
        .await
        .unwrap();
    let _ = IpcTransport::read_message::<_, ServerMessage>(&mut client_rx)
        .await
        .unwrap();

    // Unsubscribe
    IpcTransport::write_message(
        &mut client_tx,
        &IpcCommand::Unsubscribe {
            subscription_id: "s1".into(),
        },
    )
    .await
    .unwrap();
    let ack: ServerMessage = IpcTransport::read_message(&mut client_rx).await.unwrap();
    matches!(ack, ServerMessage::Response(IpcResponse::Ok));

    // 以降に emit したイベントは届かない
    ctx.event_bus.emit(PeerConnectedEvent {
        project_id: "p".into(),
        peer_id: "peer-y".into(),
        display_name: "y".into(),
        route: "Direct".into(),
        rtt_ms: 10,
    });

    let maybe = tokio::time::timeout(
        Duration::from_millis(200),
        IpcTransport::read_message::<_, ServerMessage>(&mut client_rx),
    )
    .await;
    assert!(
        maybe.is_err() || matches!(maybe, Ok(Err(_))),
        "no event should arrive after Unsubscribe"
    );

    drop(client_tx);
    let _ = server_task.await;
}

#[tokio::test]
async fn category_filter_rejects_other_categories() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, mut client_rx) = duplex(64 * 1024);
    let writer = Arc::new(Mutex::new(server_tx));

    let server_task = {
        let ctx = ctx.clone();
        let writer = writer.clone();
        tokio::spawn(async move {
            let _ = handle_client_generic(server_rx, writer, ctx).await;
        })
    };

    // Transfer カテゴリだけ購読
    IpcTransport::write_message(
        &mut client_tx,
        &IpcCommand::Subscribe {
            events: vec![EventFilter::Category(EventCategory::Transfer)],
        },
    )
    .await
    .unwrap();
    let _ = IpcTransport::read_message::<_, ServerMessage>(&mut client_rx)
        .await
        .unwrap();

    // Peer イベントを投げる → フィルタで弾かれる
    ctx.event_bus.emit(PeerConnectedEvent {
        project_id: "p".into(),
        peer_id: "px".into(),
        display_name: "x".into(),
        route: "Direct".into(),
        rtt_ms: 10,
    });
    // Transfer イベントを投げる → 届く
    ctx.event_bus.emit(TransferProgressEvent {
        transfer_id: "t".into(),
        file_name: "f".into(),
        bytes_transferred: 10,
        total_bytes: 100,
        speed_bps: 1,
    });

    let msg = tokio::time::timeout(
        Duration::from_secs(2),
        IpcTransport::read_message::<_, ServerMessage>(&mut client_rx),
    )
    .await
    .expect("event must arrive")
    .unwrap();
    match msg {
        ServerMessage::Event(IpcEvent::TransferProgress { transfer_id, .. }) => {
            assert_eq!(transfer_id, "t");
        }
        other => panic!("expected TransferProgress, got {other:?}"),
    }

    drop(client_tx);
    let _ = server_task.await;
}
