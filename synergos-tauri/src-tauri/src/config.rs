//! synergos-net.toml の read/write を Tauri 側で行う。
//!
//! 設計判断: 設定を daemon に IPC で送るより、ファイルを直接書いて再起動を促す
//! 方が単純。daemon は起動時に config を読むだけで hot reload も無いため、
//! IPC 経由にしてもどのみち再起動が要る。
//!
//! "Settings" UI で edit したいフィールドだけを `ConfigFile` に出す。
//! その他のフィールド (gossip mesh_n 等) は touched しないために raw `toml::Value`
//! で全体を保持し、編集対象のキーだけ上書きする。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPaths {
    pub config_path: String,
    pub identity_path: String,
    pub projects_path: String,
}

/// Settings UI が編集する設定フィールドのサブセット。
/// `synergos-net.toml` の主要キーに対応する。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigFile {
    /// 起動時 bootstrap する peer-info URL 群
    pub bootstrap_urls: Vec<String>,
    /// Relay-only モードを強制するか
    pub force_relay_only: bool,
    /// 起動時 NAT 越え probe を行うか (true ならデフォルト動作)
    pub auto_promote: bool,
    /// QUIC server bind アドレス (例 `[::]:7777`)。空ならデフォルト
    pub quic_listen_addr: Option<String>,
    /// peer-info HTTP servlet listen アドレス (例 `127.0.0.1:7780`)。空なら無効
    pub peer_info_listen_addr: Option<String>,
}

pub fn paths() -> ConfigPaths {
    let config_path = config_file_path();
    let identity_path = identity_dir().join("identity");
    let projects_path = identity_dir().join("projects.json");
    ConfigPaths {
        config_path: config_path.display().to_string(),
        identity_path: identity_path.display().to_string(),
        projects_path: projects_path.display().to_string(),
    }
}

pub fn load() -> anyhow::Result<ConfigFile> {
    let path = config_file_path();
    if !path.exists() {
        return Ok(ConfigFile {
            auto_promote: true,
            ..Default::default()
        });
    }
    let raw = std::fs::read_to_string(&path)?;
    let value: toml::Value = toml::from_str(&raw)?;

    let bootstrap_urls = value
        .get("bootstrap_urls")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let force_relay_only = value
        .get("force_relay_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let auto_promote = value
        .get("auto_promote")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let quic_listen_addr = value
        .get("quic")
        .and_then(|v| v.get("listen_addr"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let peer_info_listen_addr = value
        .get("peer_info_listen_addr")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(ConfigFile {
        bootstrap_urls,
        force_relay_only,
        auto_promote,
        quic_listen_addr,
        peer_info_listen_addr,
    })
}

pub fn save(cfg: &ConfigFile) -> anyhow::Result<()> {
    let path = config_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // 既存 toml を読み、編集対象キーだけ上書きする (gossip mesh 等は touch しない)
    let existing = if path.exists() {
        let raw = std::fs::read_to_string(&path)?;
        toml::from_str::<toml::Value>(&raw)?
    } else {
        toml::Value::Table(toml::value::Table::new())
    };
    let mut table = existing
        .as_table()
        .cloned()
        .unwrap_or_else(toml::value::Table::new);

    table.insert(
        "bootstrap_urls".into(),
        toml::Value::Array(
            cfg.bootstrap_urls
                .iter()
                .map(|s| toml::Value::String(s.clone()))
                .collect(),
        ),
    );
    table.insert(
        "force_relay_only".into(),
        toml::Value::Boolean(cfg.force_relay_only),
    );
    table.insert(
        "auto_promote".into(),
        toml::Value::Boolean(cfg.auto_promote),
    );
    if let Some(v) = &cfg.peer_info_listen_addr {
        table.insert(
            "peer_info_listen_addr".into(),
            toml::Value::String(v.clone()),
        );
    } else {
        table.remove("peer_info_listen_addr");
    }

    // [quic] section 内の listen_addr のみ編集
    let mut quic_table = table
        .get("quic")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    if let Some(v) = &cfg.quic_listen_addr {
        quic_table.insert("listen_addr".into(), toml::Value::String(v.clone()));
    } else {
        quic_table.remove("listen_addr");
    }
    if !quic_table.is_empty() {
        table.insert("quic".into(), toml::Value::Table(quic_table));
    }

    let serialized = toml::to_string_pretty(&toml::Value::Table(table))?;
    std::fs::write(&path, serialized)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn identity_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata).join("synergos")
    } else {
        PathBuf::from(".").join("synergos")
    }
}

#[cfg(target_os = "macos")]
fn identity_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("synergos")
    } else {
        PathBuf::from(".").join("synergos")
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn identity_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("synergos")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("synergos")
    } else {
        PathBuf::from(".").join("synergos")
    }
}

fn config_file_path() -> PathBuf {
    identity_dir().join("synergos-net.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 設定ファイル全体を取り出す save の roundtrip をシミュレートする
    /// (パス操作は os 依存なので path は touch せず、toml 構築ロジックだけ確認)。
    #[test]
    fn config_file_serializes_to_toml_keys() {
        let cfg = ConfigFile {
            bootstrap_urls: vec!["https://node1.example.com".into()],
            force_relay_only: false,
            auto_promote: true,
            quic_listen_addr: Some("[::]:7777".into()),
            peer_info_listen_addr: Some("127.0.0.1:7780".into()),
        };

        let mut t = toml::value::Table::new();
        t.insert(
            "bootstrap_urls".into(),
            toml::Value::Array(
                cfg.bootstrap_urls
                    .iter()
                    .map(|s| toml::Value::String(s.clone()))
                    .collect(),
            ),
        );
        t.insert(
            "force_relay_only".into(),
            toml::Value::Boolean(cfg.force_relay_only),
        );
        t.insert(
            "auto_promote".into(),
            toml::Value::Boolean(cfg.auto_promote),
        );
        t.insert(
            "peer_info_listen_addr".into(),
            toml::Value::String(cfg.peer_info_listen_addr.clone().unwrap()),
        );
        let mut quic = toml::value::Table::new();
        quic.insert(
            "listen_addr".into(),
            toml::Value::String(cfg.quic_listen_addr.clone().unwrap()),
        );
        t.insert("quic".into(), toml::Value::Table(quic));

        let s = toml::to_string_pretty(&toml::Value::Table(t)).unwrap();
        assert!(s.contains("bootstrap_urls"));
        assert!(s.contains("https://node1.example.com"));
        assert!(s.contains("force_relay_only = false"));
        assert!(s.contains("[quic]"));
        assert!(s.contains("listen_addr = \"[::]:7777\""));
    }
}
