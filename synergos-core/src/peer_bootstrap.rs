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

/// peer add-url の bootstrap 結果。成功時は学習した peer_id と相手の synergos
/// バージョンを返す。
#[derive(Debug, Clone)]
pub struct BootstrapResult {
    pub peer_id: PeerId,
    /// 相手 daemon の `CARGO_PKG_VERSION`。古い peer から `""` を返されることもある
    pub synergos_version: String,
}

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
    #[serde(default)]
    synergos_version: String,
}

/// URL から peer-info を取得して QUIC 接続を張る。
///
/// 成功時は学習した `PeerId` と相手の synergos バージョンを `BootstrapResult` で
/// 返し、`QuicManager` の internal pool に接続が登録された状態。
/// 呼び出し側は presence service への登録などを別途行う。
pub async fn bootstrap_from_url(
    base_url: &str,
    quic: &QuicManager,
    timeout: Duration,
) -> Result<BootstrapResult, BootstrapError> {
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

    // 4. 接続候補アドレスを列挙する。
    //    - advertised がそのまま使える場合は primary 候補に
    //    - hostname 解決の他ファミリ (v6→v4 または v4→v6) を fallback に
    //    片側 (典型は v6) の経路が ISP / 家庭ルータ起因で死んでいる
    //    非対称ネットワークでも、もう片側で繋がる経路を残す。
    let candidates = resolve_candidate_addrs(&host, advertised).await?;

    // 5. 各候補に順番に QUIC connect (S1 仕様で peer_id 検証つき)
    let expected_peer_id = PeerId::new(&info.peer_id);
    let mut last_err: Option<BootstrapError> = None;
    for addr in &candidates {
        tracing::info!(
            "bootstrap candidate {} ({}/{} for {host})",
            addr,
            if addr.is_ipv6() { "v6" } else { "v4" },
            candidates.len()
        );
        match quic
            .connect(expected_peer_id.clone(), *addr, "synergos")
            .await
        {
            Ok(()) => {
                return Ok(BootstrapResult {
                    peer_id: expected_peer_id,
                    synergos_version: info.synergos_version,
                });
            }
            Err(e) => {
                tracing::warn!("bootstrap candidate {addr} failed: {e}");
                last_err = Some(BootstrapError::QuicConnect(format!("{addr}: {e}")));
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        BootstrapError::QuicConnect(format!("no usable address resolved for {host}"))
    }))
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

/// 接続候補アドレスを優先順で列挙する。
///
/// - advertised が specific IP (host literal) なら、それを primary に。
///   さらに hostname を DNS resolve して、advertised と **異なるファミリ**
///   のアドレスがあれば fallback として末尾に追加する。
/// - advertised が unspecified (`0.0.0.0` / `[::]`) なら、hostname を DNS
///   resolve した全アドレス。advertised が `[::]:p` なら IPv6 を、`0.0.0.0:p`
///   なら IPv4 を優先する。
///
/// これにより:
/// - サーバの advertised が IPv6 でも、クライアントの v6 経路が壊れている
///   場合に v4 で再試行できる
/// - 旧来の「IPv6-first」挙動は維持される
async fn resolve_candidate_addrs(
    host: &str,
    advertised: SocketAddr,
) -> Result<Vec<SocketAddr>, BootstrapError> {
    let port = advertised.port();
    let target = format!("{host}:{port}");

    // DNS lookup (失敗しても advertised があれば続行する)
    let dns: Vec<SocketAddr> = match tokio::net::lookup_host(&target).await {
        Ok(iter) => iter.collect(),
        Err(e) => {
            // advertised が specific なら DNS なしでも進める
            if !advertised.ip().is_unspecified() {
                tracing::debug!(
                    "DNS lookup of {host} failed but advertised is specific ({advertised}); proceeding: {e}"
                );
                vec![]
            } else {
                return Err(BootstrapError::DnsFailure {
                    host: host.to_string(),
                    details: format!("{e}"),
                });
            }
        }
    };

    let mut out: Vec<SocketAddr> = Vec::new();

    if !advertised.ip().is_unspecified() {
        // advertised の IP をそのまま primary に
        out.push(advertised);
        // 異なるファミリの DNS 解決があれば fallback として後ろに
        for a in &dns {
            if a.is_ipv6() != advertised.is_ipv6() && !out.contains(a) {
                out.push(*a);
            }
        }
        // 同ファミリでも別 IP を持っていれば末尾に積む (multi-A レコード対応)
        for a in &dns {
            if !out.contains(a) {
                out.push(*a);
            }
        }
    } else {
        // unspecified の場合は DNS の結果に頼る
        let prefer_v6 = advertised.is_ipv6();
        let (primary, secondary): (Vec<_>, Vec<_>) =
            dns.iter().partition(|a| a.is_ipv6() == prefer_v6);
        out.extend(primary.iter().copied().copied());
        out.extend(secondary.iter().copied().copied());
    }

    if out.is_empty() {
        return Err(BootstrapError::DnsFailure {
            host: host.to_string(),
            details: "no addresses returned".into(),
        });
    }
    Ok(out)
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
    async fn resolve_candidates_replaces_unspecified_ipv6() {
        let advertised: SocketAddr = "[::]:7777".parse().unwrap();
        // localhost は ::1 / 127.0.0.1 に解決される
        let candidates = resolve_candidate_addrs("localhost", advertised)
            .await
            .unwrap();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|a| a.port() == 7777));
        assert!(candidates.iter().all(|a| !a.ip().is_unspecified()));
        // ファミリ別の優先: advertised が v6 → v6 を先頭に
        assert!(candidates[0].is_ipv6() || candidates.iter().all(|a| !a.is_ipv6()));
    }

    /// 具体的な IPv4 の場合は primary は advertised そのまま。
    /// hostname 解決の v6 アドレスがあれば fallback に末尾追加される。
    #[tokio::test]
    async fn resolve_candidates_keeps_specific_then_alt_family() {
        let advertised: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        let candidates = resolve_candidate_addrs("localhost", advertised)
            .await
            .unwrap();
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0], advertised);
        // localhost が ::1 を resolve すれば fallback で含まれるはず
        // (環境差で v6 解決できないこともあるので present-or-absent のいずれも許容)
        if candidates.len() > 1 {
            assert!(candidates[1..].iter().any(|a| a.is_ipv6()));
        }
    }

    /// 解決不能な host でも advertised が specific なら primary だけ返る。
    #[tokio::test]
    async fn resolve_candidates_specific_advertised_survives_dns_failure() {
        let advertised: SocketAddr = "127.0.0.1:7777".parse().unwrap();
        let candidates = resolve_candidate_addrs("does-not-resolve.invalid", advertised)
            .await
            .unwrap();
        assert!(candidates.contains(&advertised));
    }
}
