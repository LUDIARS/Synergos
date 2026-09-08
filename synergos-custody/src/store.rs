// @implements SPEC-SY-custody-durability
use crate::{
    contract::Admission,
    durable_file::{self, PartialFile},
    error::{Error, Result},
    signature::object_key,
};
use axum::body::Body;
use chrono::Utc;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex as AsyncMutex, OwnedMutexGuard},
};

pub struct Store {
    root: PathBuf,
    // OS lock lives as long as this store. Process death releases it; no stale PID lockfiles.
    _process_lock: std::fs::File,
    locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}
impl Store {
    pub async fn open(root: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        let root = tokio::fs::canonicalize(root).await?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("custody.lock"))?;
        lock.try_lock().map_err(io::Error::other)?;
        // A unique owner may safely reap parts from a previous terminated process.
        let mut entries = tokio::fs::read_dir(&root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".part") {
                if uuid::Uuid::parse_str(stem).is_ok() && entry.file_type().await?.is_file() {
                    tokio::fs::remove_file(entry.path()).await?;
                }
            }
        }
        let probe = PartialFile(root.join(format!("{}.part", uuid::Uuid::new_v4())));
        durable_file::write_new(&probe.0, b"custody publication probe").await?;
        Ok(Self {
            root,
            _process_lock: lock,
            locks: Mutex::new(HashMap::new()),
        })
    }
    pub async fn lock(&self, key: &str) -> Result<OwnedMutexGuard<()>> {
        let mutex = {
            let mut locks = self.locks.lock().map_err(|_| Error::Unavailable)?;
            if locks.len() > 1024 {
                locks.retain(|_, value| value.strong_count() > 0);
            }
            if let Some(existing) = locks.get(key).and_then(Weak::upgrade) {
                existing
            } else {
                let created = Arc::new(AsyncMutex::new(()));
                locks.insert(key.to_owned(), Arc::downgrade(&created));
                created
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), mutex.lock_owned())
            .await
            .map_err(|_| Error::Unavailable)
    }
    pub fn path(&self, key: &str, suffix: &str) -> PathBuf {
        self.root.join(format!("{key}.{suffix}"))
    }
    pub async fn admission(&self, key: &str) -> Result<Admission> {
        if tokio::fs::try_exists(self.path(key, "reclaimed")).await? {
            return Err(Error::Conflict);
        }
        let bytes = tokio::fs::read(self.path(key, "json"))
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Error::NotFound
                } else {
                    Error::Unavailable
                }
            })?;
        serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)
    }
    pub async fn admit(&self, input: &Admission) -> Result<String> {
        let operation_key = object_key(&format!("operation:{}", input.operation_id));
        let _operation_guard = self.lock(&operation_key).await?;
        let key = object_key(&input.opaque_object_id);
        let _guard = self.lock(&key).await?;
        let operation_path = self.path(&operation_key, "operation");
        match tokio::fs::read(&operation_path).await {
            Ok(bytes) => {
                let previous: Admission =
                    serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)?;
                if previous != *input {
                    return Err(Error::Conflict);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                durable_file::write_new(
                    &operation_path,
                    &serde_json::to_vec(input).map_err(|_| Error::Invalid)?,
                )
                .await?
            }
            Err(error) => return Err(error.into()),
        }
        match self.admission(&key).await {
            Ok(previous) if previous == *input => return Ok(key),
            Ok(_) => return Err(Error::Conflict),
            Err(Error::NotFound) => {}
            Err(error) => return Err(error),
        }
        durable_file::write_new(
            &self.path(&key, "json"),
            &serde_json::to_vec(input).map_err(|_| Error::Invalid)?,
        )
        .await?;
        Ok(key)
    }
    /// Presence check for read paths: the object is published only after `put` has
    /// verified and synced it, so size agreement is the invariant to re-assert here.
    /// Rehashing on every read would re-read the whole object before serving a byte.
    pub async fn present(&self, key: &str, input: &Admission) -> Result<bool> {
        match tokio::fs::symlink_metadata(self.path(key, "data")).await {
            Ok(metadata) if metadata.is_file() && metadata.len() == input.byte_length => Ok(true),
            Ok(_) => Err(Error::Conflict),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    /// Full re-read for receipt issuance: the receipt asserts durability, so it must
    /// re-derive the digest from the medium and require `sync_all` to succeed.
    pub async fn verify_file(&self, key: &str, input: &Admission) -> Result<bool> {
        if !self.present(key, input).await? {
            return Ok(false);
        }
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.path(key, "data"))
            .await?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 128 * 1024];
        let mut length = 0u64;
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            length += n as u64;
            hash.update(&buffer[..n]);
        }
        if length != input.byte_length
            || format!("sha256:{:x}", hash.finalize()) != input.content_hash
        {
            return Err(Error::Conflict);
        }
        file.sync_all().await?;
        Ok(true)
    }
    pub async fn put(&self, key: &str, input: &Admission, body: Body) -> Result<()> {
        if input.expired(Utc::now()) {
            return Err(Error::Conflict);
        }
        if self.present(key, input).await? {
            return Err(Error::Precondition);
        }
        let partial = PartialFile(self.root.join(format!("{}.part", uuid::Uuid::new_v4())));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial.0)
            .await?;
        let mut stream = body.into_data_stream();
        let mut hash = Sha256::new();
        let mut length = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| Error::Invalid)?;
            length = length
                .checked_add(chunk.len() as u64)
                .ok_or(Error::Invalid)?;
            if length > input.byte_length {
                return Err(Error::Invalid);
            }
            file.write_all(&chunk).await?;
            hash.update(&chunk);
        }
        if length != input.byte_length
            || format!("sha256:{:x}", hash.finalize()) != input.content_hash
            || input.expired(Utc::now())
        {
            return Err(Error::Conflict);
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        durable_file::publish(&partial.0, &self.path(key, "data")).await?;
        Ok(())
    }
    pub async fn reclaim(&self, key: &str) -> Result<()> {
        let _guard = self.lock(key).await?;
        if !tokio::fs::try_exists(self.path(key, "reclaimed")).await? {
            durable_file::write_new(&self.path(key, "reclaimed"), b"cancelled\n").await?;
        }
        match tokio::fs::remove_file(self.path(key, "data")).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        // Keep admission and tombstone so old PUT capabilities can never recreate data.
        Ok(())
    }
}
