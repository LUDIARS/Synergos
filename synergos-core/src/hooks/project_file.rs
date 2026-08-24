//! `<project root>/.synergos/hooks.toml` の読み込み。
//!
//! git にコミットしてチームで共有するプロジェクト単位のフック定義。
//! `[[hook]]` テーブル配列 1 個 = `HookDef` 1 個。ファイルが無ければ空 (エラーにしない)。

use std::path::Path;

use serde::{Deserialize, Serialize};
use synergos_net::config::HookDef;
use tokio::io::AsyncReadExt;

use crate::manifest::META_DIR;

/// `.synergos/hooks.toml` のファイル名。
pub const HOOKS_FILE: &str = "hooks.toml";
const MAX_HOOKS_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct ProjectHooksFile {
    #[serde(default, rename = "hook")]
    hooks: Vec<HookDef>,
}

/// `<root>/.synergos/hooks.toml` を読み込む。ファイルが存在しなければ空 `Vec`。
/// 壊れた TOML はエラーとして返す (呼び出し側が警告ログを出す)。
pub async fn load(root: &Path) -> std::io::Result<Vec<HookDef>> {
    let path = root.join(META_DIR).join(HOOKS_FILE);
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut text = String::new();
    let bytes_read = file
        .take(MAX_HOOKS_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .await?;
    if bytes_read as u64 > MAX_HOOKS_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("hooks.toml exceeds {MAX_HOOKS_FILE_BYTES} bytes"),
        ));
    }
    let file: ProjectHooksFile = toml::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    for (index, hook) in file.hooks.iter().enumerate() {
        hook.validate().map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("hook[{index}]: {error}"),
            )
        })?;
    }
    Ok(file.hooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = load(dir.path()).await.unwrap();
        assert!(hooks.is_empty());
    }

    #[tokio::test]
    async fn parses_hook_toml() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(META_DIR);
        tokio::fs::create_dir_all(&meta).await.unwrap();
        tokio::fs::write(
            meta.join(HOOKS_FILE),
            r#"
[[hook]]
event = "post-receive"
command = "python scripts/convert.py"
match = ["assets/**/*.png"]
timeout_sec = 120

[[hook]]
event = "pre-publish"
command = "true"
"#,
        )
        .await
        .unwrap();
        let hooks = load(dir.path()).await.unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].event, "post-receive");
        assert_eq!(hooks[0].timeout_sec, 120);
        assert_eq!(hooks[1].event, "pre-publish");
        assert_eq!(hooks[1].timeout_sec, 60);
    }

    #[tokio::test]
    async fn malformed_toml_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(META_DIR);
        tokio::fs::create_dir_all(&meta).await.unwrap();
        tokio::fs::write(meta.join(HOOKS_FILE), "not valid toml [[[").await.unwrap();
        assert!(load(dir.path()).await.is_err());
    }

    #[tokio::test]
    async fn invalid_hook_definition_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(META_DIR);
        tokio::fs::create_dir_all(&meta).await.unwrap();
        tokio::fs::write(
            meta.join(HOOKS_FILE),
            "[[hook]]\nevent = \"post-recieve\"\ncommand = \"true\"\n",
        )
        .await
        .unwrap();
        assert!(load(dir.path()).await.is_err());
    }

    #[tokio::test]
    async fn oversized_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(META_DIR);
        tokio::fs::create_dir_all(&meta).await.unwrap();
        tokio::fs::write(
            meta.join(HOOKS_FILE),
            vec![b' '; MAX_HOOKS_FILE_BYTES as usize + 1],
        )
        .await
        .unwrap();
        assert!(load(dir.path()).await.is_err());
    }
}
