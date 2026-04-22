//! ContentStore: CID → block のキー/値ストア。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::error::{Result, SynergosNetError};
use crate::types::Cid;

use super::block::Block;

/// 任意の content-addressed store。put / get / has の最小 API。
/// 実装は Send + Sync なのでマルチスレッド環境で共有できる。
#[async_trait]
pub trait ContentStore: Send + Sync {
    async fn put(&self, block: Block) -> Result<()>;
    async fn get(&self, cid: &Cid) -> Result<Option<Block>>;
    async fn has(&self, cid: &Cid) -> bool;
}

/// インメモリ実装。テスト / 小規模キャッシュ用。
#[derive(Default)]
pub struct MemoryContentStore {
    inner: Arc<RwLock<HashMap<Cid, Vec<u8>>>>,
}

impl MemoryContentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

#[async_trait]
impl ContentStore for MemoryContentStore {
    async fn put(&self, block: Block) -> Result<()> {
        if !block.verify() {
            return Err(SynergosNetError::Transfer("cid/bytes mismatch".into()));
        }
        self.inner.write().await.insert(block.cid, block.bytes);
        Ok(())
    }

    async fn get(&self, cid: &Cid) -> Result<Option<Block>> {
        Ok(self.inner.read().await.get(cid).map(|bytes| Block {
            cid: cid.clone(),
            bytes: bytes.clone(),
        }))
    }

    async fn has(&self, cid: &Cid) -> bool {
        self.inner.read().await.contains_key(cid)
    }
}

/// ファイルシステム実装。`<root>/<2-char prefix>/<rest>` に block を保存する
/// (Git / IPFS object store 様の分割)。ファイル名は CID 文字列そのまま。
pub struct FileContentStore {
    root: PathBuf,
}

impl FileContentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, cid: &Cid) -> PathBuf {
        let s = cid.0.as_str();
        let (prefix, rest) = if s.len() >= 2 {
            s.split_at(2)
        } else {
            ("zz", s)
        };
        self.root.join(prefix).join(rest)
    }
}

#[async_trait]
impl ContentStore for FileContentStore {
    async fn put(&self, block: Block) -> Result<()> {
        if !block.verify() {
            return Err(SynergosNetError::Transfer("cid/bytes mismatch".into()));
        }
        let path = self.path_for(&block.cid);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, &block.bytes).await?;
        Ok(())
    }

    async fn get(&self, cid: &Cid) -> Result<Option<Block>> {
        let path = self.path_for(cid);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let candidate = Block {
                    cid: cid.clone(),
                    bytes,
                };
                if !candidate.verify() {
                    return Err(SynergosNetError::Transfer(format!(
                        "stored block at {} failed verification",
                        path.display()
                    )));
                }
                Ok(Some(candidate))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SynergosNetError::Io(e)),
        }
    }

    async fn has(&self, cid: &Cid) -> bool {
        tokio::fs::metadata(self.path_for(cid)).await.is_ok()
    }
}

/// Convenience: `Path` が `FileContentStore::new(path)` を返す fn として使えるよう。
#[allow(dead_code)]
pub fn open_file_store(path: &Path) -> FileContentStore {
    FileContentStore::new(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_put_get_has_roundtrip() {
        let s = MemoryContentStore::new();
        let b = Block::new(b"abc".to_vec());
        let cid = b.cid.clone();
        s.put(b).await.unwrap();
        assert!(s.has(&cid).await);
        let got = s.get(&cid).await.unwrap().unwrap();
        assert_eq!(got.bytes, b"abc");
    }

    #[tokio::test]
    async fn memory_rejects_cid_mismatch() {
        let s = MemoryContentStore::new();
        let bad = Block {
            cid: Cid("blake3-deadbeef".into()),
            bytes: b"not matching".to_vec(),
        };
        assert!(s.put(bad).await.is_err());
    }

    #[tokio::test]
    async fn file_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("cs-{}", uuid::Uuid::new_v4()));
        let s = FileContentStore::new(&dir);
        let b = Block::new(b"disk".to_vec());
        let cid = b.cid.clone();
        s.put(b).await.unwrap();
        assert!(s.has(&cid).await);
        let got = s.get(&cid).await.unwrap().unwrap();
        assert_eq!(got.bytes, b"disk");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_store_detects_tampering() {
        let dir = std::env::temp_dir().join(format!("cs-tamp-{}", uuid::Uuid::new_v4()));
        let s = FileContentStore::new(&dir);
        let b = Block::new(b"original".to_vec());
        let cid = b.cid.clone();
        s.put(b).await.unwrap();
        // ディスク上の block を書き換える
        let path = s.path_for(&cid);
        tokio::fs::write(&path, b"tampered").await.unwrap();
        // get で検証失敗
        assert!(s.get(&cid).await.is_err());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
