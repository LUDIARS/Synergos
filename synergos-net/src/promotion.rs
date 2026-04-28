//! NAT 越え能力の自動 probe + ノード昇格判定
//!
//! 起動時に **以下を並列に試して**、ノードが direct/tunnel 経路を持てるかを判定する:
//!
//! 1. **IPv6 グローバルアドレス**: `if-addrs` で OS のインターフェースを列挙、
//!    link-local / loopback / unique-local を除外した global IPv6 が 1 つでも
//!    あれば direct 可。
//! 2. **UPnP / NAT-PMP**: `igd-next` で Internet Gateway Device に discover を
//!    投げる。応答があれば家庭用ルータでポート開放可能 → direct 可。
//! 3. **Cloudflare Tunnel**: `cloudflared` バイナリが PATH にあり、
//!    NetConfig に api_token_ref が設定されていれば tunnel 経路あり。
//! 4. **Relay**: NetConfig に relay endpoint が設定されているか。
//!
//! 結果に応じて **effective_relay_only** を決定する:
//!   - `force_relay_only=true` → ユーザ強制、probe 結果問わず relay only
//!   - `auto_promote=true` (既定) かつ direct/tunnel いずれも不可 → relay only
//!   - direct or tunnel 可能 → full node モード
//!
//! probe 自体は best-effort。ネットワーク不通や timeout は「不可」扱い。

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

/// 環境調査の結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetCapabilities {
    /// グローバル IPv6 アドレスを 1 つ以上保持しているか
    pub has_ipv6_global: bool,
    /// 検出した global IPv6 アドレス一覧 (デバッグ表示用)
    pub ipv6_addresses: Vec<String>,
    /// UPnP / NAT-PMP で port mapping できるか + 公開 IP
    pub upnp: Option<UpnpInfo>,
    /// `cloudflared` バイナリが PATH にあり tunnel 設定が揃っているか
    pub tunnel_available: bool,
    /// NetConfig に relay endpoint が設定されているか (relay は最終 fallback)
    pub relay_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpnpInfo {
    pub external_ip: String,
    pub gateway_url: String,
}

/// 推奨される運用モード。auto_promote の判定結果として返る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromotionMode {
    /// IPv6 / UPnP / Tunnel いずれかで外部到達可能 → 通常ノード
    FullNode,
    /// 直接到達手段なし → relay 経由のみで稼働
    RelayOnly,
}

impl NetCapabilities {
    /// 各 probe をタイムアウト付きで並列に走らせる。
    ///
    /// `tunnel_token_configured`: Tunnel 設定 (`api_token_ref` が空でないか) を
    /// 呼出側で判定して渡す。`relay_endpoint_configured`: 同様に relay 設定の有無。
    pub async fn detect(
        per_probe_timeout: Duration,
        tunnel_token_configured: bool,
        relay_endpoint_configured: bool,
    ) -> Self {
        let ipv6_fut = tokio::time::timeout(per_probe_timeout, probe_ipv6_global());
        let upnp_fut = tokio::time::timeout(per_probe_timeout, probe_upnp());
        let tunnel_fut = tokio::time::timeout(
            per_probe_timeout,
            probe_cloudflared(tunnel_token_configured),
        );

        let (ipv6_res, upnp_res, tunnel_res) = tokio::join!(ipv6_fut, upnp_fut, tunnel_fut);

        let ipv6 = ipv6_res.unwrap_or_else(|_| Vec::new());
        let upnp = upnp_res.unwrap_or(None);
        let tunnel_available = tunnel_res.unwrap_or(false);

        Self {
            has_ipv6_global: !ipv6.is_empty(),
            ipv6_addresses: ipv6.iter().map(|a| a.to_string()).collect(),
            upnp,
            tunnel_available,
            relay_configured: relay_endpoint_configured,
        }
    }

    /// FullNode / RelayOnly のどちらが推奨かを返す。
    pub fn recommended_mode(&self) -> PromotionMode {
        if self.has_ipv6_global || self.upnp.is_some() || self.tunnel_available {
            PromotionMode::FullNode
        } else {
            PromotionMode::RelayOnly
        }
    }

    /// 人間向けの 1 行サマリ (ログ / status 表示用)。
    pub fn summary(&self) -> String {
        let mode = self.recommended_mode();
        format!(
            "promotion={:?} ipv6_global={} upnp={} tunnel={} relay_configured={}",
            mode,
            self.has_ipv6_global,
            self.upnp.is_some(),
            self.tunnel_available,
            self.relay_configured,
        )
    }
}

