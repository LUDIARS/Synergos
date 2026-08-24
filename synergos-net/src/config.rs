use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

/// synergos-net の全設定
///
/// 設定ファイルは**書いたセクション / キーだけ上書き**で、省略した部分は
/// [`NetConfig::default`] の値になる (`[quic]` と `[tunnel]` だけの最小ファイルで起動できる)。
/// サブセクションを書く場合、そのセクション内のキーは全部書く (サブ struct 側は個別 default を持たない)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
    /// 他ノードから見た、この daemon の `/peer-info` サーブレット URL
    /// (例 `http://100.96.0.5:7780`, `https://node1.example.com`)。
    /// 自己完結型招待トークン (`project invite`) に埋め込まれる。未設定なら
    /// `peer_info_listen_addr` が具体 IP のときだけそこから導出し、
    /// それも無理なら従来型 (同一 daemon 内でしか使えない) トークンになる。
    #[serde(default)]
    pub peer_info_advertised_url: Option<String>,
    /// `/peer-info` で告知する QUIC エンドポイント (例 `[2406:da14:...]:7777`)。
    /// 通常は **未設定 (= auto)** で十分。Cloudflare Tunnel が Cloudflare proxied
    /// DNS の裏でホストする公開ノードでは、proxy が UDP/QUIC を通さない (HTTPS のみ)
    /// ため、サーブレットは EC2 の **real public IPv6 / IPv4** を返してクライアントに
    /// 直結させる必要がある。
    ///
    /// 形式:
    ///   - `None` (既定) または `Some("auto")` — **自動検出** (HTTPS echo サービス
    ///     `ipv6.icanhazip.com` → `ipv4.icanhazip.com` → ローカル NIC 列挙の 3 段
    ///     fallback)。NAT/LB/CGNAT 越しでも世界から見えるアドレスが取れる。
    ///     IPv6 が ISP/ルーターで詰まる環境では IPv4 にフォールバック。
    ///     ポートは `quic.listen_addr` のものを使う。Win/Linux/macOS 共通動作
    ///   - `Some("[2406:da14:...]:7777")` — リテラル IPv6:port (固定したい時)
    ///   - `Some("3.112.56.98:7777")` — リテラル IPv4:port
    ///   - `Some("hostname.example.com:7777")` — hostname:port
    #[serde(default)]
    pub quic_advertised_addr: Option<String>,
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
    /// 管制サーバー (synergos-control) への heartbeat 報告。
    /// `heartbeat_url` が空 (既定) なら無効。設定した場合は node_id と
    /// node key (環境変数) が必須で、欠けていると起動を拒否する (fail-fast)。
    #[serde(default)]
    pub control: ControlReportConfig,
    /// 履歴ノード設定 (docs/versioning-design.md §3)。既定は無効。
    /// 有効にすると publish / 受信した各 version の実体を丸ごと保持し、
    /// 旧版の FileWant に応答する。
    #[serde(default)]
    pub history: HistoryConfig,
    /// publish / 受信時フック (docs/hooks.md)。既定は無効。
    #[serde(default)]
    pub hooks: HooksConfig,
}

/// publish / 受信時フックの daemon 単位設定。
///
/// 定義は 2 層: この `hooks` (daemon 固有、常に有効) と、プロジェクトが
/// git にコミットして共有する `<project root>/.synergos/hooks.toml`
/// (`allow_project_hooks = true` のノードだけ実行。既定 false)。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct HooksConfig {
    /// `<project root>/.synergos/hooks.toml` (リポジトリ由来) のフックを
    /// 実行してよいか。リポジトリ由来スクリプトの自動実行になるため既定 false。
    /// true にしたノードだけがプロジェクトフックを実行する opt-in。
    #[serde(default)]
    pub allow_project_hooks: bool,
    /// この daemon 固有のフック (ノードローカル、常に有効)。
    #[serde(default)]
    pub hooks: Vec<HookDef>,
}

/// フック 1 件の定義。`<project root>/.synergos/hooks.toml` の `[[hook]]` と
/// daemon 設定の `[[hooks]] hooks = [...]` の両方で同じ形を使う。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookDef {
    /// `pre-publish` | `post-publish` | `post-receive`
    pub event: String,
    /// 実行するコマンド。project root を cwd に、シェル経由で実行する。
    pub command: String,
    /// 対象ファイルの glob パターン。省略 = 全ファイル。
    #[serde(default)]
    pub r#match: Vec<String>,
    /// タイムアウト (秒)。超過したらプロセスを kill する。
    #[serde(default = "default_hook_timeout_sec")]
    pub timeout_sec: u64,
}

fn default_hook_timeout_sec() -> u64 {
    60
}

