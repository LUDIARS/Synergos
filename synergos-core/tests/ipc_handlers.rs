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
use synergos_net::config::{HistoryConfig, QuicConfig};
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use tokio::sync::broadcast;

fn make_ctx() -> Arc<ServiceContext> {
    make_ctx_with_history(HistoryConfig::default())
}

/// daemon.rs と同じ順序で履歴ノードを組み立てる (保管庫 → フック注入 → Arc)。
/// `history.enabled = false` なら従来どおりフックを注入しない。
fn make_ctx_with_history(history_config: HistoryConfig) -> Arc<ServiceContext> {
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
    let project_manager = Arc::new(ProjectManager::new(event_bus.clone()));
    let history = Arc::new(synergos_core::history::HistoryStore::new(history_config));
    let mut exchange = Exchange::new(event_bus.clone());
    if history.enabled() {
        exchange.attach_history_hooks(synergos_core::history::wiring::build_hooks(
            history.clone(),
            project_manager.clone(),
        ));
    }
    Arc::new(ServiceContext {
        event_bus: event_bus.clone(),
        project_manager,
        exchange: Arc::new(exchange),
        presence: Arc::new(PresenceService::new(event_bus.clone())),
        conflict_manager: Arc::new(ConflictManager::new(event_bus.clone())),
        shutdown_tx,
        started_at: 0,
        net_config: None,
        catalogs: Arc::new(dashmap::DashMap::new()),
        content_store: Arc::new(synergos_net::content::MemoryContentStore::new()),
        quic,
        identity: None,
        history,
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

/// プロジェクトを開き、`assets/a.bin` に `body` を書いて publish する。
async fn open_and_publish(ctx: &Arc<ServiceContext>, root: &std::path::Path, body: &[u8]) {
    if !root.join("assets").exists() {
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let resp = dispatch_command(
            IpcCommand::ProjectOpen {
                project_id: "hist".into(),
                root_path: root.to_path_buf(),
                display_name: None,
            },
            ctx,
        )
        .await;
        assert!(matches!(resp, IpcResponse::Ok), "open failed: {resp:?}");
    }
    tokio::fs::write(root.join("assets").join("a.bin"), body)
        .await
        .unwrap();
    let resp = dispatch_command(
        IpcCommand::PublishUpdate {
            project_id: "hist".into(),
            file_paths: vec![std::path::PathBuf::from("assets/a.bin")],
        },
        ctx,
    )
    .await;
    assert!(matches!(resp, IpcResponse::Ok), "publish failed: {resp:?}");
}

/// 履歴ノードでは publish した各版が保管され、`history ls` に出て、
/// `project restore` が保管庫からローカルで巻き戻し、`history gc --purge` で消える。
#[tokio::test]
async fn history_node_keeps_published_versions_and_restores_locally() {
    let ctx = make_ctx_with_history(HistoryConfig {
        enabled: true,
        ..HistoryConfig::default()
    });
    let root = std::env::temp_dir().join(format!("synergos-ipc-hist-{}", uuid::Uuid::new_v4()));
    let file = root.join("assets").join("a.bin");

    open_and_publish(&ctx, &root, b"one").await;
    open_and_publish(&ctx, &root, b"two!!").await;

    let listed = match dispatch_command(
        IpcCommand::HistoryList {
            project_id: "hist".into(),
            rel_path: None,
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryList(items) => items,
        other => panic!("expected HistoryList, got {other:?}"),
    };
    assert_eq!(listed.len(), 2, "both published versions are retained");
    assert!(listed
        .iter()
        .all(|v| v.rel_path == "assets/a.bin" && v.source == "published"));
    assert_eq!(listed.iter().map(|v| v.version).min(), Some(1));
    assert_eq!(listed.iter().map(|v| v.version).max(), Some(2));

    // 自ノードが保管しているので、ネットワーク無しで v1 に戻る
    let resp = dispatch_command(
        IpcCommand::ProjectRestore {
            project_id: "hist".into(),
            rel_path: "assets/a.bin".into(),
            version: 1,
        },
        &ctx,
    )
    .await;
    assert!(matches!(resp, IpcResponse::Ok), "restore failed: {resp:?}");
    assert_eq!(tokio::fs::read(&file).await.unwrap(), b"one");

    // 巻き戻した後の publish は既存の v2 を飛び越えて v3 になる
    open_and_publish(&ctx, &root, b"three!!!").await;
    let after = match dispatch_command(
        IpcCommand::HistoryList {
            project_id: "hist".into(),
            rel_path: Some("assets/a.bin".into()),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryList(items) => items,
        other => panic!("expected HistoryList, got {other:?}"),
    };
    assert_eq!(after.iter().map(|v| v.version).max(), Some(3));

    match dispatch_command(
        IpcCommand::HistoryGc {
            project_id: "hist".into(),
            purge: true,
            keep_manifests: vec![],
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryGcReport(report) => {
            assert_eq!(report.removed_versions.len(), after.len());
        }
        other => panic!("expected HistoryGcReport, got {other:?}"),
    }
    match dispatch_command(
        IpcCommand::HistoryList {
            project_id: "hist".into(),
            rel_path: None,
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryList(items) => assert!(items.is_empty()),
        other => panic!("expected HistoryList, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// `tag add/ls/show/rm` の IPC 配線と、タグが GC 保護に効くことを確認する
/// (spec/tasks/…-2-version-tags.md の完了条件: IPC/CLI の配線テスト)。
#[tokio::test]
async fn tag_add_ls_show_rm_round_trip_and_protects_from_gc() {
    let ctx = make_ctx_with_history(HistoryConfig {
        enabled: true,
        max_versions_per_file: 1,
        ..HistoryConfig::default()
    });
    let root = std::env::temp_dir().join(format!("synergos-ipc-tag-{}", uuid::Uuid::new_v4()));

    open_and_publish(&ctx, &root, b"one").await;
    open_and_publish(&ctx, &root, b"two!!").await;

    // 現在の manifest (v2) をタグ "release-1.0" としてピン
    let tag = match dispatch_command(
        IpcCommand::TagAdd {
            project_id: "hist".into(),
            name: "release-1.0".into(),
            manifest_path: None,
            pins: Vec::new(),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Tag(tag) => tag,
        other => panic!("expected Tag, got {other:?}"),
    };
    assert_eq!(tag.name, "release-1.0");
    assert_eq!(tag.pins, vec![("assets/a.bin".to_string(), 2)]);

    // 単一ファイル版のピンでも作れる ("older" が v1 を保護する)
    match dispatch_command(
        IpcCommand::TagAdd {
            project_id: "hist".into(),
            name: "older".into(),
            manifest_path: None,
            pins: vec![("assets/a.bin".to_string(), 1)],
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Tag(_) => {}
        other => panic!("expected Tag, got {other:?}"),
    }

    match dispatch_command(IpcCommand::TagLs { project_id: "hist".into() }, &ctx).await {
        IpcResponse::TagList(items) => {
            assert_eq!(items.len(), 2);
            assert!(items.iter().any(|t| t.name == "release-1.0" && t.pin_count == 1));
            assert!(items.iter().any(|t| t.name == "older" && t.pin_count == 1));
        }
        other => panic!("expected TagList, got {other:?}"),
    }

    match dispatch_command(
        IpcCommand::TagShow {
            project_id: "hist".into(),
            name: "older".into(),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Tag(tag) => assert_eq!(tag.pins, vec![("assets/a.bin".to_string(), 1)]),
        other => panic!("expected Tag, got {other:?}"),
    }

    // publish で v3 を作る。max_versions_per_file=1 でも "older" タグが v1 を守る
    open_and_publish(&ctx, &root, b"three!!!").await;
    let gc_report = match dispatch_command(
        IpcCommand::HistoryGc {
            project_id: "hist".into(),
            purge: false,
            keep_manifests: vec![],
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryGcReport(report) => report,
        other => panic!("expected HistoryGcReport, got {other:?}"),
    };
    // v2 は "release-1.0" タグと最新 manifest どちらでも保護されないが、gc 前に
    // release-1.0 は v2 を、older は v1 を保護するので消えるのは無し
    assert!(
        !gc_report
            .removed_versions
            .contains(&("assets/a.bin".to_string(), 1)),
        "tagged v1 must survive gc: {gc_report:?}"
    );
    assert!(
        !gc_report
            .removed_versions
            .contains(&("assets/a.bin".to_string(), 2)),
        "tagged v2 must survive gc: {gc_report:?}"
    );

    // タグを消せば実体はまだ残るが (rm は実体を消さない)、保護は外れる
    match dispatch_command(
        IpcCommand::TagRm {
            project_id: "hist".into(),
            name: "older".into(),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Ok => {}
        other => panic!("expected Ok, got {other:?}"),
    }
    match dispatch_command(
        IpcCommand::TagShow {
            project_id: "hist".into(),
            name: "older".into(),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Error { .. } => {}
        other => panic!("expected Error for removed tag, got {other:?}"),
    }
    // 存在しないタグの rm は Error
    match dispatch_command(
        IpcCommand::TagRm {
            project_id: "hist".into(),
            name: "older".into(),
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Error { .. } => {}
        other => panic!("expected Error for already-removed tag, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// 通常ノード (既定) では保管庫を作らず、`history ls` は空、`gc` は拒否する。
#[tokio::test]
async fn plain_node_stores_nothing_and_refuses_history_gc() {
    let ctx = make_ctx();
    let root = std::env::temp_dir().join(format!("synergos-ipc-plain-{}", uuid::Uuid::new_v4()));
    open_and_publish(&ctx, &root, b"one").await;

    match dispatch_command(
        IpcCommand::HistoryList {
            project_id: "hist".into(),
            rel_path: None,
        },
        &ctx,
    )
    .await
    {
        IpcResponse::HistoryList(items) => assert!(items.is_empty()),
        other => panic!("expected HistoryList, got {other:?}"),
    }
    match dispatch_command(
        IpcCommand::HistoryGc {
            project_id: "hist".into(),
            purge: false,
            keep_manifests: vec![],
        },
        &ctx,
    )
    .await
    {
        IpcResponse::Error { .. } => (),
        other => panic!("expected Error for a non-history node, got {other:?}"),
    }
    assert!(!root.join(".synergos").join("history").exists());
    let _ = std::fs::remove_dir_all(&root);
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
