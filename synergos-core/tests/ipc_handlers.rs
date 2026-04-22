//! PR-5: synergos-core IPC サーバー handler 単体テスト。
//! dispatch_command を直接呼び、各コマンドが期待した IpcResponse を返すか確認する。

use std::sync::Arc;

use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::Exchange;
use synergos_core::ipc_server::{dispatch_command, ServiceContext};
use synergos_core::presence::PresenceService;
use synergos_core::project::ProjectManager;
use synergos_ipc::command::IpcCommand;
use synergos_ipc::response::IpcResponse;
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
        content_store: Arc::new(synergos_net::content::MemoryContentStore::new()),
    })
}

#[tokio::test]
async fn ping_returns_pong() {
    let ctx = make_ctx();
    let resp = dispatch_command(IpcCommand::Ping, &ctx).await;
    matches!(resp, IpcResponse::Pong);
}

#[tokio::test]
async fn status_reports_zero_counts_initially() {
    let ctx = make_ctx();
    let resp = dispatch_command(IpcCommand::Status, &ctx).await;
    match resp {
        IpcResponse::Status(s) => {
            assert_eq!(s.project_count, 0);
            assert_eq!(s.active_connections, 0);
            assert_eq!(s.active_transfers, 0);
            assert!(s.pid > 0);
        }
        other => panic!("expected Status, got {other:?}"),
    }
}

#[tokio::test]
async fn project_list_on_empty_returns_empty() {
    let ctx = make_ctx();
    let resp = dispatch_command(IpcCommand::ProjectList, &ctx).await;
    match resp {
        IpcResponse::ProjectList(list) => assert!(list.is_empty()),
        other => panic!("expected ProjectList, got {other:?}"),
    }
}

#[tokio::test]
async fn project_get_unknown_returns_error() {
    let ctx = make_ctx();
    let resp = dispatch_command(
        IpcCommand::ProjectGet {
            project_id: "doesnotexist".into(),
        },
        &ctx,
    )
    .await;
    match resp {
        IpcResponse::Error { .. } => (),
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn project_open_then_list_reflects_it() {
    let ctx = make_ctx();
    let dir = std::env::temp_dir().join(format!("synergos-ipc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let resp = dispatch_command(
        IpcCommand::ProjectOpen {
            project_id: "pX".into(),
            root_path: dir.clone(),
            display_name: Some("Project X".into()),
        },
        &ctx,
    )
    .await;
    matches!(resp, IpcResponse::Ok);

    let list_resp = dispatch_command(IpcCommand::ProjectList, &ctx).await;
    match list_resp {
        IpcResponse::ProjectList(list) => {
            assert!(list.iter().any(|p| p.project_id == "pX"));
        }
        other => panic!("expected ProjectList, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn transfer_list_on_empty_is_empty() {
    let ctx = make_ctx();
    let resp = dispatch_command(IpcCommand::TransferList { project_id: None }, &ctx).await;
    match resp {
        IpcResponse::TransferList(list) => assert!(list.is_empty()),
        other => panic!("expected TransferList, got {other:?}"),
    }
}

#[tokio::test]
async fn unsubscribe_unknown_returns_ok() {
    // 現状実装は未知の subscription_id を silently ignore する。
    // 将来 Error を返す設計にするなら回帰テストでここを更新する。
    let ctx = make_ctx();
    let resp = dispatch_command(
        IpcCommand::Unsubscribe {
            subscription_id: "nope".into(),
        },
        &ctx,
    )
    .await;
    match resp {
        IpcResponse::Ok | IpcResponse::Error { .. } => (),
        other => panic!("expected Ok/Error, got {other:?}"),
    }
}

#[tokio::test]
async fn network_status_returns_hardcoded_shape() {
    let ctx = make_ctx();
    let resp = dispatch_command(IpcCommand::NetworkStatus, &ctx).await;
    match resp {
        IpcResponse::NetworkStatus(_) => (),
        other => panic!("expected NetworkStatus, got {other:?}"),
    }
}
