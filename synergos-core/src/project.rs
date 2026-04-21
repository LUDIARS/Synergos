//! プロジェクト管理
//!
//! Core デーモンが管理するプロジェクトのライフサイクルを制御する。
//! ノード（ピア）はプロジェクトに紐づいて管理される。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use dashmap::DashMap;

use synergos_ipc::response::{ProjectDetail, ProjectInfo};
use synergos_net::gossip::GossipNode;
use synergos_net::types::TopicId;

use crate::event_bus::SharedEventBus;

// ── 型定義 ──

/// 同期モード
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMode {
    /// 全ファイルを自動同期
    Full,
    /// 変更通知のみ（ダウンロードは手動）
    Manual,
    /// 指定パターンのみ同期
    Selective { patterns: Vec<String> },
}

impl SyncMode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Full => "full",
            Self::Manual => "manual",
            Self::Selective { .. } => "selective",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "manual" => Self::Manual,
            "selective" => Self::Selective { patterns: vec![] },
            _ => Self::Full,
        }
    }
}

/// プロジェクト設定
#[derive(Debug, Clone)]
pub struct ProjectSettings {
    pub display_name: String,
    pub description: String,
    pub sync_mode: SyncMode,
    /// 最大接続ピア数（0 = 無制限）
    pub max_peers: u16,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            description: String::new(),
            sync_mode: SyncMode::Full,
            max_peers: 0,
        }
    }
}

/// プロジェクト設定の部分更新
#[derive(Debug, Clone, Default)]
pub struct ProjectSettingsPatch {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub sync_mode: Option<String>,
    pub max_peers: Option<u16>,
}

/// 招待トークン
#[derive(Debug, Clone)]
pub struct InviteToken {
    pub token: String,
    pub project_id: String,
    /// 有効期限（Unix epoch 秒）。None = 無期限
    pub expires_at: Option<u64>,
}

/// プロジェクト管理エラー
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("project already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid invite token")]
    InvalidInvite,
    #[error("invite token expired")]
    InviteExpired,
    #[error("max peers reached for project: {0}")]
    MaxPeersReached(String),
}

// ── trait 定義 ──

/// プロジェクト設定インターフェース
///
/// プロジェクトの作成・設定変更・招待・参加を管理する。
/// ノード（ピア）はプロジェクトにぶら下がる形で管理され、
/// プロジェクト単位で同期・転送・Presence が制御される。
#[async_trait]
pub trait ProjectConfiguration: Send + Sync {
    /// プロジェクトを開く（ネットワーク参加）
    async fn open_project(
        &self,
        project_id: String,
        root_path: PathBuf,
        display_name: Option<String>,
    ) -> Result<(), ProjectError>;

    /// プロジェクトを閉じる（ネットワーク離脱）
    async fn close_project(&self, project_id: &str) -> Result<(), ProjectError>;

    /// プロジェクトの詳細を取得
    async fn get_project(&self, project_id: &str) -> Result<ProjectDetail, ProjectError>;

    /// プロジェクト設定を部分更新
    async fn update_project(
        &self,
        project_id: &str,
        patch: ProjectSettingsPatch,
    ) -> Result<(), ProjectError>;

    /// プロジェクト一覧
    fn list_projects(&self) -> Vec<ProjectInfo>;

    /// 招待トークンを生成
    async fn create_invite(
        &self,
        project_id: &str,
        expires_in_secs: Option<u64>,
    ) -> Result<InviteToken, ProjectError>;

    /// 招待トークンでプロジェクトに参加
    async fn join_project(
        &self,
        invite_token: &str,
        root_path: PathBuf,
    ) -> Result<String, ProjectError>;

    /// 管理プロジェクト数
    fn count(&self) -> usize;
}

// ── 実装 ──

/// プロジェクトの内部状態
struct ManagedProject {
    project_id: String,
    root_path: PathBuf,
    settings: ProjectSettings,
    created_at: u64,
    /// このプロジェクトに接続中のピアID一覧
    connected_peer_ids: Vec<String>,
}

