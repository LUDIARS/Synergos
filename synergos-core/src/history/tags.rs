//! 履歴ノードの版タグ — docs/versioning-design.md §3.5 (保持ポリシー) の追記分。
//!
//! タグは名前付きの (path → version) ピン集合 (git tag に相当)。保管庫直下
//! `<store_dir>/tags/<name>.json` に node ローカルで保存する。git には入れない
//! (索引の正は git 側 manifest というルールに反しない: タグは保持保護のための
//! ローカル指定であって第二の履歴系ではない)。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// タグ置き場 (保管庫直下)。
pub const TAGS_DIR: &str = "tags";

/// タグ名として許可する文字集合。
const NAME_PATTERN_HINT: &str = "[A-Za-z0-9._-]{1,64}";

/// 1 タグ分の内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tag {
    #[serde(default = "default_format")]
    pub format: u32,
    pub project_id: String,
    pub name: String,
    pub created_at: u64,
    /// (rel_path → version) のピン集合。
    #[serde(default)]
    pub pins: BTreeMap<String, u64>,
}

fn default_format() -> u32 {
    1
}

/// タグ一覧の 1 行 (name / created_at / pin 数)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSummary {
    pub name: String,
    pub created_at: u64,
    pub pin_count: usize,
}

/// タグ名が `[A-Za-z0-9._-]{1,64}` に一致するか。パス脱出も拒否する。
pub fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn tags_dir(store_dir: &Path) -> PathBuf {
    store_dir.join(TAGS_DIR)
}

fn tag_path(dir: &Path, name: &str) -> io::Result<PathBuf> {
    if !is_valid_tag_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tag name must match {NAME_PATTERN_HINT}: {name}"),
        ));
    }
    Ok(dir.join(format!("{name}.json")))
}

async fn open_tags_dir(store_dir: &Path) -> io::Result<Option<PathBuf>> {
    let dir = tags_dir(store_dir);
    let metadata = match tokio::fs::symlink_metadata(&dir).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "history tags path must be a real directory",
        ));
    }
    let canonical_store = tokio::fs::canonicalize(store_dir).await?;
    let canonical_dir = tokio::fs::canonicalize(&dir).await?;
    if !canonical_dir.starts_with(&canonical_store) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "history tags path resolves outside the store",
        ));
    }
    Ok(Some(canonical_dir))
}

async fn prepare_tags_dir(store_dir: &Path) -> io::Result<PathBuf> {
    tokio::fs::create_dir_all(tags_dir(store_dir)).await?;
    open_tags_dir(store_dir).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "history tags directory disappeared during creation",
        )
    })
}

async fn ensure_regular_tag_file(path: &Path) -> io::Result<bool> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "history tag path must be a regular file",
            ))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn validate_tag(tag: &Tag, project_id: &str, expected_name: &str) -> io::Result<()> {
    if tag.format != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported tag format: {}", tag.format),
        ));
    }
    if tag.project_id != project_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tag belongs to a different project",
        ));
    }
    if tag.name != expected_name || !is_valid_tag_name(&tag.name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tag name does not match its file name",
        ));
    }
    if tag.pins.iter().any(|(rel, version)| {
        crate::manifest::safe_rel_to_local(rel).is_none()
            || *version == 0
            || *version == u64::MAX
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "tag contains an invalid file path or version",
        ));
    }
    Ok(())
}

/// タグを作成/上書きする。
pub async fn save(
    store_dir: &Path,
    project_id: &str,
    name: &str,
    created_at: u64,
    pins: BTreeMap<String, u64>,
) -> io::Result<Tag> {
    let _ = tag_path(store_dir, name)?;
    if pins.iter().any(|(rel, version)| {
        crate::manifest::safe_rel_to_local(rel).is_none()
            || *version == 0
            || *version == u64::MAX
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tag contains an invalid file path or version",
        ));
    }
    let tag = Tag {
        format: 1,
        project_id: project_id.to_string(),
        name: name.to_string(),
        created_at,
        pins,
    };
    let dir = prepare_tags_dir(store_dir).await?;
    let path = tag_path(&dir, name)?;
    ensure_regular_tag_file(&path).await?;
    let tmp = dir.join(format!("{name}-{}.tmp", uuid::Uuid::new_v4()));
    let json = serde_json::to_vec_pretty(&tag)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let Err(error) = tokio::fs::write(&tmp, json).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    Ok(tag)
}

