//! Tunnel Manager — Cloudflare Tunnel (cloudflared) プロセス制御
//!
//! cloudflared プロセスの起動・停止・ヘルスチェックを管理する。
//! QUIC トランスポート経由でピア接続を中継する。

use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::config::TunnelConfig;
use crate::error::{Result, SynergosNetError};

/// Tunnel の状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelState {
    /// 停止中
    Idle,
    /// 起動中
    Starting,
    /// 稼働中
    Active { tunnel_id: String, uptime_secs: u64 },
    /// エラー
    Error { message: String },
}

/// Tunnel ヘルスチェック結果
#[derive(Debug, Clone)]
pub struct TunnelHealth {
    pub reachable: bool,
    pub rtt_ms: Option<u32>,
    pub last_checked: Instant,
}

/// Cloudflare Tunnel マネージャ
///
/// cloudflared プロセスのライフサイクルを管理し、
/// Tunnel 経由の QUIC 接続を仲介する。
pub struct TunnelManager {
    config: TunnelConfig,
    /// Tunnel の公開ホスト名
    pub hostname: String,
    /// Tunnel の状態
    state: RwLock<TunnelState>,
    /// cloudflared プロセスハンドル
    process: RwLock<Option<tokio::process::Child>>,
    /// ヘルスチェック結果
    health: RwLock<Option<TunnelHealth>>,
    /// 起動時刻
    started_at: RwLock<Option<Instant>>,
}

impl TunnelManager {
    pub fn new(config: &TunnelConfig) -> Self {
        Self {
            hostname: config.hostname.clone(),
            config: config.clone(),
            state: RwLock::new(TunnelState::Idle),
            process: RwLock::new(None),
            health: RwLock::new(None),
            started_at: RwLock::new(None),
        }
    }

    /// Tunnel の現在の状態を取得
    pub async fn state(&self) -> TunnelState {
        let state = self.state.read().await;
        match &*state {
            TunnelState::Active { tunnel_id, .. } => {
                let uptime = self
                    .started_at
                    .read()
                    .await
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                TunnelState::Active {
                    tunnel_id: tunnel_id.clone(),
                    uptime_secs: uptime,
                }
            }
            other => other.clone(),
        }
    }

    /// cloudflared プロセスを起動して Tunnel を確立する
    pub async fn start(&self) -> Result<String> {
        {
            let state = self.state.read().await;
            if matches!(&*state, TunnelState::Active { .. } | TunnelState::Starting) {
                return Err(SynergosNetError::Tunnel(
                    "Tunnel already active or starting".into(),
                ));
            }
        }

        *self.state.write().await = TunnelState::Starting;
        tracing::info!("Starting Cloudflare Tunnel...");

        // cloudflared プロセスを起動
        let result = self.spawn_cloudflared().await;

        match result {
            Ok(tunnel_id) => {
                *self.started_at.write().await = Some(Instant::now());
                *self.state.write().await = TunnelState::Active {
                    tunnel_id: tunnel_id.clone(),
                    uptime_secs: 0,
                };
                tracing::info!(
                    "Tunnel active: id={}, hostname={}",
                    tunnel_id,
                    self.hostname
                );
                Ok(tunnel_id)
            }
            Err(e) => {
                *self.state.write().await = TunnelState::Error {
                    message: e.to_string(),
                };
                Err(e)
            }
        }
    }

    /// Tunnel を停止する
    pub async fn stop(&self) -> Result<()> {
        tracing::info!("Stopping Cloudflare Tunnel...");

        // cloudflared プロセスを終了
        let mut process = self.process.write().await;
        if let Some(ref mut child) = *process {
            let _ = child.kill().await;
            tracing::debug!("cloudflared process killed");
        }
        *process = None;

        *self.state.write().await = TunnelState::Idle;
        *self.started_at.write().await = None;
        *self.health.write().await = None;

        Ok(())
    }

    /// Tunnel のヘルスチェックを実施
    pub async fn health_check(&self) -> TunnelHealth {
        let state = self.state.read().await;
        let reachable = matches!(&*state, TunnelState::Active { .. });

        let health = TunnelHealth {
            reachable,
            rtt_ms: if reachable { Some(0) } else { None },
            last_checked: Instant::now(),
        };

        *self.health.write().await = Some(health.clone());
        health
    }

