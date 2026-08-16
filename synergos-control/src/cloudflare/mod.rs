mod client;

pub use client::CloudflareClient;

use serde::{Deserialize, Serialize};

/// Cloudflare 側の Mesh node (API 上は warp_connector tunnel)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConnector {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

/// Cloudflare 側の WARP デバイス登録 (人の端末)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistration {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// エンロールしたユーザのメールアドレス。
    #[serde(default)]
    pub user_email: Option<String>,
    /// Cloudflare が割り当てた Mesh IP (100.96.0.0/12)。
    #[serde(default)]
    pub virtual_ipv4: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
}
