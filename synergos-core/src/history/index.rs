//! 履歴ノードの索引 `index.json` と object sidecar の型・入出力。
//!
//! 索引はプロジェクトごとの保管庫 (`HistoryStore::store_dir`) 直下に置く。
//! 書き込みは同一ディレクトリの tmp → rename で原子的に行う。
//! 破損時は object の横に置いた `<hash>.meta.json` (sidecar) を走査して
//! 再構築できる ([`rebuild_from_sidecars`])。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 索引ファイル名 (保管庫直下)。
pub const INDEX_FILE: &str = "index.json";
/// object 置き場 (保管庫直下)。
pub const OBJECTS_DIR: &str = "objects";
/// sidecar の拡張子。
pub const META_SUFFIX: &str = ".meta.json";

/// 1 版分の索引エントリ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    /// ファイル全体の blake3 (hex)。object のアドレス。
    pub hash: String,
    pub size: u64,
    pub crc: u32,
    /// 保管した時刻 (epoch ms)。
    pub stored_at: u64,
    /// この版を publish したピア (受信なら送信元)。
    #[serde(default)]
    pub publisher: String,
    /// `"published"` (自ノードの publish) / `"received"` (受信)。
    #[serde(default)]
    pub source: String,
    /// 外部ストレージへ退避済みなら Some。ローカルの objects/sidecar は
    /// 既に削除されており、`backend`/`key` で取り戻す (spec: archive-rotation)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offloaded: Option<OffloadedRef>,
}

/// 退避先の参照。`key` は content-addressed のまま (`<project_id_hash>/<hash>`)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OffloadedRef {
    /// `"s3"` | `"local_path"` | `"gdrive"`
    pub backend: String,
    pub key: String,
    /// 退避時の保存先設定。後から daemon 設定が変わっても既存版を取り戻せるよう、
    /// 認証情報そのものではなく設定上の参照先を保持する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<synergos_net::config::RotationBackendConfig>,
}

/// 保管庫の索引本体。`entries[rel][version]`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryIndex {
    #[serde(default = "default_format")]
    pub format: u32,
    pub project_id: String,
    #[serde(default)]
    pub entries: BTreeMap<String, BTreeMap<u64, IndexEntry>>,
}

fn default_format() -> u32 {
    1
}

impl HistoryIndex {
    pub fn new(project_id: &str) -> Self {
        Self {
            format: 1,
            project_id: project_id.to_string(),
            entries: BTreeMap::new(),
        }
    }

    pub fn get(&self, rel: &str, version: u64) -> Option<&IndexEntry> {
        self.entries.get(rel).and_then(|m| m.get(&version))
    }

    pub fn insert(&mut self, rel: &str, version: u64, entry: IndexEntry) {
        self.entries
            .entry(rel.to_string())
            .or_default()
            .insert(version, entry);
    }

    pub fn remove(&mut self, rel: &str, version: u64) -> Option<IndexEntry> {
        let removed = self.entries.get_mut(rel)?.remove(&version);
        if self.entries.get(rel).is_some_and(|m| m.is_empty()) {
            self.entries.remove(rel);
        }
        removed
    }

    /// 索引から参照されている object hash の集合。
    pub fn referenced_hashes(&self) -> std::collections::BTreeSet<String> {
        self.entries
            .values()
            .flat_map(|m| m.values().map(|e| e.hash.clone()))
            .collect()
    }

    /// 全エントリを (rel, version, entry) で列挙する。
    pub fn iter_all(&self) -> impl Iterator<Item = (&str, u64, &IndexEntry)> {
        self.entries.iter().flat_map(|(rel, versions)| {
            versions
                .iter()
                .map(move |(version, entry)| (rel.as_str(), *version, entry))
        })
    }

