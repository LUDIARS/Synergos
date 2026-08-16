use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::{ControlError, ControlResult};
use crate::store::{now_ms, Node, NodeKind};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterNodeRequest {
    pub display_name: String,
    pub owner_email: String,
    pub kind: NodeKind,
    /// ClientDevice で既知の場合のみ。MeshNode は自動採番。
    #[serde(default)]
    pub synergos_peer_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PatchNodeRequest {
    pub display_name: Option<String>,
    pub owner_email: Option<String>,
    pub synergos_peer_id: Option<String>,
    pub mesh_ip: Option<String>,
}

/// 登録レスポンス。connector_token / node_key は登録時に一度だけ返す
/// (レジストリには保存しない — node_key はハッシュのみ永続化)。
#[derive(Serialize)]
pub struct RegisterNodeResponse {
    pub node: NodeView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_token: Option<String>,
    /// daemon の heartbeat 認証キー。`SYNERGOS_NODE_KEY` としてノードに配布する。
    pub node_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enroll_hint: Option<String>,
}

/// API 表示用ノード。永続化専用の `node_key_hash` は応答へ含めない。
#[derive(Debug, Serialize)]
pub struct NodeView {
    pub id: String,
    pub org_id: String,
    pub display_name: String,
    pub owner_email: String,
    pub kind: NodeKind,
    pub cf_connector_id: Option<String>,
    pub synergos_peer_id: Option<String>,
    pub mesh_ip: Option<String>,
    pub reported_mesh_ip: Option<String>,
    pub last_heartbeat_ms: Option<u64>,
    pub synergos_version: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl From<Node> for NodeView {
    fn from(node: Node) -> Self {
        Self {
            id: node.id,
            org_id: node.org_id,
            display_name: node.display_name,
            owner_email: node.owner_email,
            kind: node.kind,
            cf_connector_id: node.cf_connector_id,
            synergos_peer_id: node.synergos_peer_id,
            mesh_ip: node.mesh_ip,
            reported_mesh_ip: node.reported_mesh_ip,
            last_heartbeat_ms: node.last_heartbeat_ms,
            synergos_version: node.synergos_version,
            created_at_ms: node.created_at_ms,
            updated_at_ms: node.updated_at_ms,
        }
    }
}

/// ノード登録。MeshNode の場合は Cloudflare 側に connector を自動作成し、
/// `warp-cli connector new <TOKEN>` 用のトークンを返す (CF 設定の自動化)。
pub async fn register_node(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
    Json(req): Json<RegisterNodeRequest>,
) -> ControlResult<Json<RegisterNodeResponse>> {
    let org = state.store.get_org(&org_id).await?;
    let owner_email = req.owner_email.trim().to_ascii_lowercase();
    if !owner_email.contains('@') {
        return Err(ControlError::InvalidRequest(
            "owner_email must be an email address".to_string(),
        ));
    }
    if !org.members.iter().any(|m| m == &owner_email) {
        return Err(ControlError::InvalidRequest(format!(
            "owner {owner_email} is not a member of org {org_id}; add them to org members first"
        )));
    }

    let node_id = uuid::Uuid::new_v4().to_string();
    let node_key = super::generate_node_key();
    let mut node = Node {
        id: node_id,
        org_id: org_id.clone(),
        display_name: req.display_name,
        owner_email,
        kind: req.kind,
        cf_connector_id: None,
        synergos_peer_id: req.synergos_peer_id,
        mesh_ip: None,
        reported_mesh_ip: None,
        node_key_hash: Some(super::hash_node_key(&node_key)),
        last_heartbeat_ms: None,
        synergos_version: None,
        created_at_ms: now_ms(),
        updated_at_ms: now_ms(),
    };

    let mut connector_token = None;
    let enroll_hint;
    match req.kind {
        NodeKind::MeshNode => {
            // Cloudflare 名は "syn-<org>-<node短縮id>" で機械的に決め、突合可能にする
            let cf_name = format!("syn-{org_id}-{}", &node.id[..8]);
            let connector = state.cloudflare.create_mesh_connector(&cf_name).await?;
            let token = match state.cloudflare.mesh_connector_token(&connector.id).await {
                Ok(token) => token,
                Err(err) => {
                    cleanup_connector(&state, &connector.id, "token retrieval failure").await;
                    return Err(err);
                }
            };
            info!(org = %org_id, node = %node.id, connector = %connector.id, "created mesh connector");
            node.cf_connector_id = Some(connector.id);
            connector_token = Some(token);
            enroll_hint = Some(
                "run on the node: sudo warp-cli connector new <connector_token> && sudo warp-cli connect"
                    .to_string(),
            );
        }
        NodeKind::ClientDevice => {
            enroll_hint = Some(
                "enroll via Cloudflare One Client with the org team name; \
                 reconcile matches the device by owner_email"
                    .to_string(),
            );
        }
    }

    // レジストリ書き込みに失敗したら作成済み connector を残さない (リソース寿命)
    let created_connector_id = node.cf_connector_id.clone();
    let node = match state.store.insert_node(node).await {
        Ok(node) => node,
        Err(err) => {
            if let Some(connector_id) = created_connector_id {
                cleanup_connector(&state, &connector_id, "store failure").await;
            }
            return Err(err);
        }
    };

    Ok(Json(RegisterNodeResponse {
        node: node.into(),
        connector_token,
        node_key,
        enroll_hint,
    }))
}

pub async fn list_nodes(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
) -> ControlResult<Json<Vec<NodeView>>> {
    state.store.get_org(&org_id).await?;
    Ok(Json(
        state
            .store
            .list_nodes(&org_id)
            .await
            .into_iter()
            .map(NodeView::from)
            .collect(),
    ))
}

pub async fn get_node(
    State(state): State<Arc<AppState>>,
    Path((org_id, node_id)): Path<(String, String)>,
) -> ControlResult<Json<NodeView>> {
    Ok(Json(state.store.get_node(&org_id, &node_id).await?.into()))
}

pub async fn patch_node(
    State(state): State<Arc<AppState>>,
    Path((org_id, node_id)): Path<(String, String)>,
    Json(req): Json<PatchNodeRequest>,
) -> ControlResult<Json<NodeView>> {
    let PatchNodeRequest {
        display_name,
        owner_email: requested_owner_email,
        synergos_peer_id,
        mesh_ip,
    } = req;
    // 存在確認と owner の所属検証はロックを取る前に済ませる
    // (mutate_node のクロージャからストアを呼ぶとデッドロックする)。
    state.store.get_node(&org_id, &node_id).await?;
    let owner_email = match requested_owner_email {
        Some(owner_email) => {
            let owner_email = owner_email.trim().to_ascii_lowercase();
            let org = state.store.get_org(&org_id).await?;
            if !org.members.iter().any(|m| m == &owner_email) {
                return Err(ControlError::InvalidRequest(format!(
                    "owner {owner_email} is not a member of org {org_id}"
                )));
            }
            Some(owner_email)
        }
        None => None,
    };
    let mesh_ip = mesh_ip
        .as_deref()
        .map(super::normalize_mesh_ip)
        .transpose()?;

    let updated = state
        .store
        .mutate_node(Some(&org_id), &node_id, |node| {
            if let Some(display_name) = display_name {
                node.display_name = display_name;
            }
            if let Some(owner_email) = owner_email {
                node.owner_email = owner_email;
            }
            if let Some(peer_id) = synergos_peer_id {
                node.synergos_peer_id = Some(peer_id);
            }
            if let Some(mesh_ip) = mesh_ip {
                node.mesh_ip = Some(mesh_ip);
            }
            node.updated_at_ms = now_ms();
        })
        .await?;
    Ok(Json(updated.into()))
}

/// ノード削除。MeshNode は Cloudflare 側 connector も削除する (失効)。
pub async fn remove_node(
    State(state): State<Arc<AppState>>,
    Path((org_id, node_id)): Path<(String, String)>,
) -> ControlResult<Json<NodeView>> {
    let node = state.store.get_node(&org_id, &node_id).await?;
    if let Some(connector_id) = &node.cf_connector_id {
        state.cloudflare.delete_mesh_connector(connector_id).await?;
        info!(org = %org_id, node = %node_id, connector = %connector_id, "deleted mesh connector");
    }
    Ok(Json(
        state.store.remove_node(&org_id, &node_id).await?.into(),
    ))
}

/// MeshNode の登録トークン再発行 (ノード再セットアップ時)。
pub async fn reissue_connector_token(
    State(state): State<Arc<AppState>>,
    Path((org_id, node_id)): Path<(String, String)>,
) -> ControlResult<Json<serde_json::Value>> {
    let node = state.store.get_node(&org_id, &node_id).await?;
    let connector_id = node.cf_connector_id.as_deref().ok_or_else(|| {
        ControlError::InvalidRequest(
            "node has no mesh connector (client devices enroll via Cloudflare One Client)"
                .to_string(),
        )
    })?;
    let token = state.cloudflare.mesh_connector_token(connector_id).await?;
    Ok(Json(serde_json::json!({ "connector_token": token })))
}

/// heartbeat 用 node key の再発行 (旧キーは即無効)。
pub async fn reissue_node_key(
    State(state): State<Arc<AppState>>,
    Path((org_id, node_id)): Path<(String, String)>,
) -> ControlResult<Json<serde_json::Value>> {
    let node_key = super::generate_node_key();
    let node_key_hash = super::hash_node_key(&node_key);
    state
        .store
        .mutate_node(Some(&org_id), &node_id, |node| {
            node.node_key_hash = Some(node_key_hash);
            node.updated_at_ms = now_ms();
        })
        .await?;
    info!(org = %org_id, node = %node_id, "node key reissued");
    Ok(Json(serde_json::json!({ "node_key": node_key })))
}

async fn cleanup_connector(state: &AppState, connector_id: &str, reason: &str) {
    if let Err(cleanup_err) = state.cloudflare.delete_mesh_connector(connector_id).await {
        warn!(
            %connector_id,
            error = %cleanup_err,
            %reason,
            "failed to clean up connector"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloudflare::CloudflareClient;
    use crate::store::{JsonStore, Org};
    use axum::routing::{delete, get, post};
    use axum::Router;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn create_connector() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "success": true,
            "errors": [],
            "result": { "id": "cf-1", "name": "syn-acme-test" }
        }))
    }

    async fn fail_token() -> (axum::http::StatusCode, Json<serde_json::Value>) {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "success": false,
                "errors": [{ "code": 1000, "message": "token unavailable" }],
                "result": null
            })),
        )
    }

    async fn delete_connector(
        State(delete_count): State<Arc<AtomicUsize>>,
    ) -> Json<serde_json::Value> {
        delete_count.fetch_add(1, Ordering::SeqCst);
        Json(serde_json::json!({
            "success": true,
            "errors": [],
            "result": { "id": "cf-1", "name": "syn-acme-test" }
        }))
    }

    #[tokio::test]
    async fn token_failure_deletes_new_connector_and_does_not_persist_node() {
        let delete_count = Arc::new(AtomicUsize::new(0));
        let router = Router::new()
            .route(
                "/client/v4/accounts/account/warp_connector",
                post(create_connector),
            )
            .route(
                "/client/v4/accounts/account/warp_connector/cf-1/token",
                get(fail_token),
            )
            .route(
                "/client/v4/accounts/account/warp_connector/cf-1",
                delete(delete_connector),
            )
            .with_state(delete_count.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("store.json")).unwrap();
        store
            .insert_org(Org {
                id: "acme".to_string(),
                name: "Acme".to_string(),
                members: vec!["alice@acme.test".to_string()],
                created_at_ms: 0,
            })
            .await
            .unwrap();
        let state = Arc::new(AppState {
            store,
            cloudflare: CloudflareClient::new(
                format!("http://{addr}/client/v4"),
                "account".to_string(),
                "token".to_string(),
            )
            .unwrap(),
        });

        let result = register_node(
            State(state.clone()),
            Path("acme".to_string()),
            Json(RegisterNodeRequest {
                display_name: "test".to_string(),
                owner_email: "alice@acme.test".to_string(),
                kind: NodeKind::MeshNode,
                synergos_peer_id: None,
            }),
        )
        .await;

        assert!(matches!(result, Err(ControlError::Cloudflare(_))));
        assert_eq!(delete_count.load(Ordering::SeqCst), 1);
        assert!(state.store.list_nodes("acme").await.is_empty());
        task.abort();
    }

    #[test]
    fn node_view_does_not_serialize_key_hash() {
        let node = Node {
            id: "n1".to_string(),
            org_id: "acme".to_string(),
            display_name: "node".to_string(),
            owner_email: "alice@acme.test".to_string(),
            kind: NodeKind::MeshNode,
            cf_connector_id: None,
            synergos_peer_id: None,
            mesh_ip: None,
            reported_mesh_ip: None,
            node_key_hash: Some("sensitive-hash".to_string()),
            last_heartbeat_ms: None,
            synergos_version: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        };

        let value = serde_json::to_value(NodeView::from(node)).unwrap();
        assert!(value.get("node_key_hash").is_none());
    }
}
