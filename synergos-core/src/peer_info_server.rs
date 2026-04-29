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
    /// このノードの synergos-core build version (`CARGO_PKG_VERSION`)。
    /// クライアントが互換性確認 / GUI 表示用に使う。古い peer は省略するので
    /// `#[serde(default)]` で `""` にフォールバック。
    #[serde(default)]
    pub synergos_version: String,
}

/// 現在のプロトコルバージョン (bump はクライアントとの互換性を切るときのみ)
pub const PEER_INFO_PROTOCOL_VERSION: u32 = 1;

/// このバイナリの synergos-core build version。`Cargo.toml` の `[package].version` 由来。
pub const SYNERGOS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// servlet 共有 state。axum の `with_state` で各 handler に注入する。
#[derive(Clone)]
struct AppState {
    peer_id: PeerId,
    quic: Arc<QuicManager>,
    server_name: String,
    /// 明示的に告知する QUIC エンドポイント。`None` なら `quic.local_addr()` を返す。
    /// Cloudflare proxied DNS の裏で動く公開ノードでは、ここに EC2 の real public
    /// IPv6/IPv4:port を入れてクライアントに直結させる。
    advertised_addr: Option<String>,
}

async fn handle_peer_info(State(s): State<AppState>) -> Json<PeerInfoResponse> {
    let local = s.quic.local_addr().await;
    let endpoint = match s.advertised_addr.as_deref() {
        // 明示的なリテラル指定 (Some(literal で "auto" 以外))
        Some(literal) if literal != "auto" => Some(literal.to_string()),
        // None or "auto" → 自動検出 (HTTPS echo: IPv6 → IPv4 → NIC enum の 3 段 fallback)
        _ => match (auto_advertise_addr().await, local) {
            (Some(ip), Some(la)) => Some(format_addr(ip, la.port())),
            _ => local.map(|a| a.to_string()),
        },
    };
    Json(PeerInfoResponse {
        peer_id: s.peer_id.to_string(),
        quic_endpoint: endpoint,
        protocol_version: PEER_INFO_PROTOCOL_VERSION,
        server_name: s.server_name.clone(),
        synergos_version: SYNERGOS_VERSION.to_string(),
    })
}

fn format_addr(ip: std::net::IpAddr, port: u16) -> String {
    match ip {
        std::net::IpAddr::V6(v6) => format!("[{}]:{}", v6, port),
        std::net::IpAddr::V4(v4) => format!("{}:{}", v4, port),
    }
}

/// `quic_advertised_addr` が `None` または `"auto"` のとき呼ばれる。
///
/// 戦略 (3 段 fallback):
///   1. **HTTPS で IPv6 echo** に問い合わせ (`ipv6.icanhazip.com` 等)。NAT/LB 越しでも外から見える IPv6 が取れる
///   2. **HTTPS で IPv4 echo** に問い合わせ (`api4.ipify.org` 等)。IPv6 が ISP/家庭ルーターで詰まる環境向け
///   3. ローカル NIC を列挙して global scope IPv6 を採用 (`if-addrs`)
///   4. どれもダメ → `None` (handler 側で `local_addr` に fallback)
///
/// IPv6 を IPv4 より優先する理由:
///   - IPv6 は NAT 不要で対称な経路が取れる (本来理想)
///   - IPv4 は CGNAT 越しだと帰路の問題が出ることがある
///   - とは言え v6 が壊れていることも多いので fallback 必須
async fn auto_advertise_addr() -> Option<std::net::IpAddr> {
    if let Some(v6) = discover_public_addr_via_https::<std::net::Ipv6Addr>(IPV6_ENDPOINTS).await {
        tracing::info!("auto-advertise: discovered public IPv6 via HTTPS: {}", v6);
        return Some(std::net::IpAddr::V6(v6));
    }
    if let Some(v4) = discover_public_addr_via_https::<std::net::Ipv4Addr>(IPV4_ENDPOINTS).await {
        tracing::info!(
            "auto-advertise: IPv6 echo failed, discovered public IPv4 via HTTPS: {}",
            v4
        );
        return Some(std::net::IpAddr::V4(v4));
    }
    if let Some(v6) = synergos_net::promotion::probe_ipv6_global()
        .await
        .into_iter()
        .next()
    {
        tracing::info!(
            "auto-advertise: HTTPS probes failed, using local NIC IPv6: {}",
            v6
        );
        return Some(std::net::IpAddr::V6(v6));
    }
    tracing::warn!("auto-advertise: no public address discoverable; falling back to bind addr");
    None
}

const IPV6_ENDPOINTS: &[&str] = &[
    "https://ipv6.icanhazip.com",
    "https://api6.ipify.org",
    "https://v6.ident.me",
];

const IPV4_ENDPOINTS: &[&str] = &[
    "https://ipv4.icanhazip.com",
    "https://api4.ipify.org",
    "https://v4.ident.me",
];

