//! synergos-control REST API の呼び出し。
//!
//! すべての呼び出しに `Authorization: Bearer <管理トークン>` を付ける。
//! UI は control と同一オリジンから配信されるため、パスは相対で足りる。

use gloo_net::http::{Method, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// API 呼び出しの失敗。UI がそのまま日本語で表示できる形にする。
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// 管理トークンが未設定/不一致。呼び出し側はログイン画面へ戻す。
    Unauthorized,
    /// サーバーがエラーを返した (本文の `error` を含む)。
    Server { status: u16, message: String },
    /// 通信できなかった / 応答を解釈できなかった。
    Transport(String),
}

impl ApiError {
    pub fn message(&self) -> String {
        match self {
            Self::Unauthorized => {
                "管理トークンが受け付けられませんでした (未設定または不一致)".to_string()
            }
            Self::Server { status, message } => format!("サーバーエラー ({status}): {message}"),
            Self::Transport(detail) => format!("通信エラー: {detail}"),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// 管理トークンを束ねた API クライアント。
#[derive(Clone, PartialEq)]
pub struct ApiClient {
    token: String,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiClient(***)")
    }
}

impl ApiClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.send(Method::GET, path, None::<&()>).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> ApiResult<T> {
        self.send(Method::POST, path, Some(body)).await
    }

    /// 本文の要らない POST (再発行系)。
    pub async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.send(Method::POST, path, Some(&serde_json::json!({})))
            .await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.send(Method::DELETE, path, None::<&()>).await
    }

    async fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> ApiResult<T> {
        let mut builder = RequestBuilder::new(path)
            .method(method)
            .header("Authorization", &format!("Bearer {}", self.token));

        let request = match body {
            Some(body) => {
                builder = builder.header("Content-Type", "application/json");
                builder
                    .json(body)
                    .map_err(|e| ApiError::Transport(format!("リクエストを作れません: {e}")))?
            }
            None => builder
                .build()
                .map_err(|e| ApiError::Transport(format!("リクエストを作れません: {e}")))?,
        };

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Transport(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| ApiError::Transport(format!("応答を読めません: {e}")))?;

        if status == 401 {
            return Err(ApiError::Unauthorized);
        }
        if !(200..300).contains(&status) {
            return Err(ApiError::Server {
                status,
                message: extract_error_message(&text),
            });
        }

        serde_json::from_str::<T>(&text)
            .map_err(|e| ApiError::Transport(format!("応答を解釈できません: {e}")))
    }
}

/// control のエラー応答 `{"error": "..."}` から本文を取り出す。
fn extract_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| {
            if body.is_empty() {
                "(応答本文なし)".to_string()
            } else {
                body.to_string()
            }
        })
}
