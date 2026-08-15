//! 管制サーバー (synergos-control) への heartbeat 報告。
//!
//! opt-in 機能: `[control] heartbeat_url` が設定されている場合のみ動作する。
//! 報告内容は `{node_id, peer_id, mesh_ip, synergos_version}` で、管制サーバー側は
//! これで peer_id ↔ Mesh IP の照合 (レジストリとの突合) を行う。

use std::time::Duration;

use synergos_net::config::ControlReportConfig;
use synergos_net::types::PeerId;
use tokio::task::JoinHandle;

/// 起動時検証済みの heartbeat 設定。
pub struct HeartbeatReporter {
    config: ControlReportConfig,
    node_key: String,
    peer_id: PeerId,
}

impl HeartbeatReporter {
    /// 設定を検証して Reporter を作る。
    ///
    /// - `heartbeat_url` が空 → 機能無効 (`Ok(None)`)
    /// - 有効なのに `node_id` や node key (環境変数) が欠ける → 起動エラー
    ///   (黙って無効化しない — fail-fast)
    pub fn from_config(
        config: &ControlReportConfig,
        peer_id: PeerId,
    ) -> anyhow::Result<Option<Self>> {
        if config.heartbeat_url.trim().is_empty() {
            return Ok(None);
        }
        if config.node_id.trim().is_empty() {
            anyhow::bail!("[control] heartbeat_url is set but node_id is empty");
        }
        let node_key = std::env::var(&config.node_key_env)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "[control] heartbeat_url is set but env var {} is not set",
                    config.node_key_env
                )
            })?;
        if config.interval_secs == 0 {
            anyhow::bail!("[control] interval_secs must be >= 1");
        }
        let heartbeat_url = reqwest::Url::parse(&config.heartbeat_url)
            .map_err(|e| anyhow::anyhow!("[control] invalid heartbeat_url: {e}"))?;
        if !matches!(heartbeat_url.scheme(), "http" | "https") {
            anyhow::bail!("[control] heartbeat_url must use http or https");
        }
        if !heartbeat_url.username().is_empty() || heartbeat_url.password().is_some() {
            anyhow::bail!("[control] heartbeat_url must not contain user credentials");
        }
        if heartbeat_url.scheme() == "http" && !is_trusted_plaintext_host(&heartbeat_url) {
            anyhow::bail!(
                "[control] plaintext heartbeat_url must target loopback or Cloudflare Mesh IPv4"
            );
        }
        Ok(Some(Self {
            config: config.clone(),
            node_key,
            peer_id,
        }))
    }

    /// heartbeat 送信ループを spawn する。
    pub fn spawn(self, mut shutdown_rx: tokio::sync::broadcast::Receiver<()>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                // Mesh/loopback の平文 heartbeat を環境設定の HTTP proxy へ流さない。
                .no_proxy()
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("control heartbeat: http client init failed: {e}");
                    return;
                }
            };
            let mut interval =
                tokio::time::interval(Duration::from_secs(self.config.interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        self.send_once(&client).await;
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::debug!("control heartbeat: shutdown");
                        return;
                    }
                }
            }
        })
    }

    async fn send_once(&self, client: &reqwest::Client) {
        let mesh_ip = synergos_net::mesh_ip::detect_mesh_ipv4().map(|ip| ip.to_string());
        let body = serde_json::json!({
            "node_id": self.config.node_id,
            "peer_id": self.peer_id.to_string(),
            "mesh_ip": mesh_ip,
            "synergos_version": env!("CARGO_PKG_VERSION"),
        });
        let result = client
            .post(&self.config.heartbeat_url)
            .bearer_auth(&self.node_key)
            .json(&body)
            .send()
            .await;
        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(mesh_ip = ?mesh_ip, "control heartbeat sent");
            }
            Ok(resp) => {
                // 認証失敗・未登録などは運用者が気付くべきシグナルなので warn
                tracing::warn!(status = %resp.status(), "control heartbeat rejected");
            }
            Err(e) => {
                tracing::warn!("control heartbeat failed: {e}");
            }
        }
    }
}

fn is_trusted_plaintext_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // `Url::host_str` retains brackets around IPv6 literals (for example, `[::1]`).
    // Remove those URL delimiters before parsing the address.
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_loopback() || synergos_net::mesh_ip::is_mesh_ipv4(ip)
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::is_trusted_plaintext_host;

    #[test]
    fn plaintext_heartbeat_hosts_are_restricted() {
        for url in [
            "http://127.0.0.1:4250/v1/heartbeat",
            "http://[::1]:4250/v1/heartbeat",
            "http://localhost:4250/v1/heartbeat",
            "http://100.96.0.5:4250/v1/heartbeat",
        ] {
            assert!(is_trusted_plaintext_host(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "http://10.0.0.5:4250/v1/heartbeat",
            "http://example.test/v1/heartbeat",
        ] {
            assert!(!is_trusted_plaintext_host(&url.parse().unwrap()), "{url}");
        }
    }
}