    /// 最新のヘルスチェック結果を取得
    pub async fn last_health(&self) -> Option<TunnelHealth> {
        self.health.read().await.clone()
    }

    /// Tunnel が利用可能かどうか
    pub async fn is_available(&self) -> bool {
        matches!(&*self.state.read().await, TunnelState::Active { .. })
    }

    /// Tunnel の公開ホスト名を取得
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    // ── 内部ヘルパー ──

    /// cloudflared プロセスを起動する
    async fn spawn_cloudflared(&self) -> Result<String> {
        // hostname が指定されていれば引数注入を防ぐために先に検証する。
        // 許可する文字種: DNS サブドメインとして正当なもののみ
        // (ASCII 英数字 / ドット / ハイフン)。
        if !self.config.hostname.is_empty() && !is_valid_hostname(&self.config.hostname) {
            return Err(SynergosNetError::Tunnel(format!(
                "invalid hostname: {:?}; expected [a-zA-Z0-9.-]+",
                self.config.hostname
            )));
        }

        // cloudflared が PATH にあるか確認（Windows 含めクロスプラットフォーム）
        let cloudflared_available = locate_executable("cloudflared").is_some();

        if !cloudflared_available {
            if self.config.allow_simulation {
                tracing::warn!(
                    "cloudflared not found in PATH — running in simulation mode \
                    (allow_simulation=true)"
                );
                let tunnel_id = format!("sim-{}", uuid::Uuid::new_v4());
                return Ok(tunnel_id);
            }
            return Err(SynergosNetError::Tunnel(
                "cloudflared binary not found in PATH. Install cloudflared or \
                set tunnel.allow_simulation=true in config (development only)."
                    .into(),
            ));
        }

        // cloudflared tunnel run を起動
        let mut cmd = tokio::process::Command::new("cloudflared");
        cmd.arg("tunnel").arg("--no-autoupdate").arg("run");

        if !self.config.hostname.is_empty() {
            cmd.arg("--hostname").arg(&self.config.hostname);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| SynergosNetError::Tunnel(format!("Failed to spawn cloudflared: {}", e)))?;

        let tunnel_id = format!("cf-{}", uuid::Uuid::new_v4());

        *self.process.write().await = Some(child);

        Ok(tunnel_id)
    }
}

/// DNS ホスト名として妥当か検証。許可: `[a-zA-Z0-9.-]+`、かつ先頭/末尾が英数字。
/// cloudflared の `--hostname` にユーザー入力を渡す前の防御線。
fn is_valid_hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    let bytes = s.as_bytes();
    let ok = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'-';
    if bytes.iter().any(|&b| !ok(b)) {
        return false;
    }
    // 連続したドットや先頭/末尾の `.` / `-` を弾く
    if bytes.first().map_or(true, |&b| !b.is_ascii_alphanumeric())
        || bytes.last().map_or(true, |&b| !b.is_ascii_alphanumeric())
    {
        return false;
    }
    !s.contains("..")
}

/// PATH 上の実行ファイルを探す。Windows の `PATHEXT` (.exe / .cmd / .bat) も
/// 考慮する簡易版。見つからなければ `None`。
///
/// 純粋 std 実装にしてあるのは外部クレート `which` を追加しないため。
fn locate_executable(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;

    #[cfg(windows)]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
        .split(';')
        .map(|e| e.to_ascii_lowercase())
        .collect();

    for dir in std::env::split_paths(&path) {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        #[cfg(windows)]
        for ext in &exts {
            let with_ext = dir.join(format!("{name}{ext}"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

#[cfg(test)]
mod hostname_tests {
    use super::is_valid_hostname;

    #[test]
    fn accepts_normal_dns() {
        assert!(is_valid_hostname("tunnel.example.com"));
        assert!(is_valid_hostname("a.b"));
        assert!(is_valid_hostname("node-01.proj.example"));
    }

    #[test]
    fn rejects_injection_shapes() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname(".leading.dot"));
        assert!(!is_valid_hostname("trailing-"));
        assert!(!is_valid_hostname("double..dot"));
        assert!(!is_valid_hostname("has space"));
        assert!(!is_valid_hostname("has/slash"));
        assert!(!is_valid_hostname("semi;colon"));
        assert!(!is_valid_hostname("quote\"mark"));
        assert!(!is_valid_hostname("--flag"));
    }
}
