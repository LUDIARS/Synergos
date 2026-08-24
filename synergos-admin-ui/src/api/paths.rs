//! API パスの組み立て。
//!
//! org_id / node_id は URL パスへ入るため、必ずエスケープしてから埋め込む
//! (control 側 slug は `[a-z0-9-]` だが、node_id は UUID・将来の値も想定する)。

pub const ORGS: &str = "/v1/orgs";
pub const RECONCILE: &str = "/v1/reconcile";
pub const MESH_CONTEXT: &str = "/v1/mesh/context";
pub const MESH_TOKEN_CHECK: &str = "/v1/mesh/token-check";
pub const MESH_RECONCILE: &str = "/v1/mesh/reconcile";
pub const MESH_CONNECTOR_TOKENS: &str = "/v1/mesh/connector-tokens";

pub fn org_nodes(org_id: &str) -> String {
    format!("/v1/orgs/{}/nodes", escape(org_id))
}

pub fn node(org_id: &str, node_id: &str) -> String {
    format!("/v1/orgs/{}/nodes/{}", escape(org_id), escape(node_id))
}

pub fn node_connector_token(org_id: &str, node_id: &str) -> String {
    format!("{}/connector-token", node(org_id, node_id))
}

/// パスセグメントの最小限のエスケープ。
/// 英数字と `-._~` 以外はパーセントエンコードする。
fn escape(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
