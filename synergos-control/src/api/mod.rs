mod heartbeat;
mod nodes;
mod orgs;
mod reconcile_api;

pub(crate) use heartbeat::{generate_node_key, hash_node_key};

use std::sync::Arc;

use axum::middleware;
use axum::routing::{get, post};
use axum::extract::DefaultBodyLimit;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, PRAGMA};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;

use crate::auth::{require_admin_token, AdminToken};
use crate::cloudflare::CloudflareClient;
use crate::error::{ControlError, ControlResult};
use crate::store::JsonStore;

/// ハンドラ間で共有するアプリケーション状態。
pub struct AppState {
    pub store: JsonStore,
    pub cloudflare: CloudflareClient,
}

pub fn build_router(state: Arc<AppState>, admin_token: AdminToken) -> Router {
    let admin_routes = Router::new()
        .route("/v1/orgs", post(orgs::create_org).get(orgs::list_orgs))
        .route("/v1/orgs/:org_id", get(orgs::get_org).put(orgs::update_org))
        .route(
            "/v1/orgs/:org_id/nodes",
            post(nodes::register_node).get(nodes::list_nodes),
        )
        .route(
            "/v1/orgs/:org_id/nodes/:node_id",
            get(nodes::get_node)
                .patch(nodes::patch_node)
                .delete(nodes::remove_node),
        )
        .route(
            "/v1/orgs/:org_id/nodes/:node_id/connector-token",
            post(nodes::reissue_connector_token),
        )
        .route(
            "/v1/orgs/:org_id/nodes/:node_id/node-key",
            post(nodes::reissue_node_key),
        )
        .route("/v1/reconcile", post(reconcile_api::run_reconcile))
        .layer(middleware::from_fn_with_state(
            admin_token,
            require_admin_token,
        ))
        .with_state(state.clone());

    // heartbeat はノード自身が叩くため管理トークン層の外 (node key 認証)
    Router::new()
        .route("/v1/health", get(health))
        .route(
            "/v1/heartbeat",
            post(heartbeat::receive_heartbeat).with_state(state),
        )
        .merge(admin_routes)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(disable_response_caching))
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ok": true,
        "service": "synergos-control",
    }))
}

fn normalize_mesh_ip(value: &str) -> ControlResult<String> {
    let ip: std::net::Ipv4Addr = value.parse().map_err(|_| {
        ControlError::InvalidRequest("mesh_ip must be an IPv4 address".to_string())
    })?;
    let octets = ip.octets();
    if octets[0] != 100 || !(96..=111).contains(&octets[1]) {
        return Err(ControlError::InvalidRequest(
            "mesh_ip must be in Cloudflare Mesh range 100.96.0.0/12".to_string(),
        ));
    }
    Ok(ip.to_string())
}

async fn disable_response_caching(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(PRAGMA, HeaderValue::from_static("no-cache"));
    response
}
