use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::ControlError;

/// 管理 API の bearer トークン検証 middleware。
///
/// トークンは起動時に環境変数から解決済み (config::ResolvedSecrets)。
/// 定数時間比較で照合する。
pub async fn require_admin_token(
    State(expected): State<AdminToken>,
    request: Request,
    next: Next,
) -> Result<Response, ControlError> {
    let provided = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(ControlError::Unauthorized)?;

    if !constant_time_eq(provided.as_bytes(), expected.0.as_bytes()) {
        return Err(ControlError::Unauthorized);
    }
    Ok(next.run(request).await)
}

#[derive(Clone)]
pub struct AdminToken(pub String);

impl std::fmt::Debug for AdminToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdminToken(***)")
    }
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn eq_behaves() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
