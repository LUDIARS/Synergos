//! `local_path` バックエンド — 外部 HDD 等のマウント先へ単純コピー + fsync する。
//!
//! マウント先に到達できない (パスが存在しない) 場合は明確なエラーを返す。
//! 呼び出し元 ([`super::rotate`]) はこれを「退避をスキップして警告」として扱う。

use std::io;
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::backend::RotationBackend;

pub struct LocalDirBackend {
    root: PathBuf,
}

impl LocalDirBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dest_path(&self, key: &str) -> io::Result<PathBuf> {
        // Windows の `\\` も区切りとして解釈されるため、`..` だけでなく生成規則
        // (`<64 hex>/<64 hex>`) そのものを検証する。
        if !super::is_valid_object_key(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsafe rotation key: {key}"),
            ));
        }
        Ok(self.root.join(key))
    }

    async fn ensure_mounted(&self) -> io::Result<()> {
        tokio::fs::metadata(&self.root).await.map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "local_path backend root is not reachable ({}): {error}",
                    self.root.display()
                ),
            )
        })?;
        Ok(())
    }
}

#[async_trait]
impl RotationBackend for LocalDirBackend {
    async fn put(&self, key: &str, path: &Path) -> io::Result<()> {
        self.ensure_mounted().await?;
        let dest = self.dest_path(key)?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = dest.with_file_name(format!(".tmp-{}", uuid::Uuid::new_v4()));
        let bytes = tokio::fs::read(path).await?;
        {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            // Windows は書き込みハンドルが開いたままだと rename (MoveFileExW) を
            // ERROR_ACCESS_DENIED で拒否するため、明示的に close してから戻る。
            drop(file);
        }
        if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        Ok(())
    }

    async fn get(&self, key: &str, dest: &Path) -> io::Result<()> {
        self.ensure_mounted().await?;
        let src = self.dest_path(key)?;
        tokio::fs::copy(&src, dest).await?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> io::Result<bool> {
        self.ensure_mounted().await?;
        let path = self.dest_path(key)?;
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn size(&self, key: &str) -> io::Result<Option<u64>> {
        self.ensure_mounted().await?;
        let path = self.dest_path(key)?;
        match tokio::fs::metadata(&path).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn delete(&self, key: &str) -> io::Result<()> {
        self.ensure_mounted().await?;
        let path = self.dest_path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_exists_delete_roundtrip() {
        let root = std::env::temp_dir().join(format!("synergos-rot-local-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let backend = LocalDirBackend::new(root.clone());

        let src = root.join("src.bin");
        tokio::fs::write(&src, b"hello world").await.unwrap();

        let key = format!("{}/{}", "a".repeat(64), "b".repeat(64));
        assert!(!backend.exists(&key).await.unwrap());
        backend.put(&key, &src).await.unwrap();
        assert!(backend.exists(&key).await.unwrap());
        assert_eq!(backend.size(&key).await.unwrap(), Some(11));

        let dest = root.join("dest.bin");
        backend.get(&key, &dest).await.unwrap();
        assert_eq!(tokio::fs::read(&dest).await.unwrap(), b"hello world");

        backend.delete(&key).await.unwrap();
        assert!(!backend.exists(&key).await.unwrap());
        // 冪等
        backend.delete(&key).await.unwrap();

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn unreachable_root_returns_clear_error() {
        let missing = std::env::temp_dir().join(format!("synergos-rot-missing-{}", uuid::Uuid::new_v4()));
        let backend = LocalDirBackend::new(missing);
        let src = std::env::temp_dir().join(format!("synergos-rot-src-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&src, b"x").await.unwrap();

        let error = backend.put("aa/bb", &src).await.unwrap_err();
        assert!(error.to_string().contains("not reachable"));
        let _ = tokio::fs::remove_file(&src).await;
    }

    #[tokio::test]
    async fn unsafe_key_is_rejected() {
        let root = std::env::temp_dir().join(format!("synergos-rot-local-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let backend = LocalDirBackend::new(root.clone());
        let src = root.join("src.bin");
        tokio::fs::write(&src, b"x").await.unwrap();

        for bad in ["../escape", "a/../b", "a//b", "aa\\..\\escape", "C:\\escape"] {
            assert!(backend.put(bad, &src).await.is_err());
        }
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
