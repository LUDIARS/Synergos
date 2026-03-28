// Mesh: IPv6 Direct / TURN / STUN 接続
// Phase 2 で実装予定

use crate::config::MeshConfig;

/// IPv6/TURN メッシュ接続マネージャ
pub struct Mesh {
    config: MeshConfig,
}

impl Mesh {
    pub fn new(config: MeshConfig) -> Self {
        Self { config }
    }
}
