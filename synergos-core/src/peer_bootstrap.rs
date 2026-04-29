//! URL ベースの peer bootstrap
//!
//! `peer add-url <https://node1.example.com>` から呼ばれ、相手 daemon の
//! `/peer-info` HTTPS エンドポイントを GET → 取得した peer_id + QUIC endpoint
//! で QUIC 接続を張る。invite token を必要としないクロスマシン peer 追加経路。
//!
//! ## 設計メモ
//!
//! - `quic_endpoint` の IP 部が unspecified (`0.0.0.0` / `[::]`) なら、URL の
//!   hostname を DNS resolve した結果に置き換える。これにより daemon 側は
//!   `[::]:7777` のような bind 表現で十分で、公開 IP を自前で知る必要がない。
//! - protocol_version 不一致は早期に拒否する (将来 wire 変更時の安全弁)。
//! - `quic.connect` は S1 仕様で `expected_peer_id` を必須、ed25519 派生の
//!   ピンニング検証で偽サーバを弾く。

use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;
use synergos_net::quic::QuicManager;
use synergos_net::types::PeerId;

use crate::peer_info_server::PEER_INFO_PROTOCOL_VERSION;

/// peer add-url の bootstrap 結果。成功時は確立した peer_id を返す。
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("invalid peer-info response: {0}")]
    InvalidResponse(String),
    #[error("incompatible protocol_version: got {got}, expected {expected}")]
    IncompatibleProtocol { got: u32, expected: u32 },
    #[error("dns resolution failed for {host}: {details}")]
    DnsFailure { host: String, details: String },
    #[error("quic connect failed: {0}")]
    QuicConnect(String),
}

#[derive(Debug, Deserialize)]
struct RemotePeerInfo {
    peer_id: String,
    quic_endpoint: Option<String>,
    protocol_version: u32,
}

/// URL から peer-info を取得して QUIC 接続を張る。
///
/// 成功時は学習した `PeerId` を返し、`QuicManager` の internal pool に接続が登録された状態。
/// 呼び出し側は presence service への登録などを別途行う。
pub async fn bootstrap_from_url(
    base_url: &str,
    quic: &QuicManager,
    timeout: Duration,
) -> Result<PeerId, BootstrapError> {
    // 1. URL parse + /peer-info path 付与
    let parsed =
        reqwest::Url::parse(base_url).map_err(|e| BootstrapError::InvalidUrl(format!("{e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| BootstrapError::InvalidUrl("url has no host".into()))?
        .to_string();
    let info_url = build_peer_info_url(&parsed);

    // 2. HTTPS GET /peer-info
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| BootstrapError::Http(format!("client build: {e}")))?;
    let resp = client
        .get(info_url.clone())
        .send()
        .await
        .map_err(|e| BootstrapError::Http(format!("GET {info_url} failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(BootstrapError::Http(format!(
            "GET {info_url} returned {}",
            resp.status()
        )));
    }
    let info: RemotePeerInfo = resp
        .json()
        .await
        .map_err(|e| BootstrapError::InvalidResponse(format!("json parse: {e}")))?;

    // 3. protocol version check
    if info.protocol_version != PEER_INFO_PROTOCOL_VERSION {
        return Err(BootstrapError::IncompatibleProtocol {
            got: info.protocol_version,
            expected: PEER_INFO_PROTOCOL_VERSION,
        });
    }

    let quic_endpoint = info
        .quic_endpoint
        .as_deref()
        .ok_or_else(|| BootstrapError::InvalidResponse("remote has no quic_endpoint".into()))?;
    let advertised: SocketAddr = quic_endpoint
        .parse()
        .map_err(|e| BootstrapError::InvalidResponse(format!("invalid quic_endpoint: {e}")))?;

    // 4. unspecified IP は URL host で置換
    let connect_addr = resolve_connect_addr(&host, advertised).await?;

    // 5. QUIC connect (S1 仕様で peer_id 検証つき)
    let expected_peer_id = PeerId::new(&info.peer_id);
    quic.connect(expected_peer_id.clone(), connect_addr, "synergos")
        .await
        .map_err(|e| BootstrapError::QuicConnect(format!("{e}")))?;

    Ok(expected_peer_id)
}

fn build_peer_info_url(base: &reqwest::Url) -> reqwest::Url {
    let mut u = base.clone();
    let path = u.path();
    let new_path = if path.is_empty() || path == "/" {
        "/peer-info".to_string()
    } else if path.ends_with('/') {
        format!("{path}peer-info")
    } else {
        format!("{}/peer-info", path.trim_end_matches('/'))
    };
    u.set_path(&new_path);
    u
}

async fn resolve_connect_addr(
    host: &str,
    advertised: SocketAddr,
) -> Result<SocketAddr, BootstrapError> {
    if !advertised.ip().is_unspecified() {
        return Ok(advertised);
    }
    let target = format!("{host}:{}", advertised.port());
    let mut iter =
        tokio::net::lookup_host(&target)
            .await
            .map_err(|e| BootstrapError::DnsFailure {
                host: host.to_string(),
                details: format!("{e}"),
            })?;
    // IPv6 を優先 (LUDIARS は IPv6-first 設計)
    let collected: Vec<SocketAddr> = iter.by_ref().collect();
    let chosen = collected
        .iter()
        .find(|a| a.is_ipv6())
        .or_else(|| collected.first())
        .ok_or_else(|| BootstrapError::DnsFailure {
            host: host.to_string(),
            details: "no addresses returned".into(),
        })?;
    Ok(*chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_peer_info_url_root() {
        let url = reqwest::Url::parse("https://node1.example.com").unwrap();
        let info = build_peer_info_url(&url);
        assert_eq!(info.as_str(), "https://node1.example.com/peer-info");
    }

    #[test]
    fn build_peer_info_url_with_trailing_slash() {
        let url = reqwest::Url::parse("https://node1.example.com/").unwrap();
        let info = build_peer_info_url(&url);
        assert_eq!(info.as_str(), "https://node1.example.com/peer-info");
    }

    #[test]
    fn build_peer_info_url_with_subpath() {
        let url = reqwest::Url::parse("https://relay.example.com/api").unwrap();
        let info = build_peer_info_url(&url);
        assert_eq!(info.as_str(), "https://relay.example.com/api/peer-info");
    }

    #[test]
    fn build_peer_info_url_with_subpath_trailing_slash() {
        let url = reqwest::Url::parse("https://relay.example.com/api/").unwrap();
        let info = build_peer_info_url(&url);
        assert_eq!(info.as_str(), "https://relay.example.com/api/peer-info");
    }

    /// `[::]:port` (unspecified IPv6) なら hostname 解決にフォールバックする。
    #[tokio::test]
    async fn resolve_connect_addr_replaces_unspecified_ipv6() {
        let advertised: SocketAddr = "[::]:7777".parse().unwrap();
        // localhost は ::1 / 127.0.0.1 に解決される
        let resolved = resolve_connect_addr("localhost", advertised).await.unwrap();
        assert_eq!(resolved.port(), 7777);
        assert!(!resolved.ip().is_unspecified());
    }

    /// 具体的な IP の場合はそのまま返す (DNS lookup しない)。
    #[tokio::test]
    async fn resolve_connect_addr_keeps_specific() {
        let advertised: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        let resolved = resolve_connect_addr("does-not-matter", advertised)
            .await
            .unwrap();
        assert_eq!(resolved, advertised);
    }
}
