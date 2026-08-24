//! synergos-control API のクライアント層。

mod client;
mod models;
mod paths;

pub use client::{ApiClient, ApiError, ApiResult};
pub use models::*;

use paths as p;

impl ApiClient {
    // --- 組織 ---

    pub async fn list_orgs(&self) -> ApiResult<Vec<Org>> {
        self.get(p::ORGS).await
    }

    pub async fn create_org(&self, req: &CreateOrgRequest) -> ApiResult<Org> {
        self.post(p::ORGS, req).await
    }

    // --- ノード ---

    pub async fn list_nodes(&self, org_id: &str) -> ApiResult<Vec<NodeView>> {
        self.get(&p::org_nodes(org_id)).await
    }

    pub async fn register_node(
        &self,
        org_id: &str,
        req: &RegisterNodeRequest,
    ) -> ApiResult<RegisterNodeResponse> {
        self.post(&p::org_nodes(org_id), req).await
    }

    pub async fn remove_node(&self, org_id: &str, node_id: &str) -> ApiResult<NodeView> {
        self.delete(&p::node(org_id, node_id)).await
    }

    pub async fn reissue_connector_token(
        &self,
        org_id: &str,
        node_id: &str,
    ) -> ApiResult<ConnectorTokenResponse> {
        self.post_empty(&p::node_connector_token(org_id, node_id))
            .await
    }

    // --- 突合 (起動時 env の Cloudflare トークンを使う) ---

    pub async fn reconcile(&self) -> ApiResult<ReconcileReport> {
        self.post(p::RECONCILE, &serde_json::json!({ "revoke_dark": false }))
            .await
    }

    // --- Mesh 自動設定 (リクエストで渡す一時トークンを使う) ---

    pub async fn mesh_context(&self) -> ApiResult<MeshContext> {
        self.get(p::MESH_CONTEXT).await
    }

    pub async fn mesh_check_token(&self, api_token: &str) -> ApiResult<TokenCheckResponse> {
        self.post(
            p::MESH_TOKEN_CHECK,
            &TokenCheckRequest {
                api_token: api_token.to_string(),
            },
        )
        .await
    }

    pub async fn mesh_reconcile(&self, api_token: &str) -> ApiResult<ReconcileReport> {
        self.post(
            p::MESH_RECONCILE,
            &MeshReconcileRequest {
                api_token: api_token.to_string(),
                // UI からの自動設定では破壊的操作を行わない (検出まで)。
                revoke_dark: false,
            },
        )
        .await
    }

    pub async fn mesh_connector_tokens(
        &self,
        api_token: &str,
        org_id: &str,
    ) -> ApiResult<ConnectorTokensResponse> {
        self.post(
            p::MESH_CONNECTOR_TOKENS,
            &ConnectorTokensRequest {
                api_token: api_token.to_string(),
                org_id: org_id.to_string(),
            },
        )
        .await
    }
}