const MAX_HOOK_TIMEOUT_SEC: u64 = 24 * 60 * 60;
const MAX_HOOK_COMMAND_LEN: usize = 32 * 1024;
const MAX_HOOK_PATTERN_LEN: usize = 1024;

impl HookDef {
    /// 設定ロード時に、実行不能または意図せず無視される定義を拒否する。
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.event.as_str(),
            "pre-publish" | "post-publish" | "post-receive"
        ) {
            return Err(format!("unsupported hook event: {}", self.event));
        }
        if self.command.trim().is_empty() || self.command.len() > MAX_HOOK_COMMAND_LEN {
            return Err(format!(
                "hook command must be 1..={MAX_HOOK_COMMAND_LEN} bytes"
            ));
        }
        if self.timeout_sec == 0 || self.timeout_sec > MAX_HOOK_TIMEOUT_SEC {
            return Err(format!(
                "hook timeout_sec must be between 1 and {MAX_HOOK_TIMEOUT_SEC}"
            ));
        }
        if self
            .r#match
            .iter()
            .any(|pattern| {
                pattern.is_empty()
                    || pattern.len() > MAX_HOOK_PATTERN_LEN
                    || pattern.chars().any(char::is_control)
            })
        {
            return Err(
                format!(
                    "hook match patterns must be 1..={MAX_HOOK_PATTERN_LEN} bytes and contain no control characters"
                ),
            );
        }
        Ok(())
    }

    /// `path` (プロジェクトルート相対、`/` 区切り) が `match` に該当するか。
    /// `match` が空なら常に該当する。
    pub fn matches(&self, path: &str) -> bool {
        if self.r#match.is_empty() {
            return true;
        }
        self.r#match
            .iter()
            .any(|pattern| glob_match(pattern, path))
    }
}

/// 最小限の glob マッチャ (`*` = `/` を含まない任意文字列, `**` = 任意文字列 `/` 込み, `?` = 任意 1 文字)。
/// 外部 crate に依存せず `assets/**/*.png` のような hooks.toml の `match` パターンだけを扱う。
pub fn glob_match(pattern: &str, path: &str) -> bool {
    #[derive(Clone, Copy)]
    enum Token {
        Literal(char),
        One,
        Star,
        GlobStar,
    }

    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                tokens.push(Token::GlobStar);
                index += 2;
                // Preserve the documented `assets/**/*.png` behavior: `**/` also matches
                // zero directory levels, so the slash belongs to the globstar token.
                if chars.get(index) == Some(&'/') {
                    index += 1;
                }
            }
            '*' => {
                tokens.push(Token::Star);
                index += 1;
            }
            '?' => {
                tokens.push(Token::One);
                index += 1;
            }
            literal => {
                tokens.push(Token::Literal(literal));
                index += 1;
            }
        }
    }

    let path: Vec<char> = path.chars().collect();
    let mut previous = vec![false; path.len() + 1];
    previous[0] = true;
    for token in tokens {
        let mut current = vec![false; path.len() + 1];
        match token {
            Token::Literal(expected) => {
                for position in 1..=path.len() {
                    current[position] =
                        previous[position - 1] && path[position - 1] == expected;
                }
            }
            Token::One => {
                for position in 1..=path.len() {
                    current[position] =
                        previous[position - 1] && path[position - 1] != '/';
                }
            }
            Token::Star => {
                current[0] = previous[0];
                for position in 1..=path.len() {
                    current[position] = previous[position]
                        || (path[position - 1] != '/' && current[position - 1]);
                }
            }
            Token::GlobStar => {
                current[0] = previous[0];
                for position in 1..=path.len() {
                    current[position] = previous[position] || current[position - 1];
                }
            }
        }
        previous = current;
    }
    previous[path.len()]
}

fn default_true_auto_promote() -> bool {
    true
}

/// 履歴ノード (history node) 設定。
///
/// `enabled = true` のノードは、対象プロジェクトで publish / 受信した
/// **すべての version の実体**を `root` (既定 `<project>/.synergos/history`) に
/// 内容アドレス (ファイル全体の blake3) で保持し、旧版 `FileWant` に応答する。
/// 通常ノード (既定) は最新版だけを持ち、挙動は変わらない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryConfig {
    /// このノードを履歴ノードにする。
    #[serde(default)]
    pub enabled: bool,
    /// 対象プロジェクト ID。`"*"` は参加中すべて (既定)。
    #[serde(default = "default_history_projects")]
    pub projects: Vec<String>,
    /// 保管庫。相対パスならプロジェクトルート相対、絶対パスなら
    /// `<root>/<blake3(project_id)>/` を各プロジェクトの保管庫にする。
    #[serde(default = "default_history_root")]
    pub root: String,
    /// path ごとに残す新しい版の数。0 = 無制限。
    #[serde(default)]
    pub max_versions_per_file: u64,
    /// 版の保持期間 (日)。0 = 無制限。
    #[serde(default)]
    pub max_age_days: u64,
    /// 保管庫全体の上限バイト数。0 = 無制限。超えたら古い順に削る。
    #[serde(default)]
    pub max_bytes: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            projects: default_history_projects(),
            root: default_history_root(),
            max_versions_per_file: 0,
            max_age_days: 0,
            max_bytes: 0,
        }
    }
}

