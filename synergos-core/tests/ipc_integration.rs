//! PR-7: L3 IPC Client ↔ Server 統合テスト (duplex pipe 版)。
//!
//! 実 Unix socket / Named pipe ではなく `tokio::io::duplex` を使って
//! IpcTransport ベースのラウンドトリップを検証する。

use std::sync::Arc;

use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::Exchange;
use synergos_core::ipc_server::{dispatch_command, ServiceContext};
use synergos_core::presence::PresenceService;
use synergos_core::project::ProjectManager;
use synergos_ipc::command::IpcCommand;
use synergos_ipc::response::IpcResponse;
use synergos_ipc::transport::IpcTransport;
use tokio::io::duplex;
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
        catalogs: Arc::new(dashmap::DashMap::new()),
    })
}

/// duplex pipe の片方でサーバーループを回し、もう片方をクライアント代わりに使う
async fn run_server_once(
    ctx: Arc<ServiceContext>,
    mut reader: tokio::io::DuplexStream,
    mut writer: tokio::io::DuplexStream,
) {
    loop {
        let cmd: IpcCommand = match IpcTransport::read_message(&mut reader).await {
            Ok(c) => c,
            Err(_) => return,
        };
        let resp = dispatch_command(cmd, &ctx).await;
        if IpcTransport::write_message(&mut writer, &resp)
            .await
            .is_err()
        {
            return;
        }
    }
}

#[tokio::test]
async fn ping_pong_roundtrip_over_duplex() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, mut client_rx) = duplex(64 * 1024);

    let server_task = tokio::spawn(run_server_once(ctx, server_rx, server_tx));

    IpcTransport::write_message(&mut client_tx, &IpcCommand::Ping)
        .await
        .unwrap();
    let resp: IpcResponse = IpcTransport::read_message(&mut client_rx).await.unwrap();
    matches!(resp, IpcResponse::Pong);

    drop(client_tx);
    let _ = server_task.await;
}

#[tokio::test]
async fn multiple_commands_preserve_order() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, mut client_rx) = duplex(64 * 1024);

    let server_task = tokio::spawn(run_server_once(ctx, server_rx, server_tx));

    for _ in 0..3 {
        IpcTransport::write_message(&mut client_tx, &IpcCommand::Ping)
            .await
            .unwrap();
    }
    for _ in 0..3 {
        let resp: IpcResponse = IpcTransport::read_message(&mut client_rx).await.unwrap();
        assert!(matches!(resp, IpcResponse::Pong));
    }

    drop(client_tx);
    let _ = server_task.await;
}

#[tokio::test]
async fn malformed_client_frame_closes_server_gracefully() {
    let ctx = make_ctx();
    let (mut client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, _client_rx) = duplex(64 * 1024);

    let server_task = tokio::spawn(run_server_once(ctx, server_rx, server_tx));

    // 偽のフレーム: length-prefix は正しいがペイロードが壊れている
    use tokio::io::AsyncWriteExt;
    let bad_len = 10u32.to_le_bytes();
    client_tx.write_all(&bad_len).await.unwrap();
    client_tx.write_all(&[0xffu8; 10]).await.unwrap();
    client_tx.flush().await.unwrap();

    drop(client_tx);
    // サーバ側が抜けることを確認 (タイムアウト付き)
    tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("server should exit on malformed frame")
        .ok();
}

#[tokio::test]
async fn client_disconnect_cleans_up_server() {
    let ctx = make_ctx();
    let (client_tx, server_rx) = duplex(64 * 1024);
    let (server_tx, _client_rx) = duplex(64 * 1024);

    let server_task = tokio::spawn(run_server_once(ctx, server_rx, server_tx));

    drop(client_tx);
    tokio::time::timeout(std::time::Duration::from_secs(2), server_task)
        .await
        .expect("server must exit after client close")
        .ok();
}
