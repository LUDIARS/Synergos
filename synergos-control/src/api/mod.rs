mod heartbeat;
mod mesh_setup;
mod nodes;
mod orgs;
mod reconcile_api;
mod ui;

pub(crate) use heartbeat::{generate_node_key, hash_node_key};

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, PRAGMA};
use axum::http::HeaderValue;
use axum::middleware;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use crate::auth::{require_admin_token, AdminToken};
use crate::cloudflare::CloudflareClient;
use crate::error::{ControlError, ControlResult};
use crate::store::JsonStore;

/// ハンドラ間で共有するアプリケーション状態。
pub struct AppState {
    pub store: JsonStore,
    pub cloudflare: CloudflareClient,
    /// リクエスト限りの Cloudflare クライアントを組み立てるための設定値
    /// (秘密情報ではない)。UI からの Mesh 自動設定で使う。
    pub cf_api_base: String,
    pub cf_account_id: String,
    /// 管理 Web UI (synergos-admin-ui) のビルド成果物ディレクトリ。
    /// None なら `/ui/` は 503 を返す (API サーバー単体としては動く)。
    pub ui_dist: Option<PathBuf>,
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
        // Mesh 自動設定 (UI から Cloudflare API token をリクエストで渡す経路)。
        // 管理トークン層の内側に置く — トークンの持ち込み口を無認証にしない。
        .route("/v1/mesh/context", get(mesh_setup::mesh_context))
        .route("/v1/mesh/token-check", post(mesh_setup::check_token))
        .route("/v1/mesh/reconcile", post(mesh_setup::reconcile_with_token))
        .route(
            "/v1/mesh/connector-tokens",
            post(mesh_setup::issue_connector_tokens),
        )
        .layer(middleware::from_fn_with_state(
            admin_token,
            require_admin_token,
        ))
        .with_state(state.clone());

    // 管理 UI の静的配信。UI 自身は無認証で取得できるが、
    // 起動直後に管理トークンの入力を求め、API 呼び出しはすべて Bearer 必須。
    let ui_routes = Router::new()
        .route("/ui", get(ui::redirect_to_ui))
        .route("/ui/", get(ui::serve_index))
        .route("/ui/*ui_path", get(ui::serve_asset))
        .with_state(state.clone());

    // heartbeat はノード自身が叩くため管理トークン層の外 (node key 認証)
    Router::new()
        .route("/v1/health", get(health))
        .route(
            "/v1/heartbeat",
            post(heartbeat::receive_heartbeat).with_state(state),
        )
        .merge(admin_routes)
        .merge(ui_routes)
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
    let ip: std::net::Ipv4Addr = value
        .parse()
        .map_err(|_| ControlError::InvalidRequest("mesh_ip must be an IPv4 address".to_string()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use tower::ServiceExt;

    /// テスト用ルーター。tempdir はテスト終了まで保持する必要があるため一緒に返す。
    fn test_router() -> (Router, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("store.json")).unwrap();
        let state = Arc::new(AppState {
            store,
            cloudflare: cloudflare_client(),
            cf_api_base: API_BASE.to_string(),
            cf_account_id: "account".to_string(),
            ui_dist: None,
        });
        (build_router(state, AdminToken(admin_token())), dir)
    }

    const API_BASE: &str = "https://api.cloudflare.invalid/client/v4";

    fn cloudflare_client() -> CloudflareClient {
        CloudflareClient::new(
            API_BASE.to_string(),
            "account".to_string(),
            "env-token".to_string(),
        )
        .unwrap()
    }

    fn admin_token() -> String {
        "a".repeat(32)
    }

    async fn status_of(request: HttpRequest<Body>) -> StatusCode {
        let (router, _dir) = test_router();
        router.oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn mesh_setup_endpoints_require_the_admin_token() {
        for path in [
            "/v1/mesh/token-check",
            "/v1/mesh/reconcile",
            "/v1/mesh/connector-tokens",
        ] {
            let request = HttpRequest::post(path)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"api_token":"t","org_id":"acme"}"#))
                .unwrap();
            assert_eq!(
                status_of(request).await,
                StatusCode::UNAUTHORIZED,
                "{path} must reject unauthenticated requests"
            );
        }

        let request = HttpRequest::get("/v1/mesh/context")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn mesh_setup_endpoints_reject_a_wrong_admin_token() {
        let request = HttpRequest::post("/v1/mesh/token-check")
            .header("authorization", format!("Bearer {}", "b".repeat(32)))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"api_token":"t"}"#))
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_cloudflare_tokens_are_rejected_before_any_upstream_call() {
        // 上流ホストは解決できないため、400 が返る = 送信前に形式で弾けている。
        let request = HttpRequest::post("/v1/mesh/token-check")
            .header("authorization", format!("Bearer {}", admin_token()))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"api_token":"bad token"}"#))
            .unwrap();
        assert_eq!(status_of(request).await, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn ui_is_served_without_the_admin_token_but_reports_a_missing_build() {
        let request = HttpRequest::get("/ui/").body(Body::empty()).unwrap();
        assert_eq!(status_of(request).await, StatusCode::SERVICE_UNAVAILABLE);
    }
}
