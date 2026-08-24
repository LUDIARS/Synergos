//! synergos-control の API が返す DTO。
//!
//! サーバー側 (`synergos-control/src/api`, `src/store`, `src/reconcile.rs`) の
//! 応答形と一対一で対応させる。未知フィールドは無視する。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Org {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub created_at_ms: u64,
}

#[derive(Clone, Serialize)]
pub struct CreateOrgRequest {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    MeshNode,
    ClientDevice,
}

impl NodeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::MeshNode => "Mesh node (Linux 常駐)",
            Self::ClientDevice => "Client device (人の端末)",
        }
    }

    pub fn wire_value(self) -> &'static str {
        match self {
            Self::MeshNode => "mesh_node",
            Self::ClientDevice => "client_device",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "mesh_node" => Some(Self::MeshNode),
            "client_device" => Some(Self::ClientDevice),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodeView {
    pub id: String,
    pub org_id: String,
    pub display_name: String,
    pub owner_email: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub cf_connector_id: Option<String>,
    #[serde(default)]
    pub synergos_peer_id: Option<String>,
    #[serde(default)]
    pub mesh_ip: Option<String>,
    #[serde(default)]
    pub reported_mesh_ip: Option<String>,
    #[serde(default)]
    pub last_heartbeat_ms: Option<u64>,
    #[serde(default)]
    pub synergos_version: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterNodeRequest {
    pub display_name: String,
    pub owner_email: String,
    pub kind: NodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synergos_peer_id: Option<String>,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct RegisterNodeResponse {
    pub node: NodeView,
    #[serde(default)]
    pub connector_token: Option<String>,
    pub node_key: String,
    #[serde(default)]
    pub enroll_hint: Option<String>,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct ConnectorTokenResponse {
    pub connector_token: String,
}

// --- reconcile ---

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodeRef {
    pub node_id: String,
    pub org_id: String,
    pub display_name: String,
    pub owner_email: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MeshConnector {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DeviceRegistration {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub virtual_ipv4: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MeshIpMismatch {
    pub node: NodeRef,
    #[serde(default)]
    pub reported_mesh_ip: Option<String>,
    #[serde(default)]
    pub expected_mesh_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReconcileReport {
    #[serde(default)]
    pub generated_at_ms: u64,
    #[serde(default)]
    pub dark_connectors: Vec<MeshConnector>,
    #[serde(default)]
    pub dark_devices: Vec<DeviceRegistration>,
    #[serde(default)]
    pub missing_connectors: Vec<NodeRef>,
    #[serde(default)]
    pub mesh_ip_mismatches: Vec<MeshIpMismatch>,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl ReconcileReport {
    /// 要注意件数の合計 (ダッシュボードの一言サマリ用)。
    pub fn attention_count(&self) -> usize {
        self.dark_connectors.len()
            + self.dark_devices.len()
            + self.missing_connectors.len()
            + self.mesh_ip_mismatches.len()
    }
}

// --- Mesh 自動設定 (request-scoped Cloudflare token) ---

#[derive(Clone, Serialize)]
pub struct TokenCheckRequest {
    pub api_token: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TokenCheckResponse {
    pub token_status: String,
    #[serde(default)]
    pub expires_on: Option<String>,
    pub account_id: String,
    pub mesh_node_count: usize,
}

#[derive(Clone, Serialize)]
pub struct MeshReconcileRequest {
    pub api_token: String,
    pub revoke_dark: bool,
}

#[derive(Clone, Serialize)]
pub struct ConnectorTokensRequest {
    pub api_token: String,
    pub org_id: String,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct ConnectorTokenEntry {
    pub node_id: String,
    pub display_name: String,
    #[serde(default)]
    pub connector_token: Option<String>,
    #[serde(default)]
    pub skipped_reason: Option<String>,
    #[serde(default)]
    pub enroll_command: Option<String>,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct ConnectorTokensResponse {
    pub org_id: String,
    pub issued: usize,
    #[serde(default)]
    pub entries: Vec<ConnectorTokenEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MeshContext {
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub api_base: String,
}