    /// 読み込み。無ければ空。壊れていれば sidecar から再構築する。
    pub async fn load_or_rebuild(store_dir: &Path, project_id: &str) -> io::Result<Self> {
        let path = store_dir.join(INDEX_FILE);
        match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<HistoryIndex>(&bytes) {
                Ok(index) if index.format != 1 || index.project_id != project_id => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "history index belongs to project {} (format {}), expected {project_id}",
                        index.project_id, index.format
                    ),
                )),
                Ok(index)
                    if !index
                        .iter_all()
                        .all(|(rel, version, entry)| {
                            crate::manifest::safe_rel_to_local(rel).is_some()
                                && version > 0
                                && version < u64::MAX
                                && is_valid_object_hash(&entry.hash)
                        }) =>
                {
                    tracing::warn!(
                        "history index {} contains an invalid object hash; rebuilding from sidecars",
                        path.display()
                    );
                    let rebuilt = rebuild_from_sidecars(store_dir, project_id).await?;
                    rebuilt.save(store_dir).await?;
                    Ok(rebuilt)
                }
                Ok(index) => Ok(index),
                Err(error) => {
                    tracing::warn!(
                        "history index {} is corrupt ({error}); rebuilding from sidecars",
                        path.display()
                    );
                    let rebuilt = rebuild_from_sidecars(store_dir, project_id).await?;
                    rebuilt.save(store_dir).await?;
                    Ok(rebuilt)
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::new(project_id)),
            Err(error) => Err(error),
        }
    }

    /// 原子的に保存 (同一ディレクトリ tmp → rename)。
    pub async fn save(&self, store_dir: &Path) -> io::Result<()> {
        tokio::fs::create_dir_all(store_dir).await?;
        let path = store_dir.join(INDEX_FILE);
        let tmp = store_dir.join(format!("index-{}.tmp", uuid::Uuid::new_v4()));
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        tokio::fs::write(&tmp, json).await?;
        if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        Ok(())
    }
}

/// object の横に置く復旧用 sidecar。同じ内容 (hash) を複数の (rel, version)
/// が指すことがあるので参照の一覧を持つ。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ObjectMeta {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub refs: Vec<ObjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectRef {
    pub rel: String,
    pub version: u64,
    pub size: u64,
    pub crc: u32,
    pub stored_at: u64,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub source: String,
    /// object 実体が外部へ退避済みでも sidecar から索引を復旧できるよう保持する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offloaded: Option<OffloadedRef>,
}

/// `objects/<hh>/<hash>` のパス。
pub fn object_path(store_dir: &Path, hash: &str) -> PathBuf {
    let shard = if hash.len() >= 2 { &hash[..2] } else { "00" };
    store_dir.join(OBJECTS_DIR).join(shard).join(hash)
}

pub fn is_valid_object_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// `objects/<hh>/<hash>.meta.json` のパス。
pub fn meta_path(store_dir: &Path, hash: &str) -> PathBuf {
    let mut p = object_path(store_dir, hash);
    let name = format!(
        "{}{}",
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        META_SUFFIX
    );
    p.set_file_name(name);
    p
}

