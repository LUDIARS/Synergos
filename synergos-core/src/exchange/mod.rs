//! Exchange — ファイル転送制御
//!
//! synergos-net の QUIC ストリームを使ってファイルを送受信する。
//! 優先度キュー、帯域制御、チャンク分割・再組立を管理する。
//!
//! 旧 ars-plugin-synergos から synergos-core に移植。Ars 依存を除去。

use crate::event_bus::SharedEventBus;

/// ファイル転送制御サービス
pub struct Exchange {
    _event_bus: SharedEventBus,
}

impl Exchange {
    pub fn new(event_bus: SharedEventBus) -> Self {
        Self {
            _event_bus: event_bus,
        }
    }

    // TODO: Phase 3 で実装
    // - 転送リクエスト受付
    // - 優先度キュー管理
    // - チャンク分割・再組立
    // - 帯域制御
    // - 進捗通知（EventBus 経由）
}
