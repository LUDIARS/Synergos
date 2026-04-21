//! Conflict — コンフリクト管理
//!
//! チェーンフォークの検出とコンフリクト通知を管理する。
//! ホットスタンバイ機能でオフラインノードへの通知を保証する。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use synergos_net::chain::{ChainBlock, FileChain};
use synergos_net::types::{Blake3Hash, FileId, PeerId};

use crate::event_bus::{ConflictDetectedEvent, SharedEventBus};

// ── 型定義 ──

/// コンフリクト解決方法
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    /// ローカル版を採用
    KeepLocal,
    /// リモート版を採用
    AcceptRemote,
    /// 手動マージ済み
    ManualMerge,
}

/// コンフリクト状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictState {
    /// アクティブ（未解決）
    Active,
    /// 解決済み
    Resolved { resolution: ConflictResolution },
}

/// コンフリクト情報
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub file_id: FileId,
    /// 共通の祖先ブロックのハッシュ
    pub common_ancestor_hash: Option<Blake3Hash>,
    pub common_ancestor_version: u64,
    /// ローカルの HEAD バージョン
    pub local_version: u64,
    pub local_author: PeerId,
    /// リモートの HEAD バージョン
    pub remote_version: u64,
    pub remote_author: PeerId,
    /// コンフリクトに関与するノード
    pub involved_peers: Vec<PeerId>,
    /// 検出時刻（Unix epoch ミリ秒）
    pub detected_at: u64,
    /// 状態
    pub state: ConflictState,
    /// ファイルパス（表示用）
    pub file_path: String,
    /// プロジェクトID
    pub project_id: String,
}

/// ホットスタンバイ通知（オフラインノード向け）
#[derive(Debug, Clone)]
pub struct PendingNotification {
    pub conflict: ConflictInfo,
    pub target_peer: PeerId,
    pub created_at: u64,
    pub retry_count: u32,
}

/// コンフリクト管理エラー
#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error("conflict not found: {0}")]
    NotFound(String),
    #[error("conflict already resolved: {0}")]
    AlreadyResolved(String),
}

// ── 実装 ──

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// コンフリクト管理サービス
pub struct ConflictManager {
    event_bus: SharedEventBus,
    /// アクティブなコンフリクト（FileId → ConflictInfo）
    conflicts: DashMap<FileId, ConflictInfo>,
    /// ホットスタンバイ通知（オフラインノード向け）
    hot_standby: DashMap<PeerId, Vec<PendingNotification>>,
}

