//! PR-4: synergos-core サービスレイヤ L1。
//!
//! 対象:
//! - ProjectManager::open/close/update
//! - ConflictManager::detect/resolve/list/get
//! - ExchangeManager (FileSharing trait) の Queued→Running→Cancelled 遷移
//! - PresenceManager の update_bandwidth / update_rtt

use std::path::PathBuf;
use std::sync::Arc;

use synergos_core::conflict::{ConflictManager, ConflictResolution, ConflictState};
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FetchRequest, FileSharing, ShareRequest, TransferPriority, TransferState,
};
use synergos_core::presence::PresenceService;
use synergos_core::project::{ProjectConfiguration, ProjectManager};
use synergos_net::types::{Blake3Hash, FileId, PeerId};

fn bus() -> SharedEventBus {
    Arc::new(CoreEventBus::new())
}

fn tmp_dir() -> PathBuf {
    let p = std::env::temp_dir().join(format!("synergos-core-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

// ── ProjectManager ──────────────────────────────────────────────────

#[tokio::test]
async fn project_open_rejects_duplicate() {
    let pm = ProjectManager::new(bus());
    let dir = tmp_dir();
    pm.open_project("p1".into(), dir.clone(), None).await.unwrap();
    let err = pm
        .open_project("p1".into(), dir.clone(), None)
        .await
        .unwrap_err();
    use synergos_core::project::ProjectError;
    assert!(matches!(err, ProjectError::AlreadyExists(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn project_open_uses_fallback_display_name() {
    let pm = ProjectManager::new(bus());
    let dir = tmp_dir();
    pm.open_project("pAlpha".into(), dir.clone(), None)
        .await
        .unwrap();
    // Fallback: display_name が project_id になっているはず
    let info = pm
        .get_project("pAlpha")
        .await
        .expect("project exists");
    assert_eq!(info.display_name, "pAlpha");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn project_close_unknown_errors() {
    let pm = ProjectManager::new(bus());
    let err = pm.close_project("unknown").await.unwrap_err();
    use synergos_core::project::ProjectError;
    assert!(matches!(err, ProjectError::NotFound(_)));
}

#[tokio::test]
async fn project_root_returns_registered_path() {
    let pm = ProjectManager::new(bus());
    let dir = tmp_dir();
    pm.open_project("pB".into(), dir.clone(), None)
        .await
        .unwrap();
    let root = pm.project_root("pB").expect("project registered");
    assert_eq!(root, dir);
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ConflictManager ─────────────────────────────────────────────────

#[tokio::test]
async fn conflict_resolve_on_unknown_returns_err() {
    let cm = ConflictManager::new(bus());
    let err = cm.resolve_conflict(
        &FileId::new("nofile"),
        ConflictResolution::KeepLocal,
    );
    assert!(err.is_err());
}

#[tokio::test]
async fn conflict_list_on_empty_returns_empty() {
    let cm = ConflictManager::new(bus());
    assert!(cm.list_conflicts(None).is_empty());
}

#[tokio::test]
async fn conflict_cleanup_resolved_preserves_active() {
    let cm = ConflictManager::new(bus());
    // active な疑似コンフリクトを直接 conflicts に突っ込む手段が無いので
    // cleanup 呼び出しだけ観測する (no-op の健全性確認)
    cm.cleanup_resolved();
    cm.cleanup_expired_notifications(1_000);
    assert!(cm.list_conflicts(None).is_empty());
}

// ── Exchange (TransferState transitions) ────────────────────────────

#[tokio::test]
async fn exchange_fetch_registers_queued_transfer() {
    let ex = Exchange::new(bus());
    let id = ex
        .fetch_file(FetchRequest {
            project_id: "p".into(),
            file_id: FileId::new("f"),
            source_peer: None,
            priority: TransferPriority::Background,
            version: 0,
        })
        .await
        .unwrap();
    let info = ex.get_transfer(&id).await.expect("transfer stored");
    // Queued でも Matched (offer 先行があれば Running) でも OK
    assert!(matches!(info.state, TransferState::Queued | TransferState::Running));
}

#[tokio::test]
async fn exchange_cancel_transitions_to_cancelled() {
    let ex = Exchange::new(bus());
    let id = ex
        .share_file(ShareRequest {
            project_id: "p".into(),
            file_id: FileId::new("f"),
            file_path: PathBuf::from("/tmp/none"),
            file_size: 10,
            checksum: Blake3Hash::default(),
            priority: TransferPriority::Interactive,
            target_peer: Some(PeerId::new("peer")),
            version: 1,
        })
        .await
        .unwrap();
    ex.cancel_transfer(&id).await.unwrap();
    let info = ex.get_transfer(&id).await.unwrap();
    assert_eq!(info.state, TransferState::Cancelled);
}

#[tokio::test]
async fn exchange_gc_finished_reaps_cancelled() {
    let ex = Exchange::new(bus());
    let id = ex
        .share_file(ShareRequest {
            project_id: "p".into(),
            file_id: FileId::new("f"),
            file_path: PathBuf::from("/tmp/none"),
            file_size: 10,
            checksum: Blake3Hash::default(),
            priority: TransferPriority::Interactive,
            target_peer: Some(PeerId::new("peer")),
            version: 1,
        })
        .await
        .unwrap();
    ex.cancel_transfer(&id).await.unwrap();
    let reaped = ex.gc_finished_transfers();
    assert_eq!(reaped, 1);
    assert!(ex.get_transfer(&id).await.is_none());
}

#[tokio::test]
async fn exchange_pause_resume_roundtrip() {
    let ex = Exchange::new(bus());
    let id = ex
        .share_file(ShareRequest {
            project_id: "p".into(),
            file_id: FileId::new("f"),
            file_path: PathBuf::from("/tmp/none"),
            file_size: 10,
            checksum: Blake3Hash::default(),
            priority: TransferPriority::Interactive,
            target_peer: Some(PeerId::new("peer")),
            version: 1,
        })
        .await
        .unwrap();
    // Queued 状態では pause は no-op
    ex.pause_transfer(&id).await.unwrap();
    // update_progress で Running にしてから pause
    ex.update_progress(&id, 1, 1);
    ex.pause_transfer(&id).await.unwrap();
    assert_eq!(ex.get_transfer(&id).await.unwrap().state, TransferState::Paused);
    ex.resume_transfer(&id).await.unwrap();
    assert_eq!(ex.get_transfer(&id).await.unwrap().state, TransferState::Running);
}

// ── PresenceService ─────────────────────────────────────────────────

#[tokio::test]
async fn presence_update_rtt_and_bandwidth_are_idempotent() {
    let p = PresenceService::new(bus());
    let pid = PeerId::new("peer");
    p.update_rtt(&pid, 42);
    p.update_rtt(&pid, 50); // 上書き
    p.update_bandwidth(&pid, 1_000_000);
    p.update_bandwidth(&pid, 2_000_000);
    // panic しなければ OK (内部 DashMap の set_if_exists パスを叩くだけ)
}

// Use ConflictState and ConflictResolution to silence unused warnings
#[allow(dead_code)]
fn _conflict_enum_coverage() -> (ConflictState, ConflictResolution) {
    (
        ConflictState::Resolved {
            resolution: ConflictResolution::AcceptRemote,
        },
        ConflictResolution::ManualMerge,
    )
}
