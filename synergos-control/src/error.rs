use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// synergos-control 全体のエラー型。
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
    #[error("config error: {0}")]
    Config(String),

    #[error("store error: {0}")]
    Store(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("cloudflare api error: {0}")]
    Cloudflare(String),

    #[error("unauthorized")]
    Unauthorized,
}

impl ControlError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Config(_) | Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            // upstream (Cloudflare) の失敗はこちらの内部エラーと区別する
            Self::Cloudflare(_) => StatusCode::BAD_GATEWAY,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
        }
    }
}

impl IntoResponse for ControlError {
    fn into_response(self) -> Response {
        let status = self.status();
        // 内部エラーの詳細はローカルパスや upstream 情報を含むためクライアントに返さない。
        // 詳細はサーバログにのみ残す。それ以外は運用者が原因を特定できるようそのまま返す。
        let message = match &self {
            Self::Config(_) | Self::Store(_) | Self::Cloudflare(_) => {
                tracing::error!(error = %self, "internal error");
                if matches!(&self, Self::Cloudflare(_)) {
                    "upstream service error".to_string()
                } else {
                    "internal error".to_string()
                }
            }
            other => other.to_string(),
        };
        let body = Json(serde_json::json!({ "error": message }));
        (status, body).into_response()
    }
}

pub type ControlResult<T> = Result<T, ControlError>;
