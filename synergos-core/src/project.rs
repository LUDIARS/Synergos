//! プロジェクト管理
//!
//! Core デーモンが管理するプロジェクトのライフサイクルを制御する。

use dashmap::DashMap;
use std::path::PathBuf;

use synergos_ipc::response::ProjectInfo;

use crate::event_bus::SharedEventBus;

/// プロジェクトの内部状態
struct ManagedProject {
    project_id: String,
    root_path: PathBuf,
    // TODO: synergos-net インスタンス
    // TODO: Exchange, Presence, Conflict マネージャ
}

/// プロジェクトマネージャ
///
/// 複数プロジェクトの同時管理をサポートする。
pub struct ProjectManager {
    projects: DashMap<String, ManagedProject>,
    _event_bus: SharedEventBus,
}

impl ProjectManager {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            projects: DashMap::new(),
            _event_bus: event_bus,
        }
    }

    /// プロジェクトを開く
    pub async fn open(&self, project_id: String, root_path: PathBuf) {
        tracing::info!(
            "Opening project: {} at {}",
            project_id,
            root_path.display()
        );

        let project = ManagedProject {
            project_id: project_id.clone(),
            root_path,
            // TODO: synergos-net 接続初期化
            // TODO: Presence 開始
        };

        self.projects.insert(project_id, project);
    }

    /// プロジェクトを閉じる
    pub async fn close(&self, project_id: &str) {
        if let Some((_, _project)) = self.projects.remove(project_id) {
            tracing::info!("Closing project: {}", project_id);
            // TODO: グレースフルシャットダウン
        }
    }

    /// 全プロジェクトを閉じる
    pub async fn close_all(&self) {
        let ids: Vec<String> = self
            .projects
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for id in ids {
            self.close(&id).await;
        }
    }

    /// プロジェクト一覧を取得
    pub fn list(&self) -> Vec<ProjectInfo> {
        self.projects
            .iter()
            .map(|entry| ProjectInfo {
                project_id: entry.project_id.clone(),
                root_path: entry.root_path.display().to_string(),
                peer_count: 0,        // TODO: Presence から取得
                active_transfers: 0,  // TODO: Exchange から取得
            })
            .collect()
    }

    /// 管理プロジェクト数
    pub fn count(&self) -> usize {
        self.projects.len()
    }
}
