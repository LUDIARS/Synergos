// QUIC: セッション管理 (quinn)
// Phase 1-3 で実装予定

use crate::config::QuicConfig;
use crate::types::TransferId;

/// QUIC ストリーム種別
#[derive(Debug, Clone)]
pub enum StreamType {
    /// 制御メッセージ
    Control,
    /// ファイルデータ転送
    Data { transfer_id: TransferId },
}

/// QUIC セッションマネージャ
pub struct QuicManager {
    config: QuicConfig,
}

impl QuicManager {
    pub fn new(config: QuicConfig) -> Self {
        Self { config }
    }
}
