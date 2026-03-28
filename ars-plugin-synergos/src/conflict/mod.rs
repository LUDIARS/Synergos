//! コンフリクト管理
//!
//! チェーンの分岐（フォーク）を検出し、関与ノードへ通知する。
//! オフラインノード向けのホットスタンバイ情報も管理する。

use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use synergos_net::chain::{ChainBlock, FileChain};
use synergos_net::gossip::{GossipMessage, GossipNode};
use synergos_net::types::{FileId, PeerId, TopicId};

/// コンフリクト情報
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub file_id: FileId,
    /// 共通の祖先ブロック
    pub common_ancestor: Option<ChainBlock>,
    /// ローカルの HEAD
    pub local_head: ChainBlock,
    /// リモートの HEAD
    pub remote_head: ChainBlock,
    /// コンフリクトに関与するノード
    pub involved_nodes: Vec<PeerId>,
    /// コンフリクト検出時刻
    pub detected_at: u64,
    /// 状態
    pub state: ConflictState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictState {
    /// アクティブ（未解決）
    Active,
    /// 解決済み
    Resolved { chosen: ConflictResolution },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    KeepLocal,
    AcceptRemote,
    ManualMerge,
}

/// コンフリクト通知（ホットスタンバイ用）
#[derive(Debug, Clone)]
pub struct ConflictNotification {
    pub conflict: ConflictInfo,
    pub target_peer: PeerId,
    pub notified_at: u64,
}

/// コンフリクトマネージャ
pub struct ConflictManager {
    /// アクティブなコンフリクト
    conflicts: DashMap<FileId, ConflictInfo>,
    /// ホットスタンバイ情報（オフラインノード向け）
    hot_standby: DashMap<PeerId, Vec<ConflictNotification>>,
    /// ホットスタンバイの保持期限 (秒)
    hot_standby_ttl_secs: u64,
}

impl ConflictManager {
    pub fn new(hot_standby_ttl_secs: u64) -> Self {
        Self {
            conflicts: DashMap::new(),
            hot_standby: DashMap::new(),
            hot_standby_ttl_secs,
        }
    }

    /// コンフリクトを検出・登録
    pub fn detect_conflict(
        &self,
        file_id: &FileId,
        local_chain: &FileChain,
        remote_block: &ChainBlock,
        local_peer: &PeerId,
    ) -> Option<ConflictInfo> {
        let local_head = local_chain.head_block()?;

        // prev_hash が一致しない → フォーク
        if remote_block.prev_hash != local_chain.head {
            let common_ancestor = local_chain
                .find_block(&remote_block.prev_hash.clone().unwrap_or_default())
                .cloned();

            let info = ConflictInfo {
                file_id: file_id.clone(),
                common_ancestor,
                local_head: local_head.clone(),
                remote_head: remote_block.clone(),
                involved_nodes: vec![local_peer.clone(), remote_block.author.clone()],
                detected_at: now_ms(),
                state: ConflictState::Active,
            };

            self.conflicts.insert(file_id.clone(), info.clone());
            return Some(info);
        }
        None
    }

    /// コンフリクトを Gossip で関与ノードに通知
    pub fn notify_conflict(
        &self,
        conflict: &ConflictInfo,
        gossip: &GossipNode,
        topic: &TopicId,
    ) {
        gossip.publish(
            topic,
            GossipMessage::ConflictAlert {
                file_id: conflict.file_id.clone(),
                conflicting_nodes: conflict.involved_nodes.clone(),
                their_versions: vec![
                    conflict.local_head.version,
                    conflict.remote_head.version,
                ],
            },
        );
    }

    /// オフラインノード向けにホットスタンバイに保存
    pub fn store_hot_standby(&self, peer: &PeerId, conflict: &ConflictInfo) {
        self.hot_standby
            .entry(peer.clone())
            .or_default()
            .push(ConflictNotification {
                conflict: conflict.clone(),
                target_peer: peer.clone(),
                notified_at: now_ms(),
            });
    }

    /// ピアがオンラインに復帰した際にホットスタンバイ情報を配信
    pub fn flush_hot_standby(
        &self,
        peer: &PeerId,
        gossip: &GossipNode,
        topic: &TopicId,
    ) -> Vec<ConflictInfo> {
        let mut flushed = Vec::new();
        if let Some((_, notifications)) = self.hot_standby.remove(peer) {
            let cutoff = now_ms() - (self.hot_standby_ttl_secs * 1000);
            for notif in notifications {
                if notif.notified_at >= cutoff {
                    self.notify_conflict(&notif.conflict, gossip, topic);
                    flushed.push(notif.conflict);
                }
            }
        }
        flushed
    }

    /// コンフリクト中のファイルを更新（許可するが通知を飛ばす）
    pub fn update_during_conflict(
        &self,
        file_id: &FileId,
        new_version: u64,
        gossip: &GossipNode,
        topic: &TopicId,
    ) -> bool {
        if let Some(conflict) = self.conflicts.get(file_id) {
            gossip.publish(
                topic,
                GossipMessage::ConflictAlert {
                    file_id: file_id.clone(),
                    conflicting_nodes: conflict.involved_nodes.clone(),
                    their_versions: vec![new_version],
                },
            );
            return true; // conflict exists, notification sent
        }
        false
    }

    /// コンフリクトを解決
    pub fn resolve(&self, file_id: &FileId, resolution: ConflictResolution) -> bool {
        if let Some(mut conflict) = self.conflicts.get_mut(file_id) {
            conflict.state = ConflictState::Resolved { chosen: resolution };
            return true;
        }
        false
    }

    /// アクティブなコンフリクト一覧を取得
    pub fn active_conflicts(&self) -> Vec<ConflictInfo> {
        self.conflicts
            .iter()
            .filter(|e| e.state == ConflictState::Active)
            .map(|e| e.value().clone())
            .collect()
    }

    /// 指定ファイルがコンフリクト中か確認
    pub fn is_conflicted(&self, file_id: &FileId) -> bool {
        self.conflicts
            .get(file_id)
            .map(|c| c.state == ConflictState::Active)
            .unwrap_or(false)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