/// プロジェクトマネージャ
///
/// 複数プロジェクトの同時管理をサポートする。
/// Exchange/Presence/ConflictManager は ServiceContext で一元管理し、
/// プロジェクト操作時にそれらのサービスと連携する。
pub struct ProjectManager {
    projects: DashMap<String, ManagedProject>,
    /// 招待トークン → project_id のマッピング
    invites: DashMap<String, InviteToken>,
    event_bus: SharedEventBus,
    /// Gossipsub（任意）: プロジェクト開閉時にトピック subscribe/unsubscribe を行う
    gossip: Option<Arc<GossipNode>>,
}

impl ProjectManager {
    /// 最小構成のコンストラクタ（テスト・後方互換用）
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self::with_gossip(event_bus, None)
    }

    /// Gossipsub 依存付きのコンストラクタ
    pub fn with_gossip(event_bus: SharedEventBus, gossip: Option<Arc<GossipNode>>) -> Self {
        Self {
            projects: DashMap::new(),
            invites: DashMap::new(),
            event_bus,
            gossip,
        }
    }

    /// 期限切れ招待トークンを除去
    pub fn gc_expired_invites(&self) -> usize {
        let now = now_epoch_secs();
        let before = self.invites.len();
        self.invites.retain(|_, inv| match inv.expires_at {
            Some(exp) => exp > now,
            None => true,
        });
        before - self.invites.len()
    }

    // 既存コードとの後方互換用ヘルパー
    pub async fn open(&self, project_id: String, root_path: PathBuf) {
        let _ = self.open_project(project_id, root_path, None).await;
    }

    pub async fn close(&self, project_id: &str) {
        let _ = self.close_project(project_id).await;
    }

    pub async fn close_all(&self) {
        let ids: Vec<String> = self
            .projects
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for id in ids {
            let _ = self.close_project(&id).await;
        }
    }

    pub fn list(&self) -> Vec<ProjectInfo> {
        self.list_projects()
    }
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[async_trait]
impl ProjectConfiguration for ProjectManager {
    async fn open_project(
        &self,
        project_id: String,
        root_path: PathBuf,
        display_name: Option<String>,
    ) -> Result<(), ProjectError> {
        if self.projects.contains_key(&project_id) {
            return Err(ProjectError::AlreadyExists(project_id));
        }

        tracing::info!(
            "Opening project: {} at {}",
            project_id,
            root_path.display()
        );

        let settings = ProjectSettings {
            display_name: display_name.unwrap_or_else(|| project_id.clone()),
            ..Default::default()
        };

        let project = ManagedProject {
            project_id: project_id.clone(),
            root_path,
            settings,
            created_at: now_epoch_secs(),
            connected_peer_ids: Vec::new(),
        };

        // Gossipsub にプロジェクトトピックを subscribe
        if let Some(gossip) = &self.gossip {
            gossip.subscribe(TopicId::project(&project_id));
        }

        self.projects.insert(project_id, project);
        Ok(())
    }

    async fn close_project(&self, project_id: &str) -> Result<(), ProjectError> {
        match self.projects.remove(project_id) {
            Some((_, project)) => {
                tracing::info!("Closing project: {}", project_id);
                // Gossipsub からトピック購読解除
                if let Some(gossip) = &self.gossip {
                    gossip.unsubscribe(&TopicId::project(project_id));
                }
                // 関連する招待トークンも削除
                self.invites.retain(|_, inv| inv.project_id != project_id);

                // EventBus にプロジェクトクローズを通知（Presence/Exchange が購読して処理）
                for peer_id in &project.connected_peer_ids {
                    self.event_bus.emit(crate::event_bus::PeerDisconnectedEvent {
                        project_id: project_id.to_string(),
                        peer_id: peer_id.clone(),
                        reason: "project closed".to_string(),
                    });
                }

                Ok(())
            }
            None => Err(ProjectError::NotFound(project_id.to_string())),
        }
    }

