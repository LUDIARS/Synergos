use std::sync::Arc;

use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use tracing::{info, warn};

use crate::error::{ControlError, ControlResult};
use crate::store::now_ms;

use super::AppState;

/// Synergos daemon からの heartbeat。
/// 認証は管理トークンではなく、ノード登録時に発行した node key (Bearer)。
#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub node_id: String,
    pub peer_id: String,
    #[serde(default)]
    pub mesh_ip: Option<String>,
    #[serde(default)]
    pub synergos_version: Option<String>,
}

/// ノード登録時に返す node key を生成する (v4 UUID 2 本 = 244bit 相当、OS RNG 由来)。
pub fn generate_node_key() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// node key の保存用ハッシュ (キー本体は保存しない)。
pub fn hash_node_key(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

pub async fn receive_heartbeat(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<HeartbeatRequest>,
) -> ControlResult<Json<serde_json::Value>> {
    let provided_key = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ControlError::Unauthorized)?;

    // キー照合と更新を同じロック内で行う。別々に行うと、照合直後にキーが
    // 再発行された場合に旧キーの heartbeat を受理する競合が生じる。
    let provided_hash = hash_node_key(provided_key);
    let node_id = req.node_id.clone();
    let result = state
        .store
        .try_mutate_node(None, &node_id, move |node| {
            let Some(expected_hash) = node.node_key_hash.as_deref() else {
                warn!(node = %node.id, "heartbeat received but node has no key issued");
                return Err(ControlError::Unauthorized);
            };
            if !crate::auth::constant_time_eq(
                provided_hash.as_bytes(),
                expected_hash.as_bytes(),
            ) {
                return Err(ControlError::Unauthorized);
            }
            if req.peer_id.is_empty()
                || req.peer_id.len() > 256
                || req.peer_id.chars().any(char::is_control)
            {
                return Err(ControlError::InvalidRequest(
                    "peer_id must be 1..=256 printable characters".to_string(),
                ));
            }
            if req.synergos_version.as_ref().is_some_and(|version| {
                version.len() > 128 || version.chars().any(char::is_control)
            }) {
                return Err(ControlError::InvalidRequest(
                    "synergos_version must be at most 128 printable characters".to_string(),
                ));
            }
            let mesh_ip = req
                .mesh_ip
                .as_deref()
                .map(super::normalize_mesh_ip)
                .transpose()?;
            if node.synergos_peer_id.as_deref() != Some(req.peer_id.as_str()) {
                info!(node = %node.id, peer_id = %req.peer_id, "heartbeat updated peer_id");
            }
            node.synergos_peer_id = Some(req.peer_id);
            node.reported_mesh_ip = mesh_ip;
            node.synergos_version = req.synergos_version;
            node.last_heartbeat_ms = Some(now_ms());
            node.updated_at_ms = now_ms();
            Ok(())
        })
        .await;

    // 存在しない node_id と不正キーを応答で区別しない (列挙攻撃対策)。
    match result {
        Err(ControlError::NotFound(_)) => return Err(ControlError::Unauthorized),
        Err(err) => return Err(err),
        Ok(_) => {}
    }

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloudflare::CloudflareClient;
    use crate::store::{JsonStore, Node, NodeKind, Org};
    use axum::http::HeaderValue;

    fn bearer(token: &str) -> HeaderMap {
        let value: HeaderValue = format!("Bearer {token}").parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        headers
    }

    async fn beat_raw(
        state: &Arc<AppState>,
        headers: HeaderMap,
        req: HeartbeatRequest,
    ) -> ControlResult<()> {
        receive_heartbeat(State(state.clone()), headers, Json(req))
            .await
            .map(|_| ())
    }

    async fn beat(state: &Arc<AppState>, key: &str, req: HeartbeatRequest) -> ControlResult<()> {
        beat_raw(state, bearer(key), req).await
    }

    fn node_with_key(id: &str, key: &str) -> Node {
        Node {
            id: id.to_string(),
            org_id: "acme".to_string(),
            display_name: id.to_string(),
            owner_email: "alice@acme.test".to_string(),
            kind: NodeKind::MeshNode,
            cf_connector_id: Some("cf-1".to_string()),
            synergos_peer_id: None,
            mesh_ip: None,
            reported_mesh_ip: None,
            node_key_hash: Some(hash_node_key(key)),
            last_heartbeat_ms: None,
            synergos_version: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    /// heartbeat は Cloudflare を呼ばないので、到達不能な api_base で十分。
    async fn state_with(dir: &std::path::Path, node: Node) -> Arc<AppState> {
        let store = JsonStore::open(dir.join("store.json")).unwrap();
        store
            .insert_org(Org {
                id: "acme".to_string(),
                name: "acme".to_string(),
                members: vec!["alice@acme.test".to_string()],
                created_at_ms: 0,
            })
            .await
            .unwrap();
        store.insert_node(node).await.unwrap();
        Arc::new(AppState {
            store,
            cloudflare: CloudflareClient::new(
                "http://127.0.0.1:1/client/v4".to_string(),
                "account".to_string(),
                "unused".to_string(),
            )
            .unwrap(),
        })
    }

    fn request(node_id: &str) -> HeartbeatRequest {
        HeartbeatRequest {
            node_id: node_id.to_string(),
            peer_id: "peer-1".to_string(),
            mesh_ip: Some("100.96.0.5".to_string()),
            synergos_version: Some("0.1.0".to_string()),
        }
    }

    #[tokio::test]
    async fn accepts_valid_node_key_and_records_report() {
        let dir = tempfile::tempdir().unwrap();
        let node = node_with_key("n1", "secret-key");
        let state = state_with(dir.path(), node).await;

        beat(&state, "secret-key", request("n1"))
            .await
            .expect("valid key accepted");

        let node = state.store.get_node("acme", "n1").await.unwrap();
        assert_eq!(node.synergos_peer_id.as_deref(), Some("peer-1"));
        assert_eq!(node.reported_mesh_ip.as_deref(), Some("100.96.0.5"));
        assert!(node.last_heartbeat_ms.is_some());
    }

    #[tokio::test]
    async fn rejects_wrong_key_unknown_node_and_missing_header() {
        let dir = tempfile::tempdir().unwrap();
        let node = node_with_key("n1", "secret-key");
        let state = state_with(dir.path(), node).await;

        // 誤ったキー / 未知の node_id / キーなし (空 Bearer)
        for (key, req) in [
            ("wrong-key", request("n1")),
            ("secret-key", request("does-not-exist")),
            ("", request("n1")),
        ] {
            let Err(err) = beat(&state, key, req).await else {
                panic!("heartbeat must be rejected for key {key:?}");
            };
            assert!(matches!(&err, ControlError::Unauthorized), "got {err:?}");
        }

        // Authorization ヘッダ自体が無い場合
        let no_header = beat_raw(&state, HeaderMap::new(), request("n1")).await;
        let Err(err) = no_header else {
            panic!("heartbeat without Authorization must be rejected");
        };
        assert!(matches!(&err, ControlError::Unauthorized), "got {err:?}");

        // 拒否されたリクエストは何も書き換えていない
        let node = state.store.get_node("acme", "n1").await.unwrap();
        assert!(node.synergos_peer_id.is_none());
        assert!(node.last_heartbeat_ms.is_none());
    }

    #[tokio::test]
    async fn rejects_node_without_issued_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut node = node_with_key("n1", "secret-key");
        node.node_key_hash = None;
        let state = state_with(dir.path(), node).await;

        let Err(err) = beat(&state, "secret-key", request("n1")).await else {
            panic!("node without an issued key must be rejected");
        };
        assert!(matches!(&err, ControlError::Unauthorized), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_unprintable_peer_id_without_mutating_node() {
        let dir = tempfile::tempdir().unwrap();
        let node = node_with_key("n1", "secret-key");
        let state = state_with(dir.path(), node).await;
        let mut req = request("n1");
        req.peer_id = "peer\nforged-log-line".to_string();

        let err = beat(&state, "secret-key", req).await.unwrap_err();
        assert!(matches!(err, ControlError::InvalidRequest(_)));
        let node = state.store.get_node("acme", "n1").await.unwrap();
        assert!(node.synergos_peer_id.is_none());
        assert!(node.last_heartbeat_ms.is_none());
    }

    /// heartbeat は管理者が登録した期待値 `mesh_ip` を巻き戻さない
    /// (レコード全体の上書きではなく差分適用であること)。
    #[tokio::test]
    async fn heartbeat_does_not_clobber_expected_mesh_ip() {
        let dir = tempfile::tempdir().unwrap();
        let node = node_with_key("n1", "secret-key");
        let state = state_with(dir.path(), node).await;

        // 管理者が期待する mesh_ip を書き込む
        state
            .store
            .mutate_node(Some("acme"), "n1", |node| {
                node.mesh_ip = Some("100.96.0.5".to_string());
            })
            .await
            .unwrap();

        let mut req = request("n1");
        req.mesh_ip = Some("100.96.0.99".to_string());
        beat(&state, "secret-key", req)
            .await
            .expect("valid key accepted");

        let node = state.store.get_node("acme", "n1").await.unwrap();
        assert_eq!(node.mesh_ip.as_deref(), Some("100.96.0.5"));
        assert_eq!(node.reported_mesh_ip.as_deref(), Some("100.96.0.99"));
    }
}
