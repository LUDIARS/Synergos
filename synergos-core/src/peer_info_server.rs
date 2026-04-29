//! Peer-info HTTP servlet (bootstrap endpoint)
//!
//! Cloudflare Tunnel 等の HTTPS 公開を通じてピアが「この Synergos ノードへ
//! 直接 QUIC 接続するための情報」を JSON で配信する小さな HTTP サーバ。
//!
//! ## 設計目的
//!
//! 1. Cloudflare AAAA を直接公開しない (= IPv6 を撒かない) アーキテクチャの
//!    実現。Tunnel は HTTPS を proxy するので、EC2 の生 IPv6 はクライアントが
//!    `/peer-info` を GET したときだけ JSON 応答に乗る。
//! 2. 将来的な認証 / API 拡張のホスト。現状はテストのため認証なし。Tower 経由
//!    の middleware で auth layer を後付けする想定。
//! 3. invite token 機構の代替経路 (peer add-url の bootstrap データ源)。
//!
//! ## エンドポイント (現状)
//!
//! - `GET /peer-info` → `PeerInfoResponse` (JSON)
//! - `GET /health` → 200 OK (`"ok"`)

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::Json, routing::get, Router};
use serde::{Deserialize, Serialize};
use synergos_net::{quic::QuicManager, types::PeerId};
use tokio::sync::broadcast;

/// Peer-info JSON 応答。クライアント (peer add-url 経由の bootstrap) はこの値を
/// もとに `expected_peer_id` を学習し、`quic_endpoint` に対して QUIC 接続する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfoResponse {
    /// Synergos ノードの PeerId (ed25519 派生)。クライアントは TLS handshake
    /// 時に提示される自己署名証明書とこの ID を突き合わせて peer 真性を検証する。
    pub peer_id: String,
    /// クライアントが直接 QUIC 接続すべきエンドポイント
    /// (例 `[2406:da14:...]:7777` / `203.0.113.1:7777`)。
    /// `None` ならサーバはまだ QUIC bind を完了していない (起動初期)。
    pub quic_endpoint: Option<String>,
    /// プロトコルバージョン (semver-major)。クライアントが互換性チェックに使う。
    pub protocol_version: u32,
    /// オプションのサーバ表示名 (UI 用)。匿名性が要るなら空文字に。
    pub server_name: String,
}

/// 現在のプロトコルバージョン (bump はクライアントとの互換性を切るときのみ)
pub const PEER_INFO_PROTOCOL_VERSION: u32 = 1;

/// servlet 共有 state。axum の `with_state` で各 handler に注入する。
#[derive(Clone)]
struct AppState {
    peer_id: PeerId,
    quic: Arc<QuicManager>,
    server_name: String,
}

async fn handle_peer_info(State(s): State<AppState>) -> Json<PeerInfoResponse> {
    let endpoint = s.quic.local_addr().await.map(|a| a.to_string());
    Json(PeerInfoResponse {
        peer_id: s.peer_id.to_string(),
        quic_endpoint: endpoint,
        protocol_version: PEER_INFO_PROTOCOL_VERSION,
        server_name: s.server_name.clone(),
    })
}

async fn handle_health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

/// servlet を構築する Router。テストから直接叩けるように切り出してある。
fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/peer-info", get(handle_peer_info))
        .route("/health", get(handle_health))
        .with_state(state)
}

/// Peer-info servlet を起動する。`shutdown_rx` を受け取ったら graceful shutdown する。
///
/// `listen_addr` は通常 `127.0.0.1:7780` 等のローカル loopback で、Cloudflare
/// Tunnel 経由で外部に publish される。直接 0.0.0.0 に bind しないこと
/// (servlet 自体には auth が無いため、生で外に出すと info disclosure になる)。
pub async fn run(
    listen_addr: SocketAddr,
    peer_id: PeerId,
    quic: Arc<QuicManager>,
    server_name: String,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let state = AppState {
        peer_id,
        quic,
        server_name,
    };
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .map_err(|e| anyhow::anyhow!("peer-info bind failed on {listen_addr}: {e}"))?;
    let actual = listener
        .local_addr()
        .map_err(|e| anyhow::anyhow!("peer-info local_addr: {e}"))?;
    tracing::info!("peer-info servlet listening on {}", actual);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
            tracing::debug!("peer-info servlet shutdown signal received");
        })
        .await
        .map_err(|e| anyhow::anyhow!("peer-info serve error: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use synergos_net::config::QuicConfig;
    use synergos_net::identity::Identity;

    fn qcfg() -> QuicConfig {
        QuicConfig {
            max_concurrent_streams: 8,
            idle_timeout_ms: 5_000,
            max_udp_payload_size: 1350,
            enable_0rtt: false,
            listen_addr: None,
        }
    }

    /// QUIC が未 bind の状態でも /peer-info は 200 で `quic_endpoint=None` を返す。
    #[tokio::test]
    async fn peer_info_returns_none_endpoint_before_quic_bind() {
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));

        // bind しないまま GET /peer-info
        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert_eq!(resp.peer_id, peer_id.to_string());
        assert!(resp.quic_endpoint.is_none());
        assert_eq!(resp.protocol_version, PEER_INFO_PROTOCOL_VERSION);
        assert_eq!(resp.server_name, "test");
    }

    /// QUIC が bind 済みなら /peer-info は実際の listen address を返す。
    #[tokio::test]
    async fn peer_info_returns_quic_endpoint_after_bind() {
        use std::net::Ipv4Addr;
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));
        let bound = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert_eq!(resp.quic_endpoint, Some(bound.to_string()));
    }

    /// /health は 200 を返す。
    #[tokio::test]
    async fn health_returns_200() {
        let identity = Arc::new(Identity::generate());
        let quic = Arc::new(QuicManager::new(qcfg(), identity.clone()));
        let app = build_router(AppState {
            peer_id: identity.peer_id().clone(),
            quic,
            server_name: String::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/health", addr);
        let body = reqwest_get_text(&url).await;
        assert_eq!(body, "ok");
    }

    /// reqwest を入れたくないので tokio + 手書き HTTP/1.1 で GET する小ヘルパ。
    /// テスト専用。
    async fn reqwest_get_json(url: &str) -> PeerInfoResponse {
        let body = reqwest_get_text(url).await;
        serde_json::from_str(&body).expect("json parse")
    }

    async fn reqwest_get_text(url: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // url = http://host:port/path
        let stripped = url.strip_prefix("http://").expect("http:// prefix");
        let slash = stripped.find('/').expect("path slash");
        let host = &stripped[..slash];
        let path = &stripped[slash..];
        let mut s = tokio::net::TcpStream::connect(host).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.unwrap();
        let resp = String::from_utf8_lossy(&buf).to_string();
        // 最後の \r\n\r\n の後ろが body
        let split = resp.find("\r\n\r\n").expect("header/body separator");
        resp[split + 4..].to_string()
    }
}