/// sidecar を読み、参照を 1 件追加して原子的に書き戻す。
pub async fn append_object_ref(
    store_dir: &Path,
    project_id: &str,
    hash: &str,
    reference: ObjectRef,
) -> io::Result<()> {
    let path = meta_path(store_dir, hash);
    let mut meta = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice::<ObjectMeta>(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => ObjectMeta::default(),
        Err(error) => return Err(error),
    };
    meta.project_id = project_id.to_string();
    meta.refs
        .retain(|r| !(r.rel == reference.rel && r.version == reference.version));
    meta.refs.push(reference);
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let json = serde_json::to_vec_pretty(&meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(&tmp, json).await?;
    if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    Ok(())
}

/// sidecar から参照を除く。参照が空になったら sidecar 自体を消す。
pub async fn remove_object_ref(
    store_dir: &Path,
    hash: &str,
    rel: &str,
    version: u64,
) -> io::Result<()> {
    let path = meta_path(store_dir, hash);
    let mut meta = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice::<ObjectMeta>(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    meta.refs
        .retain(|r| !(r.rel == rel && r.version == version));
    if meta.refs.is_empty() {
        return match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let json = serde_json::to_vec_pretty(&meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, json).await?;
    if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    Ok(())
}

/// `objects/` 配下の sidecar を全走査して索引を組み立て直す。
pub async fn rebuild_from_sidecars(store_dir: &Path, project_id: &str) -> io::Result<HistoryIndex> {
    let mut index = HistoryIndex::new(project_id);
    let objects = store_dir.join(OBJECTS_DIR);
    let mut shards = match tokio::fs::read_dir(&objects).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(index),
        Err(error) => return Err(error),
    };
    while let Some(shard) = shards.next_entry().await? {
        if !shard.file_type().await?.is_dir() {
            continue;
        }
        let mut files = tokio::fs::read_dir(shard.path()).await?;
        while let Some(file) = files.next_entry().await? {
            let name = file.file_name().to_string_lossy().to_string();
            let Some(hash) = name.strip_suffix(META_SUFFIX) else {
                continue;
            };
            if !is_valid_object_hash(hash) {
                continue;
            }
            let Ok(bytes) = tokio::fs::read(file.path()).await else {
                continue;
            };
            let Ok(meta) = serde_json::from_slice::<ObjectMeta>(&bytes) else {
                continue;
            };
            if !meta.project_id.is_empty() && meta.project_id != project_id {
                continue;
            }
            let object_exists = tokio::fs::metadata(object_path(store_dir, hash)).await.is_ok();
            for r in meta.refs {
                if crate::manifest::safe_rel_to_local(&r.rel).is_none()
                    || r.version == 0
                    || r.version == u64::MAX
                {
                    continue;
                }
                // ローカル実体も退避先参照も無いエントリは復旧不能なので除外する。
                if !object_exists && r.offloaded.is_none() {
                    continue;
                }
                index.insert(
                    &r.rel,
                    r.version,
                    IndexEntry {
                        hash: hash.to_string(),
                        size: r.size,
                        crc: r.crc,
                        stored_at: r.stored_at,
                        publisher: r.publisher,
                        source: r.source,
                        offloaded: r.offloaded,
                    },
                );
            }
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hash: &str, stored_at: u64) -> IndexEntry {
        IndexEntry {
            hash: hash.into(),
            size: 3,
            crc: 7,
            stored_at,
            publisher: "peer".into(),
            source: "published".into(),
            offloaded: None,
        }
    }

    #[test]
    fn insert_remove_and_reference_set() {
        let mut index = HistoryIndex::new("p");
        index.insert("a.bin", 1, entry("aa11", 1));
        index.insert("a.bin", 2, entry("bb22", 2));
        index.insert("b.bin", 1, entry("aa11", 3));
        assert_eq!(index.referenced_hashes().len(), 2);
        assert!(index.remove("a.bin", 1).is_some());
        assert!(index.get("a.bin", 1).is_none());
        assert!(index.get("a.bin", 2).is_some());
        assert!(index.remove("b.bin", 1).is_some());
        assert!(!index.entries.contains_key("b.bin"));
        assert_eq!(index.iter_all().count(), 1);
    }

    #[test]
    fn object_and_meta_paths_are_sharded() {
        let root = Path::new("store");
        assert_eq!(
            object_path(root, "abcdef"),
            root.join("objects").join("ab").join("abcdef")
        );
        assert_eq!(
            meta_path(root, "abcdef"),
            root.join("objects").join("ab").join("abcdef.meta.json")
        );
    }

    #[tokio::test]
    async fn corrupt_index_is_rebuilt_from_sidecars() {
        let dir =
            std::env::temp_dir().join(format!("synergos-hist-index-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let hash = "cafe0123cafe0123cafe0123cafe0123cafe0123cafe0123cafe0123cafe0123";
        tokio::fs::create_dir_all(object_path(&dir, hash).parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(object_path(&dir, hash), b"xyz")
            .await
            .unwrap();
        append_object_ref(
            &dir,
            "p",
            hash,
            ObjectRef {
                rel: "a.bin".into(),
                version: 4,
                size: 3,
                crc: 9,
                stored_at: 10,
                publisher: "peer".into(),
                source: "received".into(),
                offloaded: None,
            },
        )
        .await
        .unwrap();
        tokio::fs::write(dir.join(INDEX_FILE), b"{ not json")
            .await
            .unwrap();

        let index = HistoryIndex::load_or_rebuild(&dir, "p").await.unwrap();
        let e = index.get("a.bin", 4).expect("rebuilt entry");
        assert_eq!(e.hash, hash);
        assert_eq!(e.crc, 9);
        // 再構築結果は保存されている
        let again = HistoryIndex::load_or_rebuild(&dir, "p").await.unwrap();
        assert_eq!(again, index);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn index_rejects_other_project() {
        let dir =
            std::env::temp_dir().join(format!("synergos-hist-index-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        HistoryIndex::new("p1").save(&dir).await.unwrap();
        assert!(HistoryIndex::load_or_rebuild(&dir, "p2").await.is_err());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