impl HistoryConfig {
    /// 指定プロジェクトを保持対象にするか (`enabled` かつ `projects` に該当)。
    pub fn covers(&self, project_id: &str) -> bool {
        self.enabled && self.projects.iter().any(|p| p == "*" || p == project_id)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.enabled {
            let trimmed = self.root.trim();
            if trimmed.is_empty() {
                return Err("history.root must not be empty when history.enabled = true".into());
            }
            if trimmed != self.root {
                return Err("history.root must not have surrounding whitespace".into());
            }
            let root = std::path::Path::new(trimmed);
            if !root.is_absolute()
                && root.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::CurDir
                            | std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err("relative history.root must stay inside the project root".into());
            }
            if self.projects.is_empty() {
                return Err(
                    "history.projects must not be empty when history.enabled = true".into(),
                );
            }
        }
        Ok(())
    }
}

fn default_history_projects() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_history_root() -> String {
    ".synergos/history".to_string()
}

/// 管制サーバーへの heartbeat 報告設定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlReportConfig {
    /// 管制サーバーの heartbeat エンドポイント
    /// (例 `http://100.96.0.10:4250/v1/heartbeat`)。空なら報告しない。
    #[serde(default)]
    pub heartbeat_url: String,
    /// 管制サーバーが発行したノード ID。
    #[serde(default)]
    pub node_id: String,
    /// ノード認証キーを格納した環境変数名。キー本体は設定ファイルに書かない。
    #[serde(default = "default_node_key_env")]
    pub node_key_env: String,
    /// heartbeat 送信間隔 (秒)。
    #[serde(default = "default_heartbeat_interval_secs")]
    pub interval_secs: u64,
}

impl Default for ControlReportConfig {
    fn default() -> Self {
        Self {
            heartbeat_url: String::new(),
            node_id: String::new(),
            node_key_env: default_node_key_env(),
            interval_secs: default_heartbeat_interval_secs(),
        }
    }
}

fn default_node_key_env() -> String {
    "SYNERGOS_NODE_KEY".to_string()
}

fn default_heartbeat_interval_secs() -> u64 {
    60
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
            peer_info_advertised_url: None,
            quic_advertised_addr: None,
            bootstrap_urls: Vec::new(),
            force_relay_only: false,
            auto_promote: true,
            control: ControlReportConfig::default(),
            history: HistoryConfig::default(),
            hooks: HooksConfig::default(),
        }
    }
}

