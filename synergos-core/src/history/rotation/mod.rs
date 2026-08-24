//! 外部ストレージローテーション (spec: archive-rotation)。
//!
//! `HistoryStore::gc` の保持ポリシー ([`super::gc::apply_retention`]) で
//! 残った旧版のうち、`stored_at` がさらに古いものを外部ストレージへ**退避**
//! (削除ではない) する。command / store.rs 内の既存 `archive()` (= 保管庫への
//! 取り込み) とは意味が違うため、こちらは一貫して `offload` / `rotate` と呼ぶ。
//!
//! - 退避 = [`RotationBackend::put`] → put 後に exists + サイズ + 再取得時の hash/CRC 検証 →
//!   index と復旧用 sidecar に `offloaded` を記録 → 他のローカル参照が無ければ
//!   objects 実体だけを削除する
//! - 取り戻し = [`RotationBackend::get`] → blake3 検証 → objects へ戻して
//!   `offloaded` マークを外す
//! - 候補選定の除外 (最新版・タグ保護) は [`super::store::HistoryStore::protected_versions`]
//!   を再利用する (gc.rs の保護ロジックの二重実装を避ける)

mod backend;
mod gdrive;
mod local;
mod s3;

pub use backend::RotationBackend;
pub use gdrive::GdriveBackend;
pub use local::LocalDirBackend;
pub use s3::S3Backend;

use std::io;
use std::path::Path;
use std::sync::Arc;

use synergos_net::config::RotationBackendConfig;

use super::index::{
    is_valid_object_hash, object_path, HistoryIndex, IndexEntry, ObjectRef, OffloadedRef,
};

/// 1 件の退避済み版 (`history offloaded` の戻り値)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffloadedVersion {
    pub rel: String,
    pub version: u64,
    pub size: u64,
    pub backend: String,
    pub key: String,
}

/// `history rotate` の結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RotationReport {
    pub offloaded: Vec<(String, u64)>,
    pub bytes_offloaded: u64,
    /// dry-run で「これから退避する」候補、または通常実行で put 前に確定した候補一覧。
    pub candidates: Vec<(String, u64)>,
    pub skipped: Vec<RotationSkip>,
}

/// 退避をスキップした件 (backend 不達など)。put 失敗時は何も変えずここに積む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotationSkip {
    pub rel: String,
    pub version: u64,
    pub reason: String,
}

/// 設定からバックエンド実装を組み立てる。
pub fn build_backend(config: &RotationBackendConfig) -> io::Result<Arc<dyn RotationBackend>> {
    config
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    match config {
        RotationBackendConfig::S3 {
            bucket,
            prefix,
            region,
        } => Ok(Arc::new(S3Backend::new(
            bucket.clone(),
            prefix.clone(),
            region.clone(),
        ))),
        RotationBackendConfig::LocalPath { path } => {
            Ok(Arc::new(LocalDirBackend::new(path.clone())))
        }
        RotationBackendConfig::Gdrive {
            folder_id,
            credentials_file,
        } => Ok(Arc::new(GdriveBackend::new(
            folder_id.clone(),
            credentials_file.clone(),
        ))),
        RotationBackendConfig::Unset => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "history.rotation.backend is not configured",
        )),
    }
}

/// content-addressed key: `<project_id_hash>/<blake3 hex>`。
pub fn object_key(project_id: &str, hash: &str) -> String {
    format!("{}/{hash}", blake3::hash(project_id.as_bytes()).to_hex())
}

fn is_valid_object_key(key: &str) -> bool {
    let mut segments = key.split('/');
    let valid_segment = |segment: &str| {
        segment.len() == 64 && segment.bytes().all(|byte| byte.is_ascii_hexdigit())
    };
    matches!(
        (segments.next(), segments.next(), segments.next()),
        (Some(project), Some(object), None) if valid_segment(project) && valid_segment(object)
    )
}

/// 候補選定 = `stored_at` が閾値超過 かつ 保護集合に無い かつ 既に offloaded でない版。
pub fn select_candidates(
    index: &HistoryIndex,
    offload_after_days: u64,
    now_ms: u64,
    protected: &[(String, u64)],
) -> Vec<(String, u64)> {
    let protected: std::collections::BTreeSet<(&str, u64)> =
        protected.iter().map(|(r, v)| (r.as_str(), *v)).collect();
    let cutoff = now_ms.saturating_sub(
        offload_after_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1000),
    );
    let mut out: Vec<(String, u64)> = index
        .iter_all()
        .filter(|(rel, version, entry)| {
            entry.offloaded.is_none()
                && entry.stored_at < cutoff
                && !protected.contains(&(*rel, *version))
        })
        .map(|(rel, version, _)| (rel.to_string(), version))
        .collect();
    out.sort();
    out
}

