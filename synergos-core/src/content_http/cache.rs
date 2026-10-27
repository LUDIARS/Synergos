//! Exclusively owned disk cache. Only hash-verified, synced blocks are reusable.
use super::{validate_cid, MAX_BLOCK_BYTES};
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use synergos_net::{
    content::{Block, ContentStore},
    error::SynergosNetError,
    types::Cid,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct BlockCache {
    root: PathBuf,
    // File lifetime owns the OS lock, including all early returns.
    _lock: File,
}

pub(crate) struct TemporaryFile(pub PathBuf);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        // Best effort cleanup of this operation's unique, unpublished temporary file.
        let _ = std::fs::remove_file(&self.0);
    }
}

impl BlockCache {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(".http-transfer.lock"))?;
        lock.try_lock().context("content cache is already in use")?;
        Ok(Self { root, _lock: lock })
    }

    pub async fn load(&self, cid: &str) -> Result<Option<Block>> {
        validate_cid(cid)?;
        let path = self.root.join(cid);
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            meta.file_type().is_file(),
            "cache entry is not a regular file"
        );
        if meta.len() > MAX_BLOCK_BYTES as u64 {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        tokio::fs::File::open(&path)
            .await?
            .take(MAX_BLOCK_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await?;
        let block = Block {
            cid: Cid(cid.into()),
            bytes,
        };
        if block.bytes.len() > MAX_BLOCK_BYTES || !block.verify() {
            return Ok(None);
        }
        Ok(Some(block))
    }

    pub async fn save(&self, block: Block) -> Result<()> {
        validate_cid(&block.cid.0)?;
        ensure!(
            block.bytes.len() <= MAX_BLOCK_BYTES && block.verify(),
            "invalid cache block"
        );
        if self.load(&block.cid.0).await?.is_some() {
            return Ok(());
        }
        let temp = TemporaryFile(self.root.join(format!(".{}.part", uuid::Uuid::new_v4())));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp.0)
            .await?;
        file.write_all(&block.bytes).await?;
        file.sync_all().await?;
        drop(file);
        // Cache lock excludes cooperating writers. Replace only this CID's corrupt cache.
        let dest = self.root.join(&block.cid.0);
        match tokio::fs::remove_file(&dest).await {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        tokio::fs::rename(&temp.0, &dest).await?;
        Ok(())
    }
}

#[async_trait]
impl ContentStore for BlockCache {
    async fn get(&self, cid: &Cid) -> synergos_net::error::Result<Option<Block>> {
        self.load(&cid.0)
            .await
            .map_err(|e| SynergosNetError::Transfer(e.to_string()))
    }
    async fn put(&self, block: Block) -> synergos_net::error::Result<()> {
        self.save(block)
            .await
            .map_err(|e| SynergosNetError::Transfer(e.to_string()))
    }
    async fn has(&self, cid: &Cid) -> bool {
        self.load(&cid.0).await.is_ok_and(|b| b.is_some())
    }
}
