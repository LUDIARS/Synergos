use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

/// synergos-net の全設定
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    pub tunnel: TunnelConfig,
    pub mesh: MeshConfig,
    pub quic: QuicConfig,
    pub dht: DhtConfig,
    pub gossipsub: GossipsubConfig,
    pub stream_allocation: StreamAllocationConfig,
    pub speed_test: SpeedTestConfig,
    pub peer_selection: PeerSelectionConfig,
    pub monitor: MonitorConfig,
    /// CatalogManager のチューニング (マジックナンバー解消、後方互換のため
    /// `#[serde(default)]` で旧 config からも読める)。
    #[serde(default)]
    pub catalog: CatalogConfig,
    /// Peer-info HTTP servlet (bootstrap endpoint) を起動する listen アドレス。
    /// `None` (既定) ではサーブレットを起動しない。AWS 公開ノード等では
    /// `127.0.0.1:7780` を設定し、Cloudflare Tunnel 等で外部に publish する想定。
    #[serde(default)]
    pub peer_info_listen_addr: Option<SocketAddr>,
    /// 起動時に自動 bootstrap する peer-info サーブレット URL 群。
    /// 各 URL に対して `peer add-url` 相当 (`HTTPS GET /peer-info` → QUIC connect)
    /// を非同期に発火し、成功・失敗とも `tracing::info` / `warn` で記録する。
    /// 失敗しても daemon 起動は継続する (best-effort)。
    /// 例: `["https://node1.example.com", "https://node2.example.com"]`
    #[serde(default)]
    pub bootstrap_urls: Vec<String>,
    /// **Relay-only モード**。`true` のとき:
    ///   - ピア接続時に IPv6 Direct / Tunnel を試さず、必ず WebSocket Relay
    ///     (`synergos-relay`) を経由する。
    ///   - 自ノードは direct 経路を route 通知に含めない (匿名化)。
    ///
    /// 自宅 PC が peer 一覧で見えないようにしたい / すべての通信を AWS 等の
    /// 中継サーバ経由に強制したいときに有効化する。
    #[serde(default)]
    pub force_relay_only: bool,
    /// **自動昇格モード**。既定 `true`。
    /// 起動時に IPv6 / UPnP / Cloudflare Tunnel の到達性を probe し、
    /// いずれも不可なら effective relay-only として動作する。`force_relay_only`
    /// が true の場合は probe を実行せず常に relay-only。
    /// 手動で「probe しない」運用にしたいときだけ false に。
    #[serde(default = "default_true_auto_promote")]
    pub auto_promote: bool,
}

fn default_true_auto_promote() -> bool {
    true
}

/// CatalogManager のチューニングパラメータ。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    /// 1 チャンクあたりの最大ファイル数 (default: 256)
    pub chunk_max_files: usize,
    /// FileChain の最大保持深度 (default: 10)
    pub chain_max_depth: usize,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            chunk_max_files: 256,
            chain_max_depth: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    /// Cloudflare API Token の参照キー
    pub api_token_ref: String,
    /// Tunnel の公開ホスト名（空の場合は自動生成）
    pub hostname: String,
    /// cloudflared バイナリが未検出でもシミュレーションモードで成功扱いにするか。
    /// `false`（既定）ではバイナリが無い場合はエラーを返す。
    /// 開発時のみ `true` を検討する。
    #[serde(default)]
    pub allow_simulation: bool,
    /// cloudflared プロセスが crash したとき自動再起動するか (supervisor 有効化)。
    /// 既定 `true`。`false` にすると 1 回起動して exit したらそれで終わり。
    #[serde(default = "default_true")]
    pub auto_restart: bool,
    /// supervisor 再起動の初期バックオフ (ms)。`restart_base_ms × 2^N` で N は連続失敗回数。
    #[serde(default = "default_restart_base")]
    pub restart_base_ms: u64,
    /// supervisor 再起動の最大バックオフ (ms)。
    #[serde(default = "default_restart_max")]
    pub restart_max_ms: u64,
}