impl NetConfig {
    /// 設定全体の妥当性を検証する。Daemon 起動時に呼ばれる。
    /// 個別の知識は各サブ struct の `validate()` に委譲する。
    pub fn validate(&self) -> Result<(), String> {
        self.stream_allocation.validate()?;
        self.history.validate()?;
        for (index, hook) in self.hooks.hooks.iter().enumerate() {
            hook.validate()
                .map_err(|error| format!("hooks.hooks[{index}]: {error}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_defaults_disabled_and_covers_wildcard() {
        let cfg = NetConfig::default();
        assert!(!cfg.history.enabled);
        assert!(!cfg.history.covers("any"));
        let enabled: HistoryConfig = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert!(enabled.covers("proj"));
        let scoped: HistoryConfig =
            serde_json::from_str(r#"{"enabled": true, "projects": ["a"]}"#).unwrap();
        assert!(scoped.covers("a"));
        assert!(!scoped.covers("b"));
        assert_eq!(scoped.root, ".synergos/history");
    }

    #[test]
    fn history_validate_rejects_empty_root_when_enabled() {
        let cfg = HistoryConfig {
            enabled: true,
            root: String::new(),
            ..HistoryConfig::default()
        };
        assert!(cfg.validate().is_err());
        let traversal = HistoryConfig {
            enabled: true,
            root: "../outside".into(),
            ..HistoryConfig::default()
        };
        assert!(traversal.validate().is_err());
        let project_root = HistoryConfig {
            enabled: true,
            root: ".".into(),
            ..HistoryConfig::default()
        };
        assert!(project_root.validate().is_err());
        assert!(HistoryConfig::default().validate().is_ok());
    }

    /// 手順書 (docs/two-node-operations.md 等) の最小設定 = [quic] と [tunnel] だけ +
    /// トップレベル数キー、で読めること。以前は missing field \`mesh\` で起動できなかった。
    #[test]
    fn minimal_toml_with_only_quic_and_tunnel_sections_parses() {
        let text = r#"
peer_info_listen_addr = "0.0.0.0:7780"
quic_advertised_addr = "0.0.0.0:4433"
peer_info_advertised_url = "http://100.96.0.5:7780"
auto_promote = false

[quic]
listen_addr = "[::]:4433"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

[tunnel]
api_token_ref = ""
hostname = ""
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
"#;
        let cfg: NetConfig = toml::from_str(text).expect("minimal config must parse");
        assert!(!cfg.auto_promote);
        assert_eq!(cfg.quic.listen_addr.unwrap().port(), 4433);
        assert_eq!(
            cfg.peer_info_advertised_url.as_deref(),
            Some("http://100.96.0.5:7780")
        );
        // 省略したセクションは既定値
        let d = NetConfig::default();
        assert_eq!(cfg.mesh.doh_endpoint, d.mesh.doh_endpoint);
        assert_eq!(cfg.gossipsub.mesh_n, d.gossipsub.mesh_n);
        assert!(!cfg.history.enabled);
        // 空ファイルも既定値で読める
        let empty: NetConfig = toml::from_str("").unwrap();
        assert_eq!(
            empty.quic.max_concurrent_streams,
            d.quic.max_concurrent_streams
        );
    }

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
        assert!(
            cfg.auto_promote,
            "legacy config should default auto_promote to true"
        );

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

    #[test]
    fn hooks_default_disabled_and_empty() {
        let cfg = NetConfig::default();
        assert!(!cfg.hooks.allow_project_hooks);
        assert!(cfg.hooks.hooks.is_empty());
    }

    #[test]
    fn hook_def_toml_roundtrip() {
        let text = r#"
event = "post-receive"
command = "python scripts/convert.py"
match = ["assets/**/*.png"]
timeout_sec = 120
"#;
        let def: HookDef = toml::from_str(text).expect("hook def parses");
        assert_eq!(def.event, "post-receive");
        assert_eq!(def.timeout_sec, 120);
        assert_eq!(def.r#match, vec!["assets/**/*.png".to_string()]);
    }

    #[test]
    fn hook_def_timeout_defaults_to_60() {
        let text = r#"
event = "pre-publish"
command = "true"
"#;
        let def: HookDef = toml::from_str(text).expect("hook def parses");
        assert_eq!(def.timeout_sec, 60);
        assert!(def.r#match.is_empty());
    }

    #[test]
    fn hook_validation_rejects_invalid_definitions() {
        let valid = HookDef {
            event: "pre-publish".into(),
            command: "true".into(),
            r#match: vec![],
            timeout_sec: 60,
        };
        assert!(valid.validate().is_ok());

        for invalid in [
            HookDef {
                event: "pre-publsih".into(),
                ..valid.clone()
            },
            HookDef {
                command: "  ".into(),
                ..valid.clone()
            },
            HookDef {
                command: "x".repeat(MAX_HOOK_COMMAND_LEN + 1),
                ..valid.clone()
            },
            HookDef {
                timeout_sec: 0,
                ..valid.clone()
            },
            HookDef {
                timeout_sec: MAX_HOOK_TIMEOUT_SEC + 1,
                ..valid.clone()
            },
            HookDef {
                r#match: vec![String::new()],
                ..valid.clone()
            },
        ] {
            assert!(invalid.validate().is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn hook_def_matches_empty_pattern_matches_everything() {
        let def = HookDef {
            event: "post-receive".into(),
            command: "true".into(),
            r#match: vec![],
            timeout_sec: 60,
        };
        assert!(def.matches("anything/at/all.txt"));
    }

    #[test]
    fn glob_match_star_does_not_cross_slash() {
        assert!(glob_match("*.png", "a.png"));
        assert!(!glob_match("*.png", "dir/a.png"));
        assert!(glob_match("assets/*.png", "assets/a.png"));
        assert!(!glob_match("assets/*.png", "assets/sub/a.png"));
    }

    #[test]
    fn glob_match_double_star_crosses_slash() {
        assert!(glob_match("assets/**/*.png", "assets/a.png"));
        assert!(glob_match("assets/**/*.png", "assets/sub/a.png"));
        assert!(glob_match("assets/**/*.png", "assets/sub/deep/a.png"));
        assert!(!glob_match("assets/**/*.png", "other/a.png"));
    }

    #[test]
    fn glob_match_question_mark_matches_single_char_not_slash() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "a/c"));
    }
}
