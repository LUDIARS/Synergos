use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn};

use crate::cloudflare::CloudflareClient;
use crate::error::{ControlError, ControlResult};
use crate::reconcile::ReconcileReport;
use crate::store::NodeKind;

use super::reconcile_api::reconcile_with;
use super::AppState;

/// リクエストで受け取る Cloudflare API token。
///
/// **保存もログ出力もしない。** そのリクエストの処理中だけ使い、応答にも含めない。
/// 値が誤ってログへ出ないよう `Debug` は導出せず、伏字を出す実装を与える。
#[derive(Deserialize)]
pub struct RequestScopedToken {
    pub api_token: String,
}

impl std::fmt::Debug for RequestScopedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestScopedToken(***)")
    }
}

#[derive(Deserialize)]
pub struct MeshReconcileRequest {
    pub api_token: String,
    #[serde(default)]
    pub revoke_dark: bool,
}

impl std::fmt::Debug for MeshReconcileRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshReconcileRequest")
            .field("revoke_dark", &self.revoke_dark)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
pub struct ConnectorTokensRequest {
    pub api_token: String,
    pub org_id: String,
}

impl std::fmt::Debug for ConnectorTokensRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorTokensRequest")
            .field("org_id", &self.org_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub struct TokenCheckResponse {
    /// Cloudflare が返すトークン状態 ("active" なら利用可)。
    pub token_status: String,
    pub expires_on: Option<String>,
    /// 対象アカウント (control の設定値)。UI が「どのアカウントに繋がるか」を示す。
    pub account_id: String,
    /// トークンでアカウント配下の Mesh node を読めたかの確認結果。
    pub mesh_node_count: usize,
}

#[derive(Serialize)]
pub struct ConnectorTokenEntry {
    pub node_id: String,
    pub display_name: String,
    /// MeshNode のみ。ClientDevice や connector 未作成のノードは None。
    pub connector_token: Option<String>,
    /// トークンを発行しなかった理由 (UI 表示用)。
    pub skipped_reason: Option<String>,
    pub enroll_command: Option<String>,
}

#[derive(Serialize)]
pub struct ConnectorTokensResponse {
    pub org_id: String,
    pub issued: usize,
    pub entries: Vec<ConnectorTokenEntry>,
}

/// Mesh 自動設定 step 1: 受け取ったトークンの有効性とアカウント到達性を確認する。
pub async fn check_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RequestScopedToken>,
) -> ControlResult<Json<TokenCheckResponse>> {
    let client = state.request_scoped_cloudflare(req.api_token)?;
    let status = client.verify_token().await?;
    if status.status != "active" {
        return Err(ControlError::InvalidRequest(
            "cloudflare api token is not active".to_string(),
        ));
    }
    // status が active でも権限が足りない場合があるため、実際に一覧を引いて確認する。
    let connectors = client.list_mesh_connectors().await?;
    info!(
        mesh_node_count = connectors.len(),
        "request-scoped cloudflare token verified"
    );
    Ok(Json(TokenCheckResponse {
        token_status: status.status,
        expires_on: status.expires_on,
        account_id: state.cf_account_id.clone(),
        mesh_node_count: connectors.len(),
    }))
}

/// Mesh 自動設定 step 2: 受け取ったトークンでレジストリと Cloudflare を突合する。
pub async fn reconcile_with_token(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MeshReconcileRequest>,
) -> ControlResult<Json<ReconcileReport>> {
    let MeshReconcileRequest {
        api_token,
        revoke_dark,
    } = req;
    let client = state.request_scoped_cloudflare(api_token)?;
    Ok(Json(reconcile_with(&state, &client, revoke_dark).await?))
}

