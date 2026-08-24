use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use crate::error::{ControlError, ControlResult};

use super::{DeviceRegistration, MeshConnector, TokenStatus};

/// Cloudflare API v4 の薄いクライアント。
///
/// Mesh node は API 上 `warp_connector` として表現される
/// (https://developers.cloudflare.com/api/resources/zero_trust/subresources/tunnels/subresources/warp_connector/)。
/// デバイスは `devices/registrations` (旧 `devices` API は deprecated)。
pub struct CloudflareClient {
    http: reqwest::Client,
    api_base: String,
    account_id: String,
    api_token: String,
}

/// Cloudflare API v4 の共通レスポンスエンベロープ。
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

impl CloudflareClient {
    pub fn new(api_base: String, account_id: String, api_token: String) -> ControlResult<Self> {
        validate_api_base(&api_base)?;
        validate_path_segment("account_id", &account_id)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("synergos-control/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ControlError::Cloudflare(format!("http client init: {e}")))?;
        Ok(Self {
            http,
            api_base: api_base.trim_end_matches('/').to_string(),
            account_id,
            api_token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/accounts/{}/{path}", self.api_base, self.account_id)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> ControlResult<T> {
        self.call_optional(method.clone(), path, &[], body, false)
            .await?
            .ok_or_else(|| {
                ControlError::Cloudflare(format!("{method} {path}: success but empty result"))
            })
    }

    async fn call_optional<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<serde_json::Value>,
        not_found_is_success: bool,
    ) -> ControlResult<Option<T>> {
        let mut req = self
            .http
            .request(method.clone(), self.url(path))
            .bearer_auth(&self.api_token)
            .query(query);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ControlError::Cloudflare(format!("{method} {path}: {e}")))?;
        let status = resp.status();
        if not_found_is_success && status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let envelope: Envelope<T> = resp.json().await.map_err(|e| {
            ControlError::Cloudflare(format!("{method} {path}: invalid response ({status}): {e}"))
        })?;
        if !envelope.success {
            let detail = format_api_errors(&envelope.errors);
            return Err(ControlError::Cloudflare(format!(
                "{method} {path} failed ({status}): {detail}"
            )));
        }
        Ok(envelope.result)
    }

    /// API token 自体の有効性を確認する。
    /// account 配下ではないエンドポイントなので `url()` を経由しない。
    ///
    /// UI から渡される一時トークンを、副作用のある操作より前に弾くために使う。
    pub async fn verify_token(&self) -> ControlResult<TokenStatus> {
        let url = format!("{}/user/tokens/verify", self.api_base);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.api_token)
            .send()
            .await
            .map_err(|e| ControlError::Cloudflare(format!("GET user/tokens/verify: {e}")))?;
        let status = resp.status();
        let envelope: Envelope<TokenStatus> = resp.json().await.map_err(|e| {
            ControlError::Cloudflare(format!(
                "GET user/tokens/verify: invalid response ({status}): {e}"
            ))
        })?;
        if !envelope.success {
            return Err(ControlError::Cloudflare(format!(
                "GET user/tokens/verify failed ({status}): {}",
                format_api_errors(&envelope.errors)
            )));
        }
        envelope.result.ok_or_else(|| {
            ControlError::Cloudflare("GET user/tokens/verify: success but empty result".to_string())
        })
    }

    // --- Mesh node (warp_connector) ---

    /// Mesh node を作成し、Cloudflare 側 ID を返す。
    pub async fn create_mesh_connector(&self, name: &str) -> ControlResult<MeshConnector> {
        self.call(
            reqwest::Method::POST,
            "warp_connector",
            Some(json!({ "name": name })),
        )
        .await
    }

    /// 有効な Mesh node の一覧。
    pub async fn list_mesh_connectors(&self) -> ControlResult<Vec<MeshConnector>> {
        let connectors: Vec<MeshConnector> = self
            .call(
                reqwest::Method::GET,
                "warp_connector?is_deleted=false&per_page=1000",
                None,
            )
            .await?;
        Ok(connectors)
    }

    /// `warp-cli connector new <TOKEN>` に渡すノード登録トークンを取得する。
    pub async fn mesh_connector_token(&self, connector_id: &str) -> ControlResult<String> {
        validate_path_segment("connector_id", connector_id)?;
        self.call(
            reqwest::Method::GET,
            &format!("warp_connector/{connector_id}/token"),
            None,
        )
        .await
    }

    pub async fn delete_mesh_connector(&self, connector_id: &str) -> ControlResult<()> {
        validate_path_segment("connector_id", connector_id)?;
        // DELETE は再試行可能にする。前回 Cloudflare 側だけ成功した場合の 404 は成功扱い。
        let _: Option<MeshConnector> = self
            .call_optional(
                reqwest::Method::DELETE,
                &format!("warp_connector/{connector_id}"),
                &[],
                None,
                true,
            )
            .await?;
        Ok(())
    }

