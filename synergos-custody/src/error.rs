// @implements SPEC-SY-opaque-custody
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub enum Error {
    Invalid,
    Denied,
    NotFound,
    Conflict,
    Precondition,
    Unavailable,
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        tracing::warn!(kind=?error.kind(),"custody filesystem operation failed");
        Self::Unavailable
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "invalid_custody_request"),
            Self::Denied => (StatusCode::FORBIDDEN, "custody_access_denied"),
            Self::NotFound => (StatusCode::NOT_FOUND, "custody_object_not_found"),
            Self::Conflict => (StatusCode::CONFLICT, "custody_operation_conflict"),
            Self::Precondition => (StatusCode::PRECONDITION_FAILED, "custody_object_exists"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "custody_unavailable"),
        };
        (status, Json(serde_json::json!({"error":code}))).into_response()
    }
}
