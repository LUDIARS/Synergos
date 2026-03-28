//! Presence — ピア状態管理
//!
//! 接続可能なピアの発見と状態管理を行う。
//! DHT + Gossipsub + mDNS をバックエンドとして利用。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use crate::event_bus::SharedEventBus;

/// ピア状態管理サービス
pub struct PresenceService {
    _event_bus: SharedEventBus,
}

impl PresenceService {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            _event_bus: event_bus,
        }
    }

    // TODO: Phase 3 で実装
    // - ピア登録・発見
    // - 状態追跡（Active / Idle / Away / Offline）
    // - mDNS ローカル発見
    // - CloudflareKV / CollabRelay バックエンド
}