/// global IPv6 アドレスを列挙する。link-local / loopback / unique-local は除外。
async fn probe_ipv6_global() -> Vec<Ipv6Addr> {
    // if-addrs は同期 API なので spawn_blocking で逃がす
    tokio::task::spawn_blocking(|| {
        if_addrs::get_if_addrs()
            .ok()
            .map(|ifs| {
                ifs.into_iter()
                    .filter_map(|i| match i.ip() {
                        IpAddr::V6(v6) if is_global_ipv6(&v6) => Some(v6),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

/// IPv6 が「外部到達可能なグローバル」かを判定。
///
/// 除外:
///   - loopback (`::1`)
///   - link-local (`fe80::/10`)
///   - unique-local (`fc00::/7`) — VPN/メッシュ用なのでグローバル扱いしない
///   - documentation (`2001:db8::/32`)
///   - unspecified (`::`)
///
/// 採用:
///   - global unicast (主に `2000::/3` の範囲)
fn is_global_ipv6(addr: &Ipv6Addr) -> bool {
    if addr.is_loopback() || addr.is_unspecified() || addr.is_multicast() {
        return false;
    }
    let segments = addr.segments();
    // link-local fe80::/10
    if segments[0] & 0xffc0 == 0xfe80 {
        return false;
    }
    // unique-local fc00::/7
    if segments[0] & 0xfe00 == 0xfc00 {
        return false;
    }
    // documentation 2001:db8::/32
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return false;
    }
    // 2000::/3 が global unicast (= 先頭 3 bit が 001)
    (segments[0] & 0xe000) == 0x2000
}

/// UPnP IGD discover を試みる。応答があれば external IP を返す。
async fn probe_upnp() -> Option<UpnpInfo> {
    // igd-next は async API を持つ (search_gateway / get_external_ip)
    use igd_next::aio::tokio as igd_tokio;
    use igd_next::SearchOptions;

    // discover タイムアウトは内部で 3 秒既定だが、明示的に短く設定
    let opts = SearchOptions {
        timeout: Some(Duration::from_secs(2)),
        ..Default::default()
    };

    let gateway = match igd_tokio::search_gateway(opts).await {
        Ok(g) => g,
        Err(e) => {
            tracing::debug!("UPnP discover failed: {e}");
            return None;
        }
    };
    let external_ip = match gateway.get_external_ip().await {
        Ok(ip) => ip,
        Err(e) => {
            tracing::debug!("UPnP get_external_ip failed: {e}");
            return None;
        }
    };
    let gateway_url = gateway.root_url.to_string();
    Some(UpnpInfo {
        external_ip: external_ip.to_string(),
        gateway_url,
    })
}

/// `cloudflared` がインストールされているかと、token が設定されているかをチェック。
async fn probe_cloudflared(token_configured: bool) -> bool {
    if !token_configured {
        return false;
    }
    tokio::task::spawn_blocking(|| which::which("cloudflared").is_ok())
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_global_classification() {
        // global
        assert!(is_global_ipv6(
            &"2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap()
        ));
        assert!(is_global_ipv6(
            &"2001:4860:4860::8888".parse::<Ipv6Addr>().unwrap()
        ));
        // not global
        assert!(!is_global_ipv6(&"::1".parse::<Ipv6Addr>().unwrap()));
        assert!(!is_global_ipv6(&"fe80::1".parse::<Ipv6Addr>().unwrap()));
        assert!(!is_global_ipv6(&"fc00::1".parse::<Ipv6Addr>().unwrap()));
        assert!(!is_global_ipv6(&"2001:db8::1".parse::<Ipv6Addr>().unwrap()));
        assert!(!is_global_ipv6(&"::".parse::<Ipv6Addr>().unwrap()));
    }

    #[test]
    fn recommended_mode_logic() {
        // 何も無いと relay
        let empty = NetCapabilities {
            has_ipv6_global: false,
            ipv6_addresses: vec![],
            upnp: None,
            tunnel_available: false,
            relay_configured: false,
        };
        assert_eq!(empty.recommended_mode(), PromotionMode::RelayOnly);

        // IPv6 だけで full
        let ipv6_only = NetCapabilities {
            has_ipv6_global: true,
            ..empty.clone()
        };
        assert_eq!(ipv6_only.recommended_mode(), PromotionMode::FullNode);

        // UPnP だけで full
        let upnp_only = NetCapabilities {
            upnp: Some(UpnpInfo {
                external_ip: "203.0.113.1".into(),
                gateway_url: "http://192.168.1.1:5000/rootDesc.xml".into(),
            }),
            ..empty.clone()
        };
        assert_eq!(upnp_only.recommended_mode(), PromotionMode::FullNode);

        // tunnel だけで full
        let tunnel_only = NetCapabilities {
            tunnel_available: true,
            ..empty.clone()
        };
        assert_eq!(tunnel_only.recommended_mode(), PromotionMode::FullNode);
    }

    #[tokio::test]
    async fn detect_does_not_panic_on_short_timeout() {
        // 短い timeout でも detect が完走 (panic なし) すること。
        // 結果のモードは実行環境依存 (IPv6 global がある PC なら FullNode、
        // 無ければ RelayOnly)。ここでは「呼んでも落ちない」だけを保証する。
        let caps = NetCapabilities::detect(Duration::from_millis(100), false, false).await;
        let _ = caps.recommended_mode();
        let _ = caps.summary();
    }
}
