//! 外部ストレージバックエンド抽象。実装は 3 種 ([`super::S3Backend`] /
//! [`super::LocalDirBackend`] / [`super::GdriveBackend`])。

use std::io;
use std::path::Path;

use async_trait::async_trait;

/// 退避先ストレージへの put/get/exists/size/delete。`key` は
/// [`super::object_key`] が組み立てる `<project_id_hash>/<blake3 hex>`。
#[async_trait]
pub trait RotationBackend: Send + Sync {
    /// `path` の内容を `key` として保存する。
    async fn put(&self, key: &str, path: &Path) -> io::Result<()>;
    /// `key` の内容を `dest` に取得する。
    async fn get(&self, key: &str, dest: &Path) -> io::Result<()>;
    /// `key` が存在するか。
    async fn exists(&self, key: &str) -> io::Result<bool>;
    /// `key` の保存済みバイト数。存在しなければ `None`。
    async fn size(&self, key: &str) -> io::Result<Option<u64>>;
    /// `key` を削除する (最後の参照を取り戻した後の外部コピー回収用)。
    async fn delete(&self, key: &str) -> io::Result<()>;
}
