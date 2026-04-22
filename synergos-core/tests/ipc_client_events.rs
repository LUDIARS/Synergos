//! PR-10: IpcClient が Response と Event を duplex 上で正しく多重分離できるか。
//! 実プラットフォームソケットではなく duplex ペアで代替する (実ソケットは
//! platform 依存 + CI で flaky)。

use std::sync::Arc;
use std::time::Duration;

use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{CoreEventBus, SharedEventBus, TransferProgressEvent};
use synergos_core::exchange::Exchange;
use synergos_core::ipc_server::{handle_client_generic, ServiceContext};
use synergos_core::presence::PresenceService;
use synergos_core::project::ProjectManager;
use synergos_ipc::command::IpcCommand;
use synergos_ipc::event::IpcEvent;
use synergos_ipc::response::IpcResponse;
use synergos_ipc::transport::{IpcTransport, ServerMessage};
use tokio::io::duplex;
use tokio::sync::{broadcast, Mutex};

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
    })
}

/// Response と Event が 1 本の duplex 上で多重化されても、Response 待ちと
/// Event 受信が混線しないことを確認する。
#[tokio::test]
async fn response_and_event_multiplex_cleanly_on_single_stream() {
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

    // Subscribe
    IpcTransport::write_message(&mut client_tx, &IpcCommand::Subscribe { events: vec![] })
        .await
        .unwrap();
    let msg: ServerMessage = IpcTransport::read_message(&mut client_rx).await.unwrap();
    matches!(msg, ServerMessage::Response(IpcResponse::Subscribed { .. }));

    // Ping を送り、Subscribed の後に Response(Pong) が割り込んでも順序が保たれる
    IpcTransport::write_message(&mut client_tx, &IpcCommand::Ping)
        .await
        .unwrap();

    // 先に Event を流しておく
    ctx.event_bus.emit(TransferProgressEvent {
        transfer_id: "a".into(),
        file_name: "f".into(),
        bytes_transferred: 1,
        total_bytes: 10,
        speed_bps: 1,
    });

    // 2 つ受け取る (順不定 — Response/Event を Vec に集めて両方が揃ったか確認)
    let mut saw_response = false;
    let mut saw_event = false;
    for _ in 0..2 {
        let m = tokio::time::timeout(
            Duration::from_secs(2),
            IpcTransport::read_message::<_, ServerMessage>(&mut client_rx),
        )
        .await
        .expect("msg arrives")
        .unwrap();
        match m {
            ServerMessage::Response(IpcResponse::Pong) => saw_response = true,
            ServerMessage::Event(IpcEvent::TransferProgress { .. }) => saw_event = true,
            other => panic!("unexpected message: {other:?}"),
        }
    }
    assert!(saw_response && saw_event);

    drop(client_tx);
    let _ = server_task.await;
}

#[tokio::test]
async fn is_daemon_running_false_when_no_daemon() {
    // サーバーを立てていない状態で is_daemon_running は false を返す。
    let running = synergos_ipc::IpcClient::is_daemon_running().await;
    assert!(!running, "no daemon bound — must return false");
}