/// ローテーションを実行する。`dry_run = true` なら候補一覧を返すだけで何も変更しない。
pub async fn rotate(
    store_dir: &Path,
    project_id: &str,
    backend: &dyn RotationBackend,
    backend_config: &RotationBackendConfig,
    index: &mut HistoryIndex,
    offload_after_days: u64,
    now_ms: u64,
    protected: &[(String, u64)],
    dry_run: bool,
) -> io::Result<RotationReport> {
    let candidates = select_candidates(index, offload_after_days, now_ms, protected);
    let mut report = RotationReport {
        candidates: candidates.clone(),
        ..RotationReport::default()
    };
    if dry_run {
        return Ok(report);
    }

    for (rel, version) in candidates {
        let Some(entry) = index.get(&rel, version).cloned() else {
            continue;
        };
        if entry.offloaded.is_some() {
            continue;
        }
        // 同じ hash を指す他の (rel, version) が既に offload 済みなら put をスキップし
        // 索引更新だけ行う (content-addressed の dedup を維持する)。
        let already_offloaded_to_same_backend = index.iter_all().any(|(_, _, other)| {
            other.hash == entry.hash
                && other.offloaded.as_ref().is_some_and(|offloaded| {
                    offloaded.config.as_ref() == Some(backend_config)
                        || (offloaded.config.is_none()
                            && offloaded.backend == backend_name(backend_config))
                })
        });

        let key = object_key(project_id, &entry.hash);
        let obj = object_path(store_dir, &entry.hash);

        if !already_offloaded_to_same_backend {
            if let Err(error) = backend.put(&key, &obj).await {
                report.skipped.push(RotationSkip {
                    rel,
                    version,
                    reason: format!("put failed: {error}"),
                });
                continue;
            }
        }
        // dedup で put を省いた場合も、先に退避した object が現在も完全なことを
        // 確認してからローカル参照を offloaded に切り替える。
        let exists = match backend.exists(&key).await {
            Ok(exists) => exists,
            Err(error) => {
                report.skipped.push(RotationSkip {
                    rel: rel.clone(),
                    version,
                    reason: format!("post-put verification failed: {error}"),
                });
                continue;
            }
        };
        if !exists {
            report.skipped.push(RotationSkip {
                rel,
                version,
                reason: "post-put verification failed: object not found at backend".into(),
            });
            continue;
        }
        let verified = match backend.size(&key).await {
            Ok(Some(size)) if size == entry.size => true,
            Ok(Some(size)) => {
                report.skipped.push(RotationSkip {
                    rel: rel.clone(),
                    version,
                    reason: format!(
                        "post-put verification failed: expected {} bytes, backend has {size}",
                        entry.size
                    ),
                });
                continue;
            }
            Ok(None) => false,
            Err(error) => {
                report.skipped.push(RotationSkip {
                    rel: rel.clone(),
                    version,
                    reason: format!("post-put verification failed: {error}"),
                });
                continue;
            }
        };
        if !verified {
            report.skipped.push(RotationSkip {
                rel,
                version,
                reason: "post-put verification failed: object not found at backend".into(),
            });
            continue;
        }
        let verification_tmp = obj.with_extension(format!(
            "rotation-verify-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        let downloaded = backend.get(&key, &verification_tmp).await;
        let integrity = match downloaded {
            Ok(()) => validate_fetched(&verification_tmp, &entry).await,
            Err(error) => Err(error),
        };
        let _ = tokio::fs::remove_file(&verification_tmp).await;
        match integrity {
            Ok(true) => {}
            Ok(false) => {
                report.skipped.push(RotationSkip {
                    rel,
                    version,
                    reason: "post-put verification failed: downloaded object failed integrity validation"
                        .into(),
                });
                continue;
            }
            Err(error) => {
                report.skipped.push(RotationSkip {
                    rel,
                    version,
                    reason: format!("post-put verification failed: {error}"),
                });
                continue;
            }
        }

        let mut updated = entry.clone();
        updated.offloaded = Some(OffloadedRef {
            backend: backend_name(backend_config).to_string(),
            key: key.clone(),
            config: Some(backend_config.clone()),
        });
        let offloaded_ref = updated.offloaded.clone();

        // offloaded 参照も sidecar に残す。index.json が破損しても、ローカル object が
        // 無い版を外部参照から再構築できるようにする。
        super::index::append_object_ref(
            store_dir,
            project_id,
            &entry.hash,
            ObjectRef {
                rel: rel.clone(),
                version,
                size: entry.size,
                crc: entry.crc,
                stored_at: entry.stored_at,
                publisher: entry.publisher.clone(),
                source: entry.source.clone(),
                offloaded: offloaded_ref,
            },
        )
        .await?;
        index.insert(&rel, version, updated);
        index.save(store_dir).await?;

        // 同一 object を指すローカル版が一つでも残る間は共有実体を削除しない。
        let has_local_reference = index
            .iter_all()
            .any(|(_, _, other)| other.hash == entry.hash && other.offloaded.is_none());
        if !has_local_reference {
            let _ = tokio::fs::remove_file(&obj).await;
        }

        report.offloaded.push((rel, version));
        report.bytes_offloaded = report.bytes_offloaded.saturating_add(entry.size);
    }

    Ok(report)
}

pub fn backend_name(config: &RotationBackendConfig) -> &'static str {
    match config {
        RotationBackendConfig::S3 { .. } => "s3",
        RotationBackendConfig::LocalPath { .. } => "local_path",
        RotationBackendConfig::Gdrive { .. } => "gdrive",
        RotationBackendConfig::Unset => "unset",
    }
}

/// 退避済み一覧。
pub fn list_offloaded(index: &HistoryIndex, rel: Option<&str>) -> Vec<OffloadedVersion> {
    index
        .iter_all()
        .filter(|(r, _, entry)| entry.offloaded.is_some() && rel.is_none_or(|want| want == *r))
        .map(|(r, v, entry)| {
            let offloaded = entry.offloaded.as_ref().expect("filtered above");
            OffloadedVersion {
                rel: r.to_string(),
                version: v,
                size: entry.size,
                backend: offloaded.backend.clone(),
                key: offloaded.key.clone(),
            }
        })
        .collect()
}

/// 取り戻し: backend.get → blake3 検証 → objects へ戻す → 最後の外部参照なら
/// backend から削除 → `offloaded` マークを外す。
/// backend 不達時は明確なエラーを返す。取得後の外部削除だけが失敗した場合は、
/// 索引の offloaded 参照を残し、検証済みローカル実体から次回再試行できる。
pub async fn fetch(
    store_dir: &Path,
    project_id: &str,
    backend: &dyn RotationBackend,
    index: &mut HistoryIndex,
    rel: &str,
    version: u64,
) -> io::Result<()> {
    let Some(entry) = index.get(rel, version).cloned() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such version in history index: {rel} v{version}"),
        ));
    };
    let Some(offloaded) = entry.offloaded.clone() else {
        // 既にローカルにある。何もしなくてよい。
        return Ok(());
    };
    let expected_key = object_key(project_id, &entry.hash);
    if offloaded.key != expected_key || !is_valid_object_key(&offloaded.key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid offloaded object key for {rel} v{version}"),
        ));
    }
    let obj = object_path(store_dir, &entry.hash);
    if let Some(parent) = obj.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // 前回 fetch が backend.delete 後・index 更新前に失敗していても、検証済みの
    // ローカル実体から安全に再開できる。
    let local_is_valid = validate_fetched(&obj, &entry).await.unwrap_or(false);
    if !local_is_valid {
        match backend.size(&offloaded.key).await {
            Ok(Some(size)) if size == entry.size => {}
            Ok(Some(size)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "offloaded object size mismatch for {rel} v{version}: expected {}, backend has {size}",
                        entry.size
                    ),
                ));
            }
            Ok(None) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("offloaded object is missing for {rel} v{version}"),
                ));
            }
            Err(error) => {
                return Err(io::Error::other(format!(
                    "アーカイブ先に接続できない ({}): {error}",
                    offloaded.backend
                )));
            }
        }

        let tmp = obj.with_extension(format!("fetch-{}.tmp", uuid::Uuid::new_v4()));
        if let Err(error) = backend.get(&offloaded.key, &tmp).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io::Error::other(format!(
                "アーカイブ先に接続できない ({}): {error}",
                offloaded.backend
            )));
        }

        let valid = match validate_fetched(&tmp, &entry).await {
            Ok(valid) => valid,
            Err(error) => {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(error);
            }
        };
        if !valid {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("fetched object failed integrity validation for {rel} v{version}"),
            ));
        }

        if let Err(error) = crate::manifest::replace_file_atomically(&tmp, &obj).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
    }

    // 取り戻しは move semantics。削除に失敗した場合は offloaded 参照を残し、
    // 次回 fetch でローカル実体を再利用して delete から再試行する。
    let has_other_offloaded_reference = index.iter_all().any(|(other_rel, other_version, other)| {
        (other_rel != rel || other_version != version)
            && other.offloaded.as_ref().is_some_and(|other_offloaded| {
                other_offloaded.key == offloaded.key
                    && other_offloaded.config == offloaded.config
                    && other_offloaded.backend == offloaded.backend
            })
    });
    if !has_other_offloaded_reference {
        backend.delete(&offloaded.key).await.map_err(|error| {
            io::Error::other(format!(
                "restored object locally but failed to remove archive copy ({}): {error}",
                offloaded.backend
            ))
        })?;
    }

    super::index::append_object_ref(
        store_dir,
        project_id,
        &entry.hash,
        ObjectRef {
            rel: rel.to_string(),
            version,
            size: entry.size,
            crc: entry.crc,
            stored_at: entry.stored_at,
            publisher: entry.publisher.clone(),
            source: entry.source.clone(),
            offloaded: None,
        },
    )
    .await?;

    let mut restored = entry;
    restored.offloaded = None;
    index.insert(rel, version, restored);
    index.save(store_dir).await?;
    Ok(())
}

