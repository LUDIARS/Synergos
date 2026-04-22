use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// listen アドレス (例 `0.0.0.0:8080`)
    pub listen_addr: SocketAddr,
    /// 1 サーバあたりの最大 room 数 (超過時は新 room の join を拒否)
    #[serde(default = "default_max_rooms")]
    pub max_rooms: usize,
    /// 1 room あたりの最大クライアント数
    #[serde(default = "default_max_clients_per_room")]
    pub max_clients_per_room: usize,
    /// クライアントからのメッセージが無いと disconnect するまでの秒数 (keepalive)。
    /// 0 ならタイムアウト無効。
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

fn default_max_rooms() -> usize {
    4096
}
fn default_max_clients_per_room() -> usize {
    64
}
fn default_idle_timeout_secs() -> u64 {
    300
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8080".parse().unwrap(),
            max_rooms: default_max_rooms(),
            max_clients_per_room: default_max_clients_per_room(),
            idle_timeout_secs: default_idle_timeout_secs(),
        }
    }
}

impl RelayConfig {
    pub fn from_path(path: &std::path::Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }
}