/// Mesh 自動設定 step 3: 組織の Mesh node へ配る登録トークンをまとめて発行する。
///
/// 発行したトークンは応答で一度返すだけで保存しない (既存のノード登録と同じ扱い)。
pub async fn issue_connector_tokens(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ConnectorTokensRequest>,
) -> ControlResult<Json<ConnectorTokensResponse>> {
    let ConnectorTokensRequest { api_token, org_id } = req;
    let client = state.request_scoped_cloudflare(api_token)?;

    state.store.get_org(&org_id).await?;
    let nodes = state.store.list_nodes(&org_id).await;

    let mut entries = Vec::with_capacity(nodes.len());
    let mut issued = 0usize;
    for node in nodes {
        if node.kind != NodeKind::MeshNode {
            entries.push(skipped_entry(
                &node.id,
                &node.display_name,
                "client device は Cloudflare One Client でエンロールするため登録トークンは不要",
            ));
            continue;
        }
        let Some(connector_id) = node.cf_connector_id.as_deref() else {
            entries.push(skipped_entry(
                &node.id,
                &node.display_name,
                "Cloudflare connector が未作成 (ノードを登録し直してください)",
            ));
            continue;
        };
        // 1 ノードの失敗で全体を止めず、そのノードだけ理由付きで落とす。
        match client.mesh_connector_token(connector_id).await {
            Ok(token) => {
                issued += 1;
                entries.push(ConnectorTokenEntry {
                    node_id: node.id.clone(),
                    display_name: node.display_name.clone(),
                    enroll_command: Some(format!(
                        "sudo warp-cli connector new {token} && sudo warp-cli connect"
                    )),
                    connector_token: Some(token),
                    skipped_reason: None,
                });
            }
            Err(err) => {
                warn!(node = %node.id, error = %err, "failed to issue connector token");
                entries.push(skipped_entry(
                    &node.id,
                    &node.display_name,
                    "Cloudflare からトークンを取得できません",
                ));
            }
        }
    }

    info!(org = %org_id, issued, "issued connector tokens via mesh setup");
    Ok(Json(ConnectorTokensResponse {
        org_id,
        issued,
        entries,
    }))
}

fn skipped_entry(node_id: &str, display_name: &str, reason: &str) -> ConnectorTokenEntry {
    ConnectorTokenEntry {
        node_id: node_id.to_string(),
        display_name: display_name.to_string(),
        connector_token: None,
        skipped_reason: Some(reason.to_string()),
        enroll_command: None,
    }
}

/// UI が Mesh 設定の前提 (どのアカウントへ繋ぐか) を読むための情報。
/// 秘密情報は含めない。
pub async fn mesh_context(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "account_id": state.cf_account_id,
        "api_base": state.cf_api_base,
    }))
}

impl AppState {
    /// リクエストで渡されたトークンから、そのリクエスト限りのクライアントを作る。
    pub(super) fn request_scoped_cloudflare(
        &self,
        api_token: String,
    ) -> ControlResult<CloudflareClient> {
        validate_request_token(&api_token)?;
        CloudflareClient::new(
            self.cf_api_base.clone(),
            self.cf_account_id.clone(),
            api_token,
        )
    }
}

/// HTTP ヘッダに載せる前に形式を弾く (制御文字混入・空・過長を拒否)。
fn validate_request_token(api_token: &str) -> ControlResult<()> {
    if api_token.is_empty() || api_token.len() > 512 {
        return Err(ControlError::InvalidRequest(
            "api_token must be 1..=512 bytes".to_string(),
        ));
    }
    if !api_token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ControlError::InvalidRequest(
            "api_token contains characters that are not allowed in an HTTP header".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_with_control_characters_are_rejected() {
        assert!(validate_request_token("abc\r\nX-Injected: 1").is_err());
        assert!(validate_request_token("abc def").is_err());
        assert!(validate_request_token("").is_err());
        assert!(validate_request_token(&"a".repeat(513)).is_err());
    }

    #[test]
    fn plausible_cloudflare_tokens_are_accepted() {
        assert!(validate_request_token("v1.0-abcDEF_123.456").is_ok());
    }

    #[test]
    fn token_bearing_requests_never_debug_print_the_token() {
        let req = RequestScopedToken {
            api_token: "super-secret".to_string(),
        };
        assert!(!format!("{req:?}").contains("super-secret"));

        let reconcile = MeshReconcileRequest {
            api_token: "super-secret".to_string(),
            revoke_dark: true,
        };
        assert!(!format!("{reconcile:?}").contains("super-secret"));

        let tokens = ConnectorTokensRequest {
            api_token: "super-secret".to_string(),
            org_id: "acme".to_string(),
        };
        let rendered = format!("{tokens:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("acme"));
    }
}