fn default_true() -> bool {
    true
}
fn default_restart_base() -> u64 {
    1_000
}
fn default_restart_max() -> u64 {
    60_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConfig {
    /// DNS-over-HTTPS エンドポイント
    pub doh_endpoint: String,
    /// 自前 DNS サーバー
    pub dns_servers: Vec<String>,
    /// TURN サーバー一覧
    pub turn_servers: Vec<TurnServerConfig>,
    /// STUN サーバー一覧
    pub stun_servers: Vec<String>,
    /// IPv6 到達性プローブのタイムアウト (ms)
    pub probe_timeout_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnServerConfig {
    pub uri: String,
    pub username: String,
    pub credential_ref: String,
    pub auth_method: TurnAuthMethod,
    pub token_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TurnAuthMethod {
    LongTerm,
    EphemeralRest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicConfig {
    /// 最大同時ストリーム数
    pub max_concurrent_streams: u32,
    /// アイドルタイムアウト (ms)
    pub idle_timeout_ms: u64,
    /// 最大 UDP ペイロードサイズ
    pub max_udp_payload_size: u16,
    /// 0-RTT を有効にするか
    pub enable_0rtt: bool,
    /// QUIC server がバインドする listen アドレス。
    /// `None` (既定) は `[::]:0` 相当 (IPv6/IPv4 デュアルスタックでカーネル割当ポート)。
    /// 公開ノードでは `[::]:7777` 等の固定ポートを設定する。
    /// 後方互換のため `#[serde(default)]` で旧 config からも読める。
    #[serde(default)]
    pub listen_addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtConfig {
    /// Kademlia k-bucket サイズ
    pub k_bucket_size: usize,
    /// ルーティングテーブル更新間隔 (秒)
    pub routing_refresh_secs: u64,
    /// ピアのアクティブ情報 TTL (秒)
    pub peer_ttl_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipsubConfig {
    /// メッシュの目標ピア数
    pub mesh_n: usize,
    /// メッシュの下限
    pub mesh_n_low: usize,
    /// メッシュの上限
    pub mesh_n_high: usize,
    /// ハートビート間隔 (ms)
    pub heartbeat_interval_ms: u64,
    /// メッセージキャッシュ保持数
    pub message_cache_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamAllocationConfig {
    /// Large ファイルの帯域比率
    pub large_ratio: u8,
    /// Medium ファイルの帯域比率
    pub medium_ratio: u8,
    /// Small ファイルの帯域比率
    pub small_ratio: u8,
}

impl StreamAllocationConfig {
    /// 3 つの比率の合計が 100 になっているかを検証する。
    /// 合計が 100 でないと帯域配分計算が壊れる。
    pub fn validate(&self) -> Result<(), String> {
        let sum = self.large_ratio as u16 + self.medium_ratio as u16 + self.small_ratio as u16;
        if sum != 100 {
            return Err(format!(
                "stream_allocation ratios must sum to 100 (got {sum}: large={}, medium={}, small={})",
                self.large_ratio, self.medium_ratio, self.small_ratio
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTestConfig {
    /// スピードテストを有効にするか
    pub enabled: bool,
    /// スピードテストの再実施間隔 (秒)
    pub retest_interval_secs: u64,
    /// プローブパケット数
    pub probe_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerSelectionConfig {
    /// 帯域スコアの重み (0.0 - 1.0)
    pub bandwidth_weight: f64,
    /// 安定性スコアの重み (0.0 - 1.0)
    pub stability_weight: f64,
    /// スコア再計算間隔 (秒)
    pub recalculate_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    /// スナップショット収集間隔 (ms)
    pub snapshot_interval_ms: u64,
    /// 履歴保持数
    pub history_size: usize,
    /// 帯域履歴のサンプリング間隔 (秒)
    pub graph_sample_interval_secs: u64,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            tunnel: TunnelConfig {
                api_token_ref: String::new(),
                hostname: String::new(),
                allow_simulation: false,
                auto_restart: true,
                restart_base_ms: 1_000,
                restart_max_ms: 60_000,
            },
            mesh: MeshConfig {
                doh_endpoint: "https://cloudflare-dns.com/dns-query".into(),
                dns_servers: vec![],
                turn_servers: vec![],
                stun_servers: vec![],
                probe_timeout_ms: 3000,
            },
            quic: QuicConfig {
                max_concurrent_streams: 100,
                idle_timeout_ms: 30000,
                max_udp_payload_size: 1452,
                // 0-RTT はリプレイ攻撃の余地があるため既定は OFF。
                // 明示的にリスクを受容する運用のみ true に設定する。
                enable_0rtt: false,
                listen_addr: None,
            },
            dht: DhtConfig {
                k_bucket_size: 20,
                routing_refresh_secs: 60,
                peer_ttl_secs: 120,
            },
            gossipsub: GossipsubConfig {
                mesh_n: 6,
                mesh_n_low: 4,
                mesh_n_high: 12,
                heartbeat_interval_ms: 1000,
                message_cache_size: 1000,
            },
            stream_allocation: StreamAllocationConfig {
                large_ratio: 60,
                medium_ratio: 30,
                small_ratio: 10,
            },
            speed_test: SpeedTestConfig {
                enabled: true,
                retest_interval_secs: 300,
                probe_count: 10,
            },
            peer_selection: PeerSelectionConfig {
                bandwidth_weight: 0.7,
                stability_weight: 0.3,
                recalculate_interval_secs: 60,
            },
            monitor: MonitorConfig {
                snapshot_interval_ms: 1000,
                history_size: 3600,
                graph_sample_interval_secs: 1,
            },
            catalog: CatalogConfig::default(),
            peer_info_listen_addr: None,
            bootstrap_urls: Vec::new(),
            force_relay_only: false,
            auto_promote: true,
        }
    }
}

impl NetConfig {
    /// 設定全体の妥当性を検証する。Daemon 起動時に呼ばれる。
    /// 個別の知識は各サブ struct の `validate()` に委譲する。
    pub fn validate(&self) -> Result<(), String> {
        self.stream_allocation.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_listen_addr_defaults_to_none() {
        let cfg = NetConfig::default();
        assert!(cfg.quic.listen_addr.is_none());
    }

    #[test]
    fn quic_listen_addr_serde_roundtrip_explicit() {
        // listen_addr が指定された QuicConfig は string -> SocketAddr で読み戻せること。
        let json = r#"{
            "max_concurrent_streams": 100,
            "idle_timeout_ms": 30000,
            "max_udp_payload_size": 1452,
            "enable_0rtt": false,
            "listen_addr": "[::]:7777"
        }"#;
        let qcfg: QuicConfig = serde_json::from_str(json).expect("json parse");
        let addr = qcfg.listen_addr.expect("listen_addr present");
        assert_eq!(addr.to_string(), "[::]:7777");
    }

    #[test]
    fn quic_listen_addr_serde_roundtrip_omitted() {
        // listen_addr フィールドが無い旧 config (serde default) からも読めること。
        let json = r#"{
            "max_concurrent_streams": 100,
            "idle_timeout_ms": 30000,
            "max_udp_payload_size": 1452,
            "enable_0rtt": false
        }"#;
        let qcfg: QuicConfig = serde_json::from_str(json).expect("json parse");
        assert!(qcfg.listen_addr.is_none());
    }

    #[test]
    fn bootstrap_urls_defaults_to_empty() {
        let cfg = NetConfig::default();
        assert!(cfg.bootstrap_urls.is_empty());
    }

    #[test]
    fn peer_info_listen_addr_defaults_to_none() {
        let cfg = NetConfig::default();
        assert!(cfg.peer_info_listen_addr.is_none());
    }

    #[test]
    fn force_relay_only_defaults_to_false() {
        let cfg = NetConfig::default();
        assert!(!cfg.force_relay_only);
    }

    #[test]
    fn auto_promote_defaults_to_true() {
        let cfg = NetConfig::default();
        assert!(cfg.auto_promote);
    }

    #[test]
    fn force_relay_only_serde_roundtrip() {
        // 旧 config (force_relay_only フィールドが無い JSON) からも読めること。
        // 同様に PR-1〜4 で追加された listen_addr / peer_info_listen_addr /
        // bootstrap_urls も `#[serde(default)]` でこの legacy JSON から読めるはず。
        let legacy = r#"{
            "tunnel": {"api_token_ref": "", "hostname": ""},
            "mesh": {"doh_endpoint": "", "dns_servers": [], "turn_servers": [], "stun_servers": [], "probe_timeout_ms": 3000},
            "quic": {"max_concurrent_streams": 100, "idle_timeout_ms": 30000, "max_udp_payload_size": 1452, "enable_0rtt": false},
            "dht": {"k_bucket_size": 20, "routing_refresh_secs": 60, "peer_ttl_secs": 120},
            "gossipsub": {"mesh_n": 6, "mesh_n_low": 4, "mesh_n_high": 12, "heartbeat_interval_ms": 1000, "message_cache_size": 1000},
            "stream_allocation": {"large_ratio": 60, "medium_ratio": 30, "small_ratio": 10},
            "speed_test": {"enabled": true, "retest_interval_secs": 300, "probe_count": 10},
            "peer_selection": {"bandwidth_weight": 0.7, "stability_weight": 0.3, "recalculate_interval_secs": 60},
            "monitor": {"snapshot_interval_ms": 1000, "history_size": 3600, "graph_sample_interval_secs": 1}
        }"#;
        let cfg: NetConfig = serde_json::from_str(legacy).expect("legacy config should parse");
        assert!(!cfg.force_relay_only);
        assert!(cfg.auto_promote, "legacy config should default auto_promote to true");

        // 明示的に true を指定した JSON も読める
        let with_flag = r#"{
            "tunnel": {"api_token_ref": "", "hostname": ""},
            "mesh": {"doh_endpoint": "", "dns_servers": [], "turn_servers": [], "stun_servers": [], "probe_timeout_ms": 3000},
            "quic": {"max_concurrent_streams": 100, "idle_timeout_ms": 30000, "max_udp_payload_size": 1452, "enable_0rtt": false},
            "dht": {"k_bucket_size": 20, "routing_refresh_secs": 60, "peer_ttl_secs": 120},
            "gossipsub": {"mesh_n": 6, "mesh_n_low": 4, "mesh_n_high": 12, "heartbeat_interval_ms": 1000, "message_cache_size": 1000},
            "stream_allocation": {"large_ratio": 60, "medium_ratio": 30, "small_ratio": 10},
            "speed_test": {"enabled": true, "retest_interval_secs": 300, "probe_count": 10},
            "peer_selection": {"bandwidth_weight": 0.7, "stability_weight": 0.3, "recalculate_interval_secs": 60},
            "monitor": {"snapshot_interval_ms": 1000, "history_size": 3600, "graph_sample_interval_secs": 1},
            "force_relay_only": true
        }"#;
        let cfg: NetConfig =
            serde_json::from_str(with_flag).expect("config with flag should parse");
        assert!(cfg.force_relay_only);
    }
}
