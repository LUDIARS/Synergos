//! `project checkout` の突合ロジック (docs/versioning-design.md §3.4)。
//! ネットワーク無し: FileWant を出す判断と、既に一致するファイルのスキップ、
//! `--manifest` 指定時の manifest 差し替えを確認する。

use std::path::PathBuf;
use std::sync::Arc;

use synergos_core::checkout::{checkout_project, CheckoutContext};
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{Exchange, FileSharing};
use synergos_core::history::HistoryStore;
use synergos_core::manifest::{BumpOutcome, ProjectManifest};
use synergos_core::project::{ProjectConfiguration, ProjectManager};

const PROJECT: &str = "checkout-test";

async fn setup() -> (
    PathBuf,
    Arc<ProjectManager>,
    Arc<Exchange>,
    Arc<HistoryStore>,
) {
    let root = std::env::temp_dir().join(format!("syn-checkout-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(root.join("sub")).await.unwrap();
    let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let pm = Arc::new(ProjectManager::new(event_bus.clone()));
    pm.open_project(PROJECT.into(), root.clone(), None)
        .await
        .unwrap();
    let ex = Arc::new(Exchange::new(event_bus));
    let history = Arc::new(HistoryStore::new(Default::default()));
    (root, pm, ex, history)
}

#[tokio::test]
async fn checkout_requests_only_files_that_differ_from_manifest() {
    let (root, pm, ex, history) = setup().await;
    // 作業ツリー: a.bin = "two", sub/b.bin = "same"
    tokio::fs::write(root.join("a.bin"), b"two").await.unwrap();
    tokio::fs::write(root.join("sub").join("b.bin"), b"same")
        .await
        .unwrap();
    // 手元台帳には c.bin (対象 manifest に無い) が載っている
    pm.bump_file_version(PROJECT, "c.bin", 1, 1, "me")
        .await
        .unwrap();
    // git checkout 後を模す: ディスク上の manifest が a.bin=v1("one") / sub/b.bin=v4("same")
    let mut m = ProjectManifest::new(PROJECT);
    m.bump("a.bin", 3, crc32fast::hash(b"one"), "peer", 1);
    m.bump("sub/b.bin", 4, crc32fast::hash(b"same"), "peer", 1);
    m.files.get_mut("sub/b.bin").unwrap().version = 4;
    m.save(&root).await.unwrap();

    let ctx = CheckoutContext {
        projects: &pm,
        exchange: &ex,
        history: &history,
    };
    let report = checkout_project(&ctx, PROJECT, None).await.unwrap();
    assert_eq!(report.requested, vec![("a.bin".to_string(), 1)]);
    assert_eq!(report.up_to_date, 1);
    assert_eq!(report.extra, vec!["c.bin".to_string()]);
    // 一致していたファイルは台帳がその版に揃う
    let fid = synergos_net::types::FileId::new("sub/b.bin");
    assert!(ex.has_shared_file(PROJECT, &fid, 4));
    // 要求中の版は pin されている (受信時に手元より古くても受け入れる)
    // pin は private なので、fetch が transfer として登録されたことで確認する
    let transfers = ex.list_transfers(Some(PROJECT)).await;
    assert!(transfers
        .iter()
        .any(|t| t.file_id.0 == "a.bin" && t.version == 1));
    // in-memory manifest はディスクの内容に置き換わっている (c.bin は消える)
    assert!(pm
        .manifest_entries(PROJECT)
        .iter()
        .all(|(rel, _)| rel != "c.bin"));
    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn checkout_with_explicit_manifest_replaces_project_manifest() {
    let (root, pm, ex, history) = setup().await;
    tokio::fs::write(root.join("a.bin"), b"one").await.unwrap();
    // 現在の manifest は a.bin=v5
    let mut current = ProjectManifest::new(PROJECT);
    current.bump("a.bin", 3, crc32fast::hash(b"one"), "peer", 1);
    current.files.get_mut("a.bin").unwrap().version = 5;
    current.save(&root).await.unwrap();
    // 別コミットから取り出した manifest: a.bin=v2 (内容は同じ "one")
    let mut old = ProjectManifest::new(PROJECT);
    old.bump("a.bin", 3, crc32fast::hash(b"one"), "peer", 1);
    old.files.get_mut("a.bin").unwrap().version = 2;
    let old_path = root.join("old-manifest.json");
    tokio::fs::write(&old_path, serde_json::to_vec(&old).unwrap())
        .await
        .unwrap();

    let ctx = CheckoutContext {
        projects: &pm,
        exchange: &ex,
        history: &history,
    };
    let report = checkout_project(&ctx, PROJECT, Some(&old_path))
        .await
        .unwrap();
    assert!(report.requested.is_empty());
    assert_eq!(report.up_to_date, 1);
    // プロジェクトの manifest.json が指定 manifest に置き換わる
    let on_disk = ProjectManifest::load(&root, PROJECT).await.unwrap();
    assert_eq!(on_disk.get("a.bin").unwrap().version, 2);

    // project_id が違う manifest は拒否
    let mut other = ProjectManifest::new("someone-else");
    other.bump("a.bin", 3, 1, "peer", 1);
    let other_path = root.join("other.json");
    tokio::fs::write(&other_path, serde_json::to_vec(&other).unwrap())
        .await
        .unwrap();
    assert!(checkout_project(&ctx, PROJECT, Some(&other_path))
        .await
        .is_err());
    let _ = tokio::fs::remove_dir_all(&root).await;
}

#[tokio::test]
async fn version_high_water_survives_manifest_rollback_and_restart() {
    let root = std::env::temp_dir().join(format!("syn-version-state-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&root).await.unwrap();
    let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let first = Arc::new(ProjectManager::new(event_bus.clone()));
    first
        .open_project(PROJECT.into(), root.clone(), None)
        .await
        .unwrap();
    first
        .note_confirmed_version(PROJECT, "a.bin", 5)
        .await
        .unwrap();
    drop(first);

    // git checkout で manifest が v1 へ戻った状態を再現する。
    let mut old = ProjectManifest::new(PROJECT);
    old.bump("a.bin", 1, 1, "peer", 1);
    old.save(&root).await.unwrap();

    let restarted = Arc::new(ProjectManager::new(event_bus));
    restarted
        .open_project(PROJECT.into(), root.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        restarted
            .bump_file_version(PROJECT, "a.bin", 2, 2, "me")
            .await
            .unwrap(),
        BumpOutcome::Bumped(6)
    );
    let _ = tokio::fs::remove_dir_all(&root).await;
}
