//! ピア参加ヘルパ — `/peer-info` URL への bootstrap (QUIC 接続) と
//! PresenceService への登録をひとまとめにする。
//!
//! `peer add-url` と自己完結型招待トークンによる `project join` の両方が
//! 同じ手順を踏むので、ここに寄せる。

use std::time::Duration;

use synergos_net::types::PeerId;

use crate::ipc_server::ServiceContext;
use crate::peer_bootstrap::{bootstrap_from_url_expected, BootstrapResult};
use crate::presence::NodeRegistration;
use crate::presence::{NodeRegistry, PeerState};

/// `url` の `/peer-info` を引いて QUIC 接続し、Presence に Connected として登録する。
/// `expected_peer` が与えられた場合、相手の peer_id が一致しなければエラー
/// (招待トークンに書かれたホストと別ノードに繋がった = URL 差し替え等)。
pub async fn bootstrap_and_register(
    ctx: &ServiceContext,
    project_id: &str,
    url: &str,
    expected_peer: Option<&PeerId>,
) -> Result<BootstrapResult, String> {
    let result =
        bootstrap_from_url_expected(url, &ctx.quic, Duration::from_secs(10), expected_peer)
            .await
            .map_err(|e| format!("bootstrap failed: {e}"))?;
    let registration = NodeRegistration {
        peer_id: result.peer_id.clone(),
        display_name: result.peer_id.to_string(),
        endpoints: vec![],
        project_ids: vec![project_id.to_string()],
        synergos_version: result.synergos_version.clone(),
    };
    ctx.presence
        .register_node(registration)
        .await
        .map_err(|e| format!("register_node failed: {e}"))?;
    let _ = ctx
        .presence
        .update_node_state(&result.peer_id, PeerState::Connected)
        .await;
    Ok(result)
}

/// 招待トークンに埋める `/peer-info` URL を決める。
/// 優先順: 明示引数 → config `peer_info_advertised_url` → `peer_info_listen_addr`
/// が具体 IP なら `http://<ip>:<port>` → None (従来型トークンへフォールバック)。
pub fn resolve_advertised_peer_info_url(
    explicit: Option<String>,
    net_config: Option<&synergos_net::config::NetConfig>,
) -> Option<String> {
    if let Some(u) = explicit.filter(|u| !u.trim().is_empty()) {
        return Some(u.trim().to_string());
    }
    let cfg = net_config?;
    if let Some(u) = cfg
        .peer_info_advertised_url
        .clone()
        .filter(|u| !u.trim().is_empty())
    {
        return Some(u.trim().to_string());
    }
    let addr = cfg.peer_info_listen_addr?;
    if addr.ip().is_unspecified() || addr.ip().is_loopback() {
        return None;
    }
    Some(format!("http://{addr}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use synergos_net::config::NetConfig;

    #[test]
    fn explicit_wins() {
        let cfg = NetConfig::default();
        assert_eq!(
            resolve_advertised_peer_info_url(Some("http://x:1".into()), Some(&cfg)),
            Some("http://x:1".into())
        );
    }

    #[test]
    fn config_then_derived_then_none() {
        let mut cfg = NetConfig::default();
        assert_eq!(resolve_advertised_peer_info_url(None, Some(&cfg)), None);
        cfg.peer_info_listen_addr = Some("0.0.0.0:7780".parse().unwrap());
        assert_eq!(resolve_advertised_peer_info_url(None, Some(&cfg)), None);
        cfg.peer_info_listen_addr = Some("192.168.1.10:7780".parse().unwrap());
        assert_eq!(
            resolve_advertised_peer_info_url(None, Some(&cfg)),
            Some("http://192.168.1.10:7780".into())
        );
        cfg.peer_info_advertised_url = Some("http://100.96.0.5:7780".into());
        assert_eq!(
            resolve_advertised_peer_info_url(None, Some(&cfg)),
            Some("http://100.96.0.5:7780".into())
        );
    }
}