impl ConflictManager {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            event_bus,
            conflicts: DashMap::new(),
            hot_standby: DashMap::new(),
        }
    }

    /// チェーンフォークを検出してコンフリクトを生成する
    ///
    /// ローカルチェーンの HEAD とリモートブロックの prev_hash が不一致の場合、
    /// 分岐点を特定してコンフリクト情報を生成する。
    pub fn detect_conflict(
        &self,
        project_id: &str,
        file_id: &FileId,
        file_path: &str,
        local_chain: &FileChain,
        remote_block: &ChainBlock,
        local_peer: &PeerId,
    ) -> Option<ConflictInfo> {
        // チェーンの HEAD が存在しない場合はコンフリクトなし
        let local_head = local_chain.head.as_ref()?;

        // remote_block の prev_hash が local の HEAD と一致すれば正常
        if remote_block.prev_hash.as_ref() == Some(local_head) {
            return None;
        }

        // 既にこのファイルのコンフリクトが存在する場合は重複検出しない
        if self.conflicts.contains_key(file_id) {
            return None;
        }

        // 分岐点を特定: remote_block の prev_hash がチェーン内に存在するか探す
        let (ancestor_hash, ancestor_version) =
            if let Some(prev) = &remote_block.prev_hash {
                // ローカルチェーンの中から一致するブロックを探す
                let found = local_chain
                    .blocks_iter()
                    .find(|b| &b.hash == prev);
                match found {
                    Some(b) => (Some(b.hash.clone()), b.version),
                    None => (None, 0),
                }
            } else {
                (None, 0)
            };

        let local_version = local_chain
            .blocks_iter()
            .last()
            .map(|b| b.version)
            .unwrap_or(0);

        let conflict = ConflictInfo {
            file_id: file_id.clone(),
            common_ancestor_hash: ancestor_hash,
            common_ancestor_version: ancestor_version,
            local_version,
            local_author: local_peer.clone(),
            remote_version: remote_block.version,
            remote_author: remote_block.author.clone(),
            involved_peers: vec![local_peer.clone(), remote_block.author.clone()],
            detected_at: now_ms(),
            state: ConflictState::Active,
            file_path: file_path.to_string(),
            project_id: project_id.to_string(),
        };

        // コンフリクトを登録
        self.conflicts.insert(file_id.clone(), conflict.clone());

        // 関連 peer に対して hot-standby 通知を自動 enqueue (#15 対策)。
        // これまで queue_notification を呼ぶ側がおらず機能していなかった。
        for peer in &conflict.involved_peers {
            if peer != local_peer {
                self.queue_notification(&conflict, peer);
            }
        }

        // EventBus にコンフリクト検出イベントを発行
        self.event_bus.emit(ConflictDetectedEvent {
            project_id: project_id.to_string(),
            file_id: file_id.to_string(),
            file_path: file_path.to_string(),
            involved_peers: conflict
                .involved_peers
                .iter()
                .map(|p| p.to_string())
                .collect(),
        });

        Some(conflict)
    }

    /// コンフリクトを解決する
    pub fn resolve_conflict(
        &self,
        file_id: &FileId,
        resolution: ConflictResolution,
    ) -> Result<ConflictInfo, ConflictError> {
        match self.conflicts.get_mut(file_id) {
            Some(mut entry) => {
                if let ConflictState::Resolved { .. } = &entry.state {
                    return Err(ConflictError::AlreadyResolved(file_id.to_string()));
                }

                entry.state = ConflictState::Resolved { resolution };
                let resolved = entry.clone();

                tracing::info!(
                    "Conflict resolved for file {}: {:?}",
                    file_id,
                    resolution
                );

                Ok(resolved)
            }
            None => Err(ConflictError::NotFound(file_id.to_string())),
        }
    }

    /// アクティブなコンフリクト一覧を取得
    pub fn list_conflicts(&self, project_id: Option<&str>) -> Vec<ConflictInfo> {
        self.conflicts
            .iter()
            .filter(|entry| {
                entry.state == ConflictState::Active
                    && match project_id {
                        Some(pid) => entry.project_id == pid,
                        None => true,
                    }
            })
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// 指定ファイルのコンフリクト情報を取得
    pub fn get_conflict(&self, file_id: &FileId) -> Option<ConflictInfo> {
        self.conflicts
            .get(file_id)
            .map(|entry| entry.value().clone())
    }

    /// 解決済みコンフリクトをクリーンアップ
    pub fn cleanup_resolved(&self) {
        self.conflicts
            .retain(|_, v| v.state == ConflictState::Active);
    }

    /// オフラインノード向けにコンフリクト通知をホットスタンバイに保存
    pub fn queue_notification(&self, conflict: &ConflictInfo, target_peer: &PeerId) {
        let notification = PendingNotification {
            conflict: conflict.clone(),
            target_peer: target_peer.clone(),
            created_at: now_ms(),
            retry_count: 0,
        };

        self.hot_standby
            .entry(target_peer.clone())
            .or_default()
            .push(notification);

        tracing::debug!(
            "Queued conflict notification for offline peer: {}",
            target_peer
        );
    }

    /// ピアがオンラインに復帰した際にホットスタンバイ通知を取得
    pub fn drain_pending_notifications(&self, peer: &PeerId) -> Vec<PendingNotification> {
        self.hot_standby
            .remove(peer)
            .map(|(_, notifications)| notifications)
            .unwrap_or_default()
    }

    /// ホットスタンバイの期限切れ通知を削除（TTL: 24時間）
    pub fn cleanup_expired_notifications(&self, ttl_ms: u64) {
        let now = now_ms();
        self.hot_standby.iter_mut().for_each(|mut entry| {
            entry.value_mut().retain(|n| now - n.created_at < ttl_ms);
        });
        self.hot_standby.retain(|_, v| !v.is_empty());
    }
}
