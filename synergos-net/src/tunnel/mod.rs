// Tunnel Manager: Cloudflare Tunnel (cloudflared) 制御
// Phase 1-2 で実装予定

use crate::config::TunnelConfig;

/// Tunnel の状態
#[derive(Debug, Clone)]
pub enum TunnelState {
    Idle,
    Starting,
    Active { tunnel_id: String, uptime_secs: u64 },
    Error { message: String },
}

/// Cloudflare Tunnel マネージャ
pub struct TunnelManager {
    pub hostname: String,
    pub state: TunnelState,
}

impl TunnelManager {
    pub fn new(config: &TunnelConfig) -> Self {
        Self {
            hostname: config.hostname.clone(),
            state: TunnelState::Idle,
        }
    }
}
