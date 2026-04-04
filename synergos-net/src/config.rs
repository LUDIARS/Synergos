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
    pub catalog: CatalogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    /// Cloudflare API Token の参照キー
    pub api_token_ref: String,
    /// Tunnel の公開ホスト名（空の場合は自動生成）
    pub hostname: String,
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
    /// 帯域比率の合計が 100 であることを検証する
    pub fn validate(&self) -> std::result::Result<(), String> {
        let total = self.large_ratio as u16 + self.medium_ratio as u16 + self.small_ratio as u16;
        if total != 100 {
            return Err(format!(
                "Stream allocation ratios must sum to 100, got {}",
                total
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogConfig {
    /// チャンクあたりの最大ファイル数
    pub chunk_max_files: usize,
    /// ファイルチェーンの最大深度
    pub chain_max_depth: usize,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            tunnel: TunnelConfig {
                api_token_ref: String::new(),
                hostname: String::new(),
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
                enable_0rtt: true,
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
            catalog: CatalogConfig {
                chunk_max_files: 256,
                chain_max_depth: 10,
            },
        }
    }
}