/// 1 タグを読む。無ければ `None`。
pub async fn load(store_dir: &Path, project_id: &str, name: &str) -> io::Result<Option<Tag>> {
    let _ = tag_path(store_dir, name)?;
    let Some(dir) = open_tags_dir(store_dir).await? else {
        return Ok(None);
    };
    let path = tag_path(&dir, name)?;
    if !ensure_regular_tag_file(&path).await? {
        return Ok(None);
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let tag: Tag = serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            validate_tag(&tag, project_id, name)?;
            Ok(Some(tag))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// タグを削除する (実体は消さない。以後 GC 対象に戻るだけ)。`false` = 元々無かった。
pub async fn remove(store_dir: &Path, project_id: &str, name: &str) -> io::Result<bool> {
    if load(store_dir, project_id, name).await?.is_none() {
        return Ok(false);
    }
    let Some(dir) = open_tags_dir(store_dir).await? else {
        return Ok(false);
    };
    let path = tag_path(&dir, name)?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// 全タグの一覧 (name / created_at / pin 数)。名前順。
pub async fn list(store_dir: &Path, project_id: &str) -> io::Result<Vec<TagSummary>> {
    let Some(dir) = open_tags_dir(store_dir).await? else {
        return Ok(Vec::new());
    };
    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut out = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_valid_tag_name(stem) {
            continue;
        }
        if !ensure_regular_tag_file(&entry.path()).await? {
            continue;
        }
        let bytes = tokio::fs::read(entry.path()).await?;
        let tag = serde_json::from_slice::<Tag>(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if tag.project_id != project_id {
            continue;
        }
        validate_tag(&tag, project_id, stem)?;
        out.push(TagSummary {
            name: tag.name,
            created_at: tag.created_at,
            pin_count: tag.pins.len(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 全タグの pins を (rel, version) の保護集合として合流させる。
/// 壊れたタグを無視すると GC が保護版を消し得るため、読み取り・検証失敗は返す。
pub async fn all_pins(store_dir: &Path, project_id: &str) -> io::Result<Vec<(String, u64)>> {
    let Some(dir) = open_tags_dir(store_dir).await? else {
        return Ok(Vec::new());
    };
    let mut entries = tokio::fs::read_dir(&dir).await?;
    let mut out = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_valid_tag_name(stem) {
            continue;
        }
        if !ensure_regular_tag_file(&entry.path()).await? {
            continue;
        }
        let bytes = tokio::fs::read(entry.path()).await?;
        let tag = serde_json::from_slice::<Tag>(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if tag.project_id != project_id {
            continue;
        }
        validate_tag(&tag, project_id, stem)?;
        out.extend(tag.pins.into_iter());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_store() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("synergos-tags-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[test]
    fn tag_name_validation() {
        assert!(is_valid_tag_name("release-1.0"));
        assert!(is_valid_tag_name("a"));
        assert!(is_valid_tag_name(&"a".repeat(64)));
        assert!(!is_valid_tag_name(""));
        assert!(!is_valid_tag_name(&"a".repeat(65)));
        assert!(!is_valid_tag_name(".."));
        assert!(!is_valid_tag_name("."));
        assert!(!is_valid_tag_name("../escape"));
        assert!(!is_valid_tag_name("has space"));
        assert!(!is_valid_tag_name("has/slash"));
        assert!(!is_valid_tag_name("has\\backslash"));
    }

    #[tokio::test]
    async fn save_load_remove_roundtrip() {
        let dir = temp_store().await;
        let mut pins = BTreeMap::new();
        pins.insert("assets/big.bin".to_string(), 3);
        pins.insert("levels/01.unity".to_string(), 1);
        let saved = save(&dir, "p", "release-1.0", 1_000, pins.clone())
            .await
            .unwrap();
        assert_eq!(saved.pins, pins);

        let loaded = load(&dir, "p", "release-1.0").await.unwrap().unwrap();
        assert_eq!(loaded.pins, pins);
        assert_eq!(loaded.created_at, 1_000);

        let summaries = list(&dir, "p").await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "release-1.0");
        assert_eq!(summaries[0].pin_count, 2);

        assert!(remove(&dir, "p", "release-1.0").await.unwrap());
        assert!(load(&dir, "p", "release-1.0").await.unwrap().is_none());
        assert!(!remove(&dir, "p", "release-1.0").await.unwrap());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn invalid_tag_names_are_rejected() {
        let dir = temp_store().await;
        for bad in ["", "../escape", "a/b", "a\\b", &"x".repeat(65)] {
            assert!(save(&dir, "p", bad, 0, BTreeMap::new()).await.is_err());
            assert!(load(&dir, "p", bad).await.is_err());
            assert!(remove(&dir, "p", bad).await.is_err());
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn invalid_pins_are_rejected_at_the_storage_boundary() {
        let dir = temp_store().await;
        for (rel, version) in [("../escape", 1), ("a.bin", 0), ("a.bin", u64::MAX)] {
            let mut pins = BTreeMap::new();
            pins.insert(rel.to_string(), version);
            assert!(save(&dir, "p", "release", 0, pins).await.is_err());
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn corrupt_tag_stops_pin_collection_instead_of_disabling_gc_protection() {
        let dir = temp_store().await;
        let tag_dir = dir.join(TAGS_DIR);
        tokio::fs::create_dir_all(&tag_dir).await.unwrap();
        tokio::fs::write(tag_dir.join("release.json"), b"not json")
            .await
            .unwrap();

        assert!(all_pins(&dir, "p").await.is_err());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn tag_paths_must_not_follow_symbolic_links() {
        use std::os::unix::fs::symlink;

        let dir = temp_store().await;
        let outside = temp_store().await;
        symlink(&outside, dir.join(TAGS_DIR)).unwrap();

        assert!(save(&dir, "p", "release", 0, BTreeMap::new())
            .await
            .is_err());
        assert!(load(&dir, "p", "release").await.is_err());
        assert!(!outside.join("release.json").exists());
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let _ = tokio::fs::remove_dir_all(&outside).await;
    }

    #[tokio::test]
    async fn load_rejects_tag_from_a_different_project() {
        let dir = temp_store().await;
        save(&dir, "p1", "shared-name", 0, BTreeMap::new())
            .await
            .unwrap();
        assert!(load(&dir, "p2", "shared-name").await.is_err());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn all_pins_merges_across_tags_and_ignores_other_projects() {
        let dir = temp_store().await;
        let mut pins_a = BTreeMap::new();
        pins_a.insert("a.bin".to_string(), 1);
        save(&dir, "p", "tag-a", 0, pins_a).await.unwrap();
        let mut pins_b = BTreeMap::new();
        pins_b.insert("a.bin".to_string(), 2);
        pins_b.insert("b.bin".to_string(), 5);
        save(&dir, "p", "tag-b", 0, pins_b).await.unwrap();
        save(&dir, "other", "tag-c", 0, {
            let mut m = BTreeMap::new();
            m.insert("c.bin".to_string(), 9);
            m
        })
        .await
        .unwrap();

        let mut merged = all_pins(&dir, "p").await.unwrap();
        merged.sort();
        assert_eq!(
            merged,
            vec![
                ("a.bin".to_string(), 1),
                ("a.bin".to_string(), 2),
                ("b.bin".to_string(), 5),
            ]
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn list_and_all_pins_are_empty_when_no_tags_dir() {
        let dir = temp_store().await;
        assert!(list(&dir, "p").await.unwrap().is_empty());
        assert!(all_pins(&dir, "p").await.unwrap().is_empty());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
