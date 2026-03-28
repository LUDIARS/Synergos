//! Conflict — コンフリクト管理
//!
//! チェーンフォークの検出とコンフリクト通知を管理する。
//! ホットスタンバイ機能でオフラインノードへの通知を保証する。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use crate::event_bus::SharedEventBus;

/// コンフリクト管理サービス
pub struct ConflictManager {
    _event_bus: SharedEventBus,
}

impl ConflictManager {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            _event_bus: event_bus,
        }
    }

    // TODO: Phase 3 で実装
    // - チェーンフォーク検出
    // - コンフリクト通知（Gossipsub 経由）
    // - ホットスタンバイ（オフラインノード向け）
    // - 解決操作（KeepLocal / AcceptRemote / ManualMerge）
}