async fn validate_fetched(path: &Path, entry: &IndexEntry) -> io::Result<bool> {
    let (actual_crc, actual_size) = crate::manifest::crc32_of_file(path).await?;
    if actual_size != entry.size || actual_crc != entry.crc {
        return Ok(false);
    }
    let (hash, hashed_size, _) = synergos_net::transfer::hash_file(path)
        .await
        .map_err(|error| io::Error::other(format!("hash fetched object: {error}")))?;
    Ok(hashed_size == entry.size
        && blake3::Hash::from_bytes(hash.0).to_hex().to_string() == entry.hash
        && is_valid_object_hash(&entry.hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct WrongSizeBackend;

    #[async_trait::async_trait]
    impl RotationBackend for WrongSizeBackend {
        async fn put(&self, _key: &str, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        async fn get(&self, _key: &str, _dest: &Path) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        async fn exists(&self, _key: &str) -> io::Result<bool> {
            Ok(true)
        }

        async fn size(&self, _key: &str) -> io::Result<Option<u64>> {
            Ok(Some(1))
        }

        async fn delete(&self, _key: &str) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }
    }

    #[tokio::test]
    async fn rotate_does_not_delete_local_object_when_uploaded_size_is_wrong() {
        let store_dir = std::env::temp_dir().join(format!(
            "synergos-rotation-size-{}",
            uuid::Uuid::new_v4()
        ));
        let hash = "a".repeat(64);
        let obj = object_path(&store_dir, &hash);
        tokio::fs::create_dir_all(obj.parent().unwrap()).await.unwrap();
        tokio::fs::write(&obj, b"local data").await.unwrap();
        let mut index = HistoryIndex::new("p");
        index.insert(
            "a.bin",
            1,
            IndexEntry {
                hash,
                size: 10,
                crc: 0,
                stored_at: 1,
                publisher: String::new(),
                source: String::new(),
                offloaded: None,
            },
        );
        let config = RotationBackendConfig::LocalPath {
            path: store_dir.to_string_lossy().into_owned(),
        };

        let report = rotate(
            &store_dir,
            "p",
            &WrongSizeBackend,
            &config,
            &mut index,
            1,
            2 * 24 * 60 * 60 * 1000,
            &[],
            false,
        )
        .await
        .unwrap();

        assert!(report.offloaded.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(index.get("a.bin", 1).unwrap().offloaded.is_none());
        assert!(obj.exists());
        let _ = tokio::fs::remove_dir_all(&store_dir).await;
    }
}