    // --- Device registrations ---

    /// WARP デバイス登録の一覧 (人の端末)。
    pub async fn list_device_registrations(&self) -> ControlResult<Vec<DeviceRegistration>> {
        #[derive(Debug, Deserialize)]
        struct RawRegistration {
            id: String,
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            user: Option<RawUser>,
            #[serde(default)]
            virtual_ipv4: Option<String>,
            #[serde(default)]
            last_seen_at: Option<String>,
            #[serde(default)]
            revoked_at: Option<String>,
        }
        #[derive(Debug, Deserialize)]
        struct RawUser {
            #[serde(default)]
            email: Option<String>,
        }

        let raw: Vec<RawRegistration> = self
            .call(
                reqwest::Method::GET,
                "devices/registrations?per_page=1000",
                None,
            )
            .await?;
        Ok(raw
            .into_iter()
            .map(|r| DeviceRegistration {
                id: r.id,
                name: r.name,
                user_email: r.user.and_then(|u| u.email),
                virtual_ipv4: r.virtual_ipv4,
                last_seen_at: r.last_seen_at,
                revoked_at: r.revoked_at,
            })
            .collect())
    }

    /// デバイス登録を失効させる (dark node の排除)。
    pub async fn revoke_device_registrations(&self, ids: &[String]) -> ControlResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let query: Vec<(&str, &str)> = ids.iter().map(|id| ("id", id.as_str())).collect();
        // API は JSON body ではなく、繰り返しの `id` query parameter を要求する。
        // result は null の場合があるため空でも成功として扱う。
        let _: Option<serde_json::Value> = self
            .call_optional(
                reqwest::Method::POST,
                "devices/registrations/revoke",
                &query,
                None,
                false,
            )
            .await?;
        Ok(())
    }
}

fn format_api_errors(errors: &[ApiError]) -> String {
    errors
        .iter()
        .map(|e| format!("{}:{}", e.code, e.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn validate_api_base(api_base: &str) -> ControlResult<()> {
    let url = reqwest::Url::parse(api_base)
        .map_err(|e| ControlError::Config(format!("invalid cloudflare.api_base: {e}")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ControlError::Config(
            "cloudflare.api_base must not contain user credentials".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ControlError::Config(
            "cloudflare.api_base must not contain a query or fragment".to_string(),
        ));
    }
    if url.scheme() == "https" {
        return Ok(());
    }
    let is_loopback_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if is_loopback_http {
        Ok(())
    } else {
        Err(ControlError::Config(
            "cloudflare.api_base must use HTTPS (HTTP is allowed only for loopback tests)"
                .to_string(),
        ))
    }
}

fn validate_path_segment(label: &str, value: &str) -> ControlResult<()> {
    let valid = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(ControlError::Config(format!(
            "{label} contains characters that are unsafe in an API path"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{StatusCode, Uri};
    use axum::routing::{delete, post};
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CapturedRequest {
        query: Option<String>,
        body_len: usize,
    }

    async fn capture_revoke(
        State(captured): State<Arc<Mutex<CapturedRequest>>>,
        uri: Uri,
        body: Bytes,
    ) -> Json<serde_json::Value> {
        let mut captured = captured.lock().unwrap();
        captured.query = uri.query().map(str::to_string);
        captured.body_len = body.len();
        Json(json!({ "success": true, "errors": [], "result": null }))
    }

    async fn not_found() -> StatusCode {
        StatusCode::NOT_FOUND
    }

    async fn test_client(router: Router) -> (CloudflareClient, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = CloudflareClient::new(
            format!("http://{addr}/client/v4"),
            "account".to_string(),
            "token".to_string(),
        )
        .unwrap();
        (client, task)
    }

    #[tokio::test]
    async fn revoke_devices_uses_repeated_query_parameters_without_json_body() {
        let captured = Arc::new(Mutex::new(CapturedRequest::default()));
        let router = Router::new()
            .route(
                "/client/v4/accounts/account/devices/registrations/revoke",
                post(capture_revoke),
            )
            .with_state(captured.clone());
        let (client, task) = test_client(router).await;

        client
            .revoke_device_registrations(&["device-a".to_string(), "device-b".to_string()])
            .await
            .unwrap();

        let captured = captured.lock().unwrap();
        assert_eq!(captured.query.as_deref(), Some("id=device-a&id=device-b"));
        assert_eq!(captured.body_len, 0);
        task.abort();
    }

    #[tokio::test]
    async fn deleting_an_already_absent_connector_is_successful() {
        let router = Router::new().route(
            "/client/v4/accounts/account/warp_connector/missing",
            delete(not_found),
        );
        let (client, task) = test_client(router).await;

        client.delete_mesh_connector("missing").await.unwrap();

        task.abort();
    }
}