    async fn get_project(&self, project_id: &str) -> Result<ProjectDetail, ProjectError> {
        match self.projects.get(project_id) {
            Some(entry) => {
                let p = entry.value();
                Ok(ProjectDetail {
                    project_id: p.project_id.clone(),
                    display_name: p.settings.display_name.clone(),
                    description: p.settings.description.clone(),
                    root_path: p.root_path.display().to_string(),
                    sync_mode: p.settings.sync_mode.as_str().to_string(),
                    max_peers: p.settings.max_peers,
                    peer_count: p.connected_peer_ids.len(),
                    active_transfers: 0, // IPC server fills via ServiceContext
                    created_at: p.created_at,
                    connected_peer_ids: p.connected_peer_ids.clone(),
                })
            }
            None => Err(ProjectError::NotFound(project_id.to_string())),
        }
    }

    async fn update_project(
        &self,
        project_id: &str,
        patch: ProjectSettingsPatch,
    ) -> Result<(), ProjectError> {
        match self.projects.get_mut(project_id) {
            Some(mut entry) => {
                let settings = &mut entry.value_mut().settings;
                if let Some(name) = patch.display_name {
                    settings.display_name = name;
                }
                if let Some(desc) = patch.description {
                    settings.description = desc;
                }
                if let Some(mode) = patch.sync_mode {
                    settings.sync_mode = SyncMode::from_str(&mode);
                }
                if let Some(max) = patch.max_peers {
                    settings.max_peers = max;
                }
                tracing::info!("Updated project settings: {}", project_id);
                Ok(())
            }
            None => Err(ProjectError::NotFound(project_id.to_string())),
        }
    }

    fn list_projects(&self) -> Vec<ProjectInfo> {
        self.projects
            .iter()
            .map(|entry| {
                let p = entry.value();
                ProjectInfo {
                    project_id: p.project_id.clone(),
                    display_name: p.settings.display_name.clone(),
                    root_path: p.root_path.display().to_string(),
                    peer_count: p.connected_peer_ids.len(),
                    active_transfers: 0, // IPC server fills via ServiceContext
                }
            })
            .collect()
    }

    async fn create_invite(
        &self,
        project_id: &str,
        expires_in_secs: Option<u64>,
    ) -> Result<InviteToken, ProjectError> {
        if !self.projects.contains_key(project_id) {
            return Err(ProjectError::NotFound(project_id.to_string()));
        }

        let token = uuid::Uuid::new_v4().to_string();
        let expires_at = expires_in_secs.map(|secs| now_epoch_secs() + secs);

        let invite = InviteToken {
            token: token.clone(),
            project_id: project_id.to_string(),
            expires_at,
        };

        // トークン本体はログに残さない（クレデンシャル扱い）
        tracing::info!(
            "Created invite for project {} (expires_at={:?})",
            project_id,
            expires_at
        );

        self.invites.insert(token, invite.clone());
        Ok(invite)
    }

    async fn join_project(
        &self,
        invite_token: &str,
        root_path: PathBuf,
    ) -> Result<String, ProjectError> {
        let invite = self
            .invites
            .get(invite_token)
            .map(|e| e.value().clone())
            .ok_or(ProjectError::InvalidInvite)?;

        // 有効期限チェック
        if let Some(expires_at) = invite.expires_at {
            if now_epoch_secs() > expires_at {
                self.invites.remove(invite_token);
                return Err(ProjectError::InviteExpired);
            }
        }

        let project_id = invite.project_id.clone();

        // 既に参加中ならそのまま返す
        if self.projects.contains_key(&project_id) {
            return Ok(project_id);
        }

        tracing::info!(
            "Joining project {} via invite at {}",
            project_id,
            root_path.display()
        );

        // リモートからのプロジェクト設定取得は Gossipsub 経由で非同期に行われる
        self.open_project(project_id.clone(), root_path, None).await?;

        Ok(project_id)
    }

    fn count(&self) -> usize {
        self.projects.len()
    }
}
