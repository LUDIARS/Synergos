//! ProjectManager の状態永続化テスト。Daemon 再起動に相当するフローで
//! プロジェクトがディスクから復元できることを確認する (Issue #6 §2.5)。

use std::sync::Arc;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::project::{ProjectConfiguration, ProjectManager, ProjectSettingsPatch};

fn bus() -> SharedEventBus {
    Arc::new(CoreEventBus::new())
}

#[tokio::test]
async fn open_close_persist_and_restore() {
    let tmp = std::env::temp_dir();
    let state = tmp.join(format!("syn-state-{}.json", uuid::Uuid::new_v4()));
    let root1 = tmp.join(format!("syn-root-{}", uuid::Uuid::new_v4()));
    let root2 = tmp.join(format!("syn-root-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root1).unwrap();
    std::fs::create_dir_all(&root2).unwrap();

    // PM1: 2 件 open して close 1 件
    {
        let pm = ProjectManager::with_state_path(bus(), None, state.clone());
        pm.open_project("alpha".into(), root1.clone(), Some("Alpha".into()))
            .await
            .unwrap();
        pm.open_project("beta".into(), root2.clone(), Some("Beta".into()))
            .await
            .unwrap();
        pm.close_project("alpha").await.unwrap();
    }

    // PM2: 同じ state_path で立て直し
    let pm2 = ProjectManager::with_state_path(bus(), None, state.clone());
    let persisted = pm2.load_state().await.unwrap();
    assert_eq!(persisted.len(), 1, "alpha is closed; beta should remain");
    assert_eq!(persisted[0].project_id, "beta");

    // ProjectManager は open し直さないと list に出ないので、手動で再 open
    pm2.open_project(
        persisted[0].project_id.clone(),
        persisted[0].root_path.clone(),
        Some(persisted[0].display_name.clone()),
    )
    .await
    .unwrap();

    let list = pm2.list_projects();
    assert!(list.iter().any(|p| p.project_id == "beta"));

    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_dir_all(&root1);
    let _ = std::fs::remove_dir_all(&root2);
}

#[tokio::test]
async fn update_project_persists_changes() {
    let tmp = std::env::temp_dir();
    let state = tmp.join(format!("syn-state-upd-{}.json", uuid::Uuid::new_v4()));
    let root = tmp.join(format!("syn-root-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();

    {
        let pm = ProjectManager::with_state_path(bus(), None, state.clone());
        pm.open_project("proj".into(), root.clone(), None)
            .await
            .unwrap();
        pm.update_project(
            "proj",
            ProjectSettingsPatch {
                display_name: Some("Renamed".into()),
                description: None,
                sync_mode: None,
                max_peers: Some(5),
            },
        )
        .await
        .unwrap();
    }

    let pm2 = ProjectManager::with_state_path(bus(), None, state.clone());
    let persisted = pm2.load_state().await.unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].display_name, "Renamed");
    assert_eq!(persisted[0].max_peers, 5);

    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn no_state_path_means_no_persistence() {
    // with_gossip (state_path = None) はインメモリのみ。
    // save_state / load_state は no-op で Ok を返す。
    let pm = ProjectManager::new(bus());
    let persisted = pm.load_state().await.unwrap();
    assert!(persisted.is_empty());
    pm.save_state().await.unwrap();
}
