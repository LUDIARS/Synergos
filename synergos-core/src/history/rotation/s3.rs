//! `s3` バックエンド — AWS S3 (または互換 API)。
//!
//! `object_store` crate (`object_store::aws::AmazonS3Builder`) を使う。選定理由:
//! `aws-sdk-s3` はフル SDK で依存グラフが重く、この機能に必要なのは
//! put/get/head/delete の 4 操作だけ。`object_store` はそれを単一の薄い
//! `ObjectStore` trait で提供し、datafusion/deltalake 等で実績があるため
//! 認証まわりは環境変数 / web identity / ECS / IMDS に委ねる。
//! 認証情報は設定ファイルに書かない。

use std::io;
use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, PutPayload};

use super::backend::RotationBackend;

pub struct S3Backend {
    bucket: String,
    prefix: String,
    region: String,
}

impl S3Backend {
    pub fn new(bucket: String, prefix: String, region: String) -> Self {
        Self {
            bucket,
            prefix,
            region,
        }
    }

    fn store(&self) -> io::Result<Box<dyn ObjectStore>> {
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(&self.bucket)
            .with_region(&self.region);
        // AmazonS3Builder::from_env() が AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY /
        // AWS_SESSION_TOKEN / AWS_ENDPOINT (互換 API 用) を拾う。未設定でも
        // IMDS (EC2 インスタンスロール) 等へフォールバックする object_store の
        // 既定動作に任せる。
        if let Ok(endpoint) = std::env::var("AWS_ENDPOINT") {
            builder = builder.with_endpoint(endpoint);
        }
        let store = builder
            .build()
            .map_err(|error| io::Error::other(format!("s3 backend config invalid: {error}")))?;
        Ok(Box::new(store))
    }

    fn object_path(&self, key: &str) -> ObjectPath {
        if self.prefix.is_empty() {
            ObjectPath::from(key)
        } else {
            ObjectPath::from(format!(
                "{}/{key}",
                self.prefix.trim_end_matches('/')
            ))
        }
    }
}

#[async_trait]
impl RotationBackend for S3Backend {
    async fn put(&self, key: &str, path: &Path) -> io::Result<()> {
        let store = self.store()?;
        let bytes = tokio::fs::read(path).await?;
        store
            .put(&self.object_path(key), PutPayload::from_bytes(Bytes::from(bytes)))
            .await
            .map_err(|error| io::Error::other(format!("s3 put failed: {error}")))?;
        Ok(())
    }

    async fn get(&self, key: &str, dest: &Path) -> io::Result<()> {
        let store = self.store()?;
        let result = store
            .get(&self.object_path(key))
            .await
            .map_err(|error| io::Error::other(format!("s3 get failed: {error}")))?;
        let bytes = result
            .bytes()
            .await
            .map_err(|error| io::Error::other(format!("s3 get body read failed: {error}")))?;
        tokio::fs::write(dest, &bytes).await?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> io::Result<bool> {
        let store = self.store()?;
        match store.head(&self.object_path(key)).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(error) => Err(io::Error::other(format!("s3 head failed: {error}"))),
        }
    }

    async fn size(&self, key: &str) -> io::Result<Option<u64>> {
        let store = self.store()?;
        match store.head(&self.object_path(key)).await {
            Ok(metadata) => u64::try_from(metadata.size)
                .map(Some)
                .map_err(|_| io::Error::other("s3 object size does not fit in u64")),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(io::Error::other(format!("s3 head failed: {error}"))),
        }
    }

    async fn delete(&self, key: &str) -> io::Result<()> {
        let store = self.store()?;
        match store.delete(&self.object_path(key)).await {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(io::Error::other(format!("s3 delete failed: {error}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ネットワーク非依存: key → ObjectPath の組み立てだけを検証する
    /// (実環境疎通は PR 説明に手動手順を記載する仕様どおり)。
    #[test]
    fn object_path_applies_prefix() {
        let backend = S3Backend::new("bucket".into(), "myteam/".into(), "ap-northeast-1".into());
        assert_eq!(
            backend.object_path("aa/bb").to_string(),
            "myteam/aa/bb"
        );
        let no_prefix = S3Backend::new("bucket".into(), String::new(), "ap-northeast-1".into());
        assert_eq!(no_prefix.object_path("aa/bb").to_string(), "aa/bb");
    }
}