/// HTTPS で IP echo サービス群に問い合わせ、最初に取れた値を返す。
/// `T` は `Ipv4Addr` か `Ipv6Addr`。response が parse できなければ次の endpoint を試す。
async fn discover_public_addr_via_https<T>(endpoints: &[&str]) -> Option<T>
where
    T: std::str::FromStr,
{
    use std::time::Duration;

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("auto-advertise: reqwest client build failed: {e}");
            return None;
        }
    };

    for url in endpoints {
        match client.get(*url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(body) => {
                    let trimmed = body.trim();
                    if let Ok(addr) = trimmed.parse::<T>() {
                        return Some(addr);
                    } else {
                        tracing::debug!("auto-advertise: {url} returned unparseable: {trimmed:?}");
                    }
                }
                Err(e) => tracing::debug!("auto-advertise: {url} body read failed: {e}"),
            },
            Ok(resp) => tracing::debug!("auto-advertise: {url} status {}", resp.status()),
            Err(e) => tracing::debug!("auto-advertise: {url} send failed: {e}"),
        }
    }
    None
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
///
/// `advertised_addr` の意味:
///   - `None` または `Some("auto")` → 自動検出 (HTTPS echo IPv6 → IPv4 → NIC enum)
///   - `Some("[2406::1]:7777")` 等のリテラル → そのまま `/peer-info` に返す
///
/// 既定 (None) を auto にした理由: 公開ノードでは config に IP を書きたくない要件、
/// かつ bind addr (`[::]:port`) は unspecified なのでクライアントから繋がらない。
pub async fn run(
    listen_addr: SocketAddr,
    peer_id: PeerId,
    quic: Arc<QuicManager>,
    server_name: String,
    advertised_addr: Option<String>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let state = AppState {
        peer_id,
        quic,
        server_name,
        advertised_addr,
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
            advertised_addr: None,
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

    /// QUIC が bind 済みかつ advertised_addr がリテラル指定 (auto 以外) のときは
    /// 自動検出を一切走らせず、その値をそのまま返すこと。auto 検出は HTTPS を叩くので
    /// テスト環境で発火させたくない (リテラル経路の純粋検証用)。
    #[tokio::test]
    async fn peer_info_returns_literal_advertised_addr() {
        use std::net::Ipv4Addr;
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));
        let _ = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

        let literal = "192.0.2.1:9999".to_string();
        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
            advertised_addr: Some(literal.clone()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert_eq!(resp.quic_endpoint, Some(literal));
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
            advertised_addr: None,
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

    /// `advertised_addr` を指定するとそれを優先して /peer-info に返す。
    #[tokio::test]
    async fn peer_info_uses_advertised_addr_when_set() {
        use std::net::Ipv4Addr;
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));
        let _ = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

        let advertised = "[2406:da14:abcd:ef::1]:7777".to_string();
        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
            advertised_addr: Some(advertised.clone()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert_eq!(resp.quic_endpoint, Some(advertised));
    }

    /// `advertised_addr = "auto"` (or `None`) はどちらも自動検出経路を走らせる。
    /// 検出に成功すれば public IPv6/IPv4、失敗すれば local_addr に fallback する。
    /// CI runner で global IPv6 が無い + HTTPS 不通の場合は local_addr が返る前提。
    /// どちらの経路でも None にはならない (QUIC bind 済) こと、ポートが bind と
    /// 一致することを確認。
    #[tokio::test]
    async fn peer_info_auto_advertise_returns_some_endpoint_with_bind_port() {
        use std::net::Ipv4Addr;
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));
        let bound = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
            advertised_addr: Some("auto".into()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert!(resp.quic_endpoint.is_some());
        let endpoint = resp.quic_endpoint.unwrap();
        assert!(endpoint.ends_with(&format!(":{}", bound.port())));
    }

    /// `advertised_addr = None` も "auto" と同じ自動検出経路を辿ること。
    /// (config 未設定をデフォルトで auto 扱いにする変更の保証)
    #[tokio::test]
    async fn peer_info_none_advertise_uses_auto_path() {
        use std::net::Ipv4Addr;
        let identity = Arc::new(Identity::generate());
        let peer_id = identity.peer_id().clone();
        let quic = Arc::new(QuicManager::new(qcfg(), identity));
        let bound = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

        let app = build_router(AppState {
            peer_id: peer_id.clone(),
            quic: quic.clone(),
            server_name: "test".into(),
            advertised_addr: None, // ← "auto" ではなく None
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{}/peer-info", addr);
        let resp = reqwest_get_json(&url).await;
        assert!(resp.quic_endpoint.is_some());
        // bind port が保たれること (検出が成功しても失敗しても)
        let endpoint = resp.quic_endpoint.unwrap();
        assert!(endpoint.ends_with(&format!(":{}", bound.port())));
    }

    /// `format_addr` は IPv6 を `[v6]:port`、IPv4 を `v4:port` に整形する。
    #[test]
    fn format_addr_v6_uses_brackets() {
        let v6: std::net::IpAddr = "2406:da14:abcd:ef::1".parse().unwrap();
        assert_eq!(format_addr(v6, 7777), "[2406:da14:abcd:ef::1]:7777");
    }

    #[test]
    fn format_addr_v4_no_brackets() {
        let v4: std::net::IpAddr = "3.112.56.98".parse().unwrap();
        assert_eq!(format_addr(v4, 7777), "3.112.56.98:7777");
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
