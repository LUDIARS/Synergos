use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{ControlError, ControlResult};

/// synergos-control の起動設定 (TOML)。
///
/// 秘密情報 (Cloudflare API token / 管理トークン) は設定ファイルに直接書かず、
/// 環境変数名だけを指定する (RULE_CODE §14)。
#[derive(Debug, Clone, Deserialize)]
pub struct ControlConfig {
    /// 管理 API の bind 先。既定は loopback (管理面はローカル専用)。
    #[serde(default = "default_bind_addr")]
    pub bind_addr: SocketAddr,

    /// レジストリ永続化先 (JSON)。
    #[serde(default = "default_store_path")]
    pub store_path: PathBuf,

    /// 管理 API bearer トークンを格納した環境変数名。
    #[serde(default = "default_admin_token_env")]
    pub admin_token_env: String,

    pub cloudflare: CloudflareConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloudflareConfig {
    /// Cloudflare account ID (Zero Trust org が属するアカウント)。
    pub account_id: String,

    /// Cloudflare API token を格納した環境変数名。
    /// token 本体を設定ファイルに書かせない。
    #[serde(default = "default_api_token_env")]
    pub api_token_env: String,

    #[serde(default = "default_api_base")]
    pub api_base: String,
}

fn default_bind_addr() -> SocketAddr {
    "127.0.0.1:4250".parse().expect("static addr")
}

fn default_store_path() -> PathBuf {
    PathBuf::from("synergos-control-store.json")
}

fn default_admin_token_env() -> String {
    "SYNERGOS_CONTROL_ADMIN_TOKEN".to_string()
}

fn default_api_token_env() -> String {
    "CLOUDFLARE_API_TOKEN".to_string()
}

fn default_api_base() -> String {
    "https://api.cloudflare.com/client/v4".to_string()
}

/// 実行時に解決済みの秘密情報。Debug 出力に値を含めない。
#[derive(Clone)]
pub struct ResolvedSecrets {
    pub admin_token: String,
    pub cloudflare_api_token: String,
}

impl std::fmt::Debug for ResolvedSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedSecrets").finish_non_exhaustive()
    }
}

impl ControlConfig {
    pub fn load(path: &Path) -> ControlResult<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| ControlError::Config(format!("failed to read {}: {e}", path.display())))?;
        let config: Self = toml::from_str(&raw).map_err(|e| {
            ControlError::Config(format!("failed to parse {}: {e}", path.display()))
        })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> ControlResult<()> {
        if self.cloudflare.account_id.trim().is_empty() {
            return Err(ControlError::Config(
                "cloudflare.account_id must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// 必須の秘密情報を起動時に解決する。欠けていたら即エラー (fail-fast, §7.1)。
    pub fn resolve_secrets(&self) -> ControlResult<ResolvedSecrets> {
        let admin_token = read_secret_env(&self.admin_token_env)?;
        if admin_token.as_bytes().len() < 32 {
            return Err(ControlError::Config(format!(
                "required secret env var {} must contain at least 32 bytes",
                self.admin_token_env
            )));
        }
        let cloudflare_api_token = read_secret_env(&self.cloudflare.api_token_env)?;
        Ok(ResolvedSecrets {
            admin_token,
            cloudflare_api_token,
        })
    }
}

fn read_secret_env(name: &str) -> ControlResult<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(ControlError::Config(format!(
            "required secret env var {name} is not set (refusing to start without it)"
        ))),
    }
}
