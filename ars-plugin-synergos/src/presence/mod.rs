//! Presence: ピア状態管理
//!
//! DHT + Gossipsub を活用して接続可能なピアの発見と状態管理を行う。

use synergos_net::gossip::ActivityState;
use synergos_net::types::{PeerId, Route};

/// ピア情報
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub endpoints: Vec<Route>,
    pub state: ActivityState,
    pub project_id: String,
    /// 帯域 (bps)
    pub bandwidth_bps: Option<u64>,
    /// RTT (ms)
    pub rtt_ms: Option<u32>,
}

/// Presence バックエンド種別
#[derive(Debug, Clone)]
pub enum PresenceBackend {
    /// Cloudflare Workers KV
    CloudflareKV {
        account_id: String,
        namespace_id: String,
    },
    /// ars-collab WebSocket リレー
    CollabRelay {
        server_url: String,
    },
    /// mDNS（同一LAN内）
    Mdns,
}

/// Presence サービス（スタブ）
pub struct PresenceService {
    // TODO: Phase 3 で実装
}

impl PresenceService {
    pub fn new() -> Self {
        Self {}
    }
}
