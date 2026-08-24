//! 履歴ノードの保管庫 — 版の実体を content-addressed で保持し、索引を更新する。
//!
//! - 実体は **チャンク化せず** ファイル全体を `objects/<hh>/<blake3>` に置く。
//!   同じ内容は 1 回しか置かない (ファイル単位の重複排除)。
//! - **ハードリンクは使わずコピーする**。publisher 側の作業ツリーは人が in-place で
//!   編集するので、リンクしていると保管した旧版が後から書き換わってしまう。
//! - 索引更新はプロジェクトごとの Mutex で直列化する (読み→変更→原子的保存)。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use synergos_net::config::HistoryConfig;

use super::gc::{apply_retention, GcReport};
use super::index::{
    append_object_ref, is_valid_object_hash, meta_path, object_path, remove_object_ref,
    HistoryIndex, IndexEntry, ObjectRef, OBJECTS_DIR,
};
use super::rotation::{self, OffloadedVersion, RotationReport};
use super::tags::{self, Tag, TagSummary};

/// 保管庫に置いた版 1 件 (lookup / list の戻り値)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVersion {
    pub rel: String,
    pub version: u64,
    pub hash: String,
    pub size: u64,
    pub crc: u32,
    pub stored_at: u64,
    pub publisher: String,
    pub source: String,
    /// object 実体の絶対パス。
    pub path: PathBuf,
}

/// archive の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveOutcome {
    /// 新しい object を書いた
    Stored,
    /// 同じ内容が既にあった (索引だけ更新)
    Deduplicated,
    /// 同じ (rel, version, hash) が既に索引にあり何もしなかった
    AlreadyIndexed,
    /// このプロジェクトは保持対象外
    NotCovered,
}

/// 履歴ノードの保管庫。設定と per-project ロックだけを持つ (状態はディスク)。
pub struct HistoryStore {
    config: HistoryConfig,
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl HistoryStore {
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            config,
            locks: DashMap::new(),
        }
    }

    pub fn config(&self) -> &HistoryConfig {
        &self.config
    }

    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// このプロジェクトの版を保持するか。
    pub fn covers(&self, project_id: &str) -> bool {
        self.config.covers(project_id)
    }

    /// プロジェクトの保管庫ディレクトリ。相対 root ならプロジェクトルート相対、
    /// 絶対 root なら `<root>/<blake3(project_id)>`。
    pub fn store_dir(&self, project_root: &Path, project_id: &str) -> io::Result<PathBuf> {
        self.config
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let root = Path::new(&self.config.root);
        if root.is_absolute() {
            Ok(root.join(project_dir_name(project_id)))
        } else {
            let canonical_root = std::fs::canonicalize(project_root)?;
            let mut candidate = canonical_root.clone();
            for component in root.components() {
                if let std::path::Component::Normal(segment) = component {
                    candidate.push(segment);
                    match std::fs::symlink_metadata(&candidate) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "relative history.root must not traverse a symlink",
                            ));
                        }
                        Ok(_) => {
                            if !std::fs::canonicalize(&candidate)?.starts_with(&canonical_root) {
                                return Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "relative history.root resolves outside the project root",
                                ));
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            Ok(candidate)
        }
    }

    fn lock(&self, project_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// 版の実体を保管庫に入れる。`src` は作業ツリー上の完成ファイル
    /// (受信なら rename 後の final path、publish なら publish 対象)。
    #[allow(clippy::too_many_arguments)]
    pub async fn archive(
        &self,
        project_root: &Path,
        project_id: &str,
        rel: &str,
        version: u64,
        expected_size: u64,
        crc: u32,
        publisher: &str,
        source: &str,
        src: &Path,
    ) -> io::Result<ArchiveOutcome> {
        if !self.covers(project_id) {
            return Ok(ArchiveOutcome::NotCovered);
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        let snapshot = create_snapshot(src, &store_dir, expected_size, crc).await?;
        let mut index = match HistoryIndex::load_or_rebuild(&store_dir, project_id).await {
            Ok(index) => index,
            Err(error) => {
                let _ = tokio::fs::remove_file(&snapshot.path).await;
                return Err(error);
            }
        };
        if let Some(existing) = index.get(rel, version) {
            if existing.hash != snapshot.hash {
                let _ = tokio::fs::remove_file(&snapshot.path).await;
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("history already contains different content for {rel} v{version}"),
                ));
            }
        }
        let obj = object_path(&store_dir, &snapshot.hash);
        let object_is_valid = match validate_object(&obj, &snapshot.hash, expected_size, crc).await {
            Ok(valid) => valid,
            Err(error) => {
                let _ = tokio::fs::remove_file(&snapshot.path).await;
                return Err(error);
            }
        };
        let outcome = if object_is_valid {
            let _ = tokio::fs::remove_file(&snapshot.path).await;
            if index.get(rel, version).is_some() {
                ArchiveOutcome::AlreadyIndexed
            } else {
                ArchiveOutcome::Deduplicated
            }
        } else {
            if let Some(parent) = obj.parent() {
                if let Err(error) = tokio::fs::create_dir_all(parent).await {
                    let _ = tokio::fs::remove_file(&snapshot.path).await;
                    return Err(error);
                }
            }
            if let Err(error) = crate::manifest::replace_file_atomically(&snapshot.path, &obj).await
            {
                let _ = tokio::fs::remove_file(&snapshot.path).await;
                return Err(error);
            }
            ArchiveOutcome::Stored
        };
        if outcome == ArchiveOutcome::AlreadyIndexed {
            let existing = index.get(rel, version).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "history index changed during archive",
                )
            })?;
            append_object_ref(
                &store_dir,
                project_id,
                &existing.hash,
                ObjectRef {
                    rel: rel.to_string(),
                    version,
                    size: existing.size,
                    crc: existing.crc,
                    stored_at: existing.stored_at,
                    publisher: existing.publisher.clone(),
                    source: existing.source.clone(),
                    offloaded: existing.offloaded.clone(),
                },
            )
            .await?;
            return Ok(outcome);
        }
        let now = synergos_net::types::now_ms();
        append_object_ref(
            &store_dir,
            project_id,
            &snapshot.hash,
            ObjectRef {
                rel: rel.to_string(),
                version,
                size: expected_size,
                crc,
                stored_at: now,
                publisher: publisher.to_string(),
                source: source.to_string(),
                offloaded: None,
            },
        )
        .await?;
        index.insert(
            rel,
            version,
            IndexEntry {
                hash: snapshot.hash,
                size: expected_size,
                crc,
                stored_at: now,
                publisher: publisher.to_string(),
                source: source.to_string(),
                offloaded: None,
            },
        );
        index.save(&store_dir).await?;
        Ok(outcome)
    }

    /// (rel, version) の実体を探す。索引にあっても object が無ければ None。
    /// 索引が `offloaded` を指しており `rotation.enabled` なら、ここで
    /// backend から自動的に取り戻す (checkout / restore / 旧版 FileWant 応答の
    /// 共通経路。spec: archive-rotation §取り戻し)。backend 不達時は warn して
    /// None を返す (lookup は「無かった」を表す型なので panic/エラー伝播しない)。
    pub async fn lookup(
        &self,
        project_root: &Path,
        project_id: &str,
        rel: &str,
        version: u64,
    ) -> io::Result<Option<StoredVersion>> {
        if !self.covers(project_id) {
            return Ok(None);
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        let Some(entry) = index.get(rel, version) else {
            return Ok(None);
        };
        if entry.offloaded.is_some() {
            if !self.config.rotation.enabled {
                tracing::warn!(
                    "history {rel} v{version} is offloaded but rotation is disabled; cannot serve"
                );
                return Ok(None);
            }
            if let Err(error) = self.fetch_offloaded(project_root, project_id, rel, version).await {
                tracing::warn!("history {rel} v{version} fetch from rotation backend failed: {error}");
                return Ok(None);
            }
            let refreshed = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
            let Some(entry) = refreshed.get(rel, version) else {
                return Ok(None);
            };
            let path = object_path(&store_dir, &entry.hash);
            return Ok(Some(to_stored(rel, version, entry, path)));
        }
        let path = object_path(&store_dir, &entry.hash);
        if !validate_object(&path, &entry.hash, entry.size, entry.crc)
            .await
            .unwrap_or(false)
        {
            tracing::warn!("history object failed integrity validation: {rel} v{version}");
            return Ok(None);
        }
        Ok(Some(to_stored(rel, version, entry, path)))
    }

    /// 保持している版の一覧 (rel を渡せばそのファイルだけ)。
    pub async fn list(
        &self,
        project_root: &Path,
        project_id: &str,
        rel: Option<&str>,
    ) -> io::Result<Vec<StoredVersion>> {
        if !self.covers(project_id) {
            return Ok(Vec::new());
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        Ok(index
            .iter_all()
            .filter(|(r, _, _)| rel.is_none_or(|want| want == *r))
            .map(|(r, v, e)| to_stored(r, v, e, object_path(&store_dir, &e.hash)))
            .collect())
    }

    /// タグを作成/上書きする (`pins` = 保護したい (rel, version) 集合)。
    pub async fn tag_add(
        &self,
        project_root: &Path,
        project_id: &str,
        name: &str,
        pins: std::collections::BTreeMap<String, u64>,
    ) -> io::Result<Tag> {
        if !self.covers(project_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this node is not a history node for the project",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        let now = synergos_net::types::now_ms();
        tags::save(&store_dir, project_id, name, now, pins).await
    }

    /// タグ一覧 (name / created_at / pin 数)。
    pub async fn tag_list(
        &self,
        project_root: &Path,
        project_id: &str,
    ) -> io::Result<Vec<TagSummary>> {
        if !self.covers(project_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this node is not a history node for the project",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        tags::list(&store_dir, project_id).await
    }

    /// 1 タグの内容を取得する。無ければ `None`。
    pub async fn tag_show(
        &self,
        project_root: &Path,
        project_id: &str,
        name: &str,
    ) -> io::Result<Option<Tag>> {
        if !self.covers(project_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this node is not a history node for the project",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        tags::load(&store_dir, project_id, name).await
    }

    /// タグを削除する (実体は消さない。以後 GC 対象に戻るだけ)。`false` = 元々無かった。
    pub async fn tag_remove(
        &self,
        project_root: &Path,
        project_id: &str,
        name: &str,
    ) -> io::Result<bool> {
        if !self.covers(project_id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this node is not a history node for the project",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        tags::remove(&store_dir, project_id, name).await
    }

    /// GC / ローテーションが参照する「保護済み (path, version) 集合」を返す。
    /// `extra_keep` (手元 manifest の最新版、`--keep-manifest` 等) と全タグの
    /// pins を合流させる。保管庫にタグ pins ディレクトリが無ければ `extra_keep`
    /// のみ返す。
    pub async fn protected_versions(
        &self,
        project_root: &Path,
        project_id: &str,
        extra_keep: &[(String, u64)],
    ) -> io::Result<Vec<(String, u64)>> {
        let store_dir = self.store_dir(project_root, project_id)?;
        let mut protected = extra_keep.to_vec();
        let tag_pins = tags::all_pins(&store_dir, project_id).await?;
        if !tag_pins.is_empty() {
            let index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
            for (rel, version) in &tag_pins {
                if index.get(rel, *version).is_none() {
                    tracing::warn!(
                        "tag pin {rel} v{version} is not present in the store for project {project_id}"
                    );
                }
            }
        }
        protected.extend(tag_pins);
        Ok(protected)
    }

    /// 保持ポリシー (設定) を適用する。`keep` は削ってはいけない (rel, version)
    /// (手元 manifest の最新版など)。タグが指す版はここで自動的に追加保護される。
    /// `purge = true` なら保管庫を全消去する。退避済み版が残る場合は、外部 object の
    /// orphan 化を避けるため先に fetch するよう明示エラーを返す。
    pub async fn gc(
        &self,
        project_root: &Path,
        project_id: &str,
        keep: &[(String, u64)],
        purge: bool,
    ) -> io::Result<GcReport> {
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        let mut index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        let mut report = GcReport::default();
        if purge {
            let offloaded_count = index
                .iter_all()
                .filter(|(_, _, entry)| entry.offloaded.is_some())
                .count();
            if offloaded_count > 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "history purge refused: {offloaded_count} offloaded version(s) must be fetched first"
                    ),
                ));
            }
            for (rel, version, _) in index.iter_all() {
                report.removed_versions.push((rel.to_string(), version));
            }
            let hashes = index.referenced_hashes();
            report.removed_objects = hashes.len();
            for hash in hashes {
                if let Ok(metadata) = tokio::fs::metadata(object_path(&store_dir, &hash)).await {
                    report.bytes_freed = report.bytes_freed.saturating_add(metadata.len());
                }
            }
            // 索引を先に空へ切り替える。削除途中で失敗しても、旧索引が消えた
            // object を参照する状態には戻らず、残骸は次回 GC で回収できる。
            HistoryIndex::new(project_id).save(&store_dir).await?;
            match tokio::fs::remove_dir_all(store_dir.join(OBJECTS_DIR)).await {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            // objects/ ごと消えるので staging コピーも一緒に片付くが、
            // 保管庫直下に残った index tmp はここで回収する。
            report.bytes_freed = report
                .bytes_freed
                .saturating_add(sweep_stale_temporaries(&store_dir).await?);
            return Ok(report);
        }
        let now = synergos_net::types::now_ms();
        let protected = self.protected_versions(project_root, project_id, keep).await?;
        let victims = apply_retention(&index, &self.config, &protected, now);
        let mut removed = Vec::new();
        for (rel, version) in &victims {
            if let Some(entry) = index.remove(rel, *version) {
                removed.push((rel.clone(), *version, entry.hash));
            }
        }
        // 先に新索引を確定し、以後の object 削除失敗は安全な orphan にする。
        // 逆順だと index 保存失敗時に旧索引が消えた object を指してしまう。
        index.save(&store_dir).await?;
        for (rel, version, hash) in removed {
            remove_object_ref(&store_dir, &hash, &rel, version).await?;
            report.removed_versions.push((rel, version));
        }
        // 参照が無くなった object を消す
        let referenced = index.referenced_hashes();
        for hex in collect_object_hashes(&store_dir).await? {
            if referenced.contains(&hex) {
                continue;
            }
            let obj = object_path(&store_dir, &hex);
            if let Ok(meta) = tokio::fs::metadata(&obj).await {
                report.bytes_freed += meta.len();
            }
            let _ = tokio::fs::remove_file(&obj).await;
            let _ = tokio::fs::remove_file(meta_path(&store_dir, &hex)).await;
            report.removed_objects += 1;
        }
        // archive 中にプロセスが落ちると staging コピー (元ファイルと同サイズ) が
        // 残り、通常の object 走査からは見えないので放置すると際限なく溜まる。
        // gc は archive と同じ per-project ロックの下で走るため、ここで残骸を
        // 消しても進行中の archive を壊さない。
        report.bytes_freed = report
            .bytes_freed
            .saturating_add(sweep_stale_temporaries(&store_dir).await?);
        Ok(report)
    }

    /// 保持ポリシーで残った旧版のうち、設定の `rotation.offload_after_days` より
    /// 古いものを外部ストレージへ退避する (spec: archive-rotation)。
    /// `keep` は gc と同じ「削ってはいけない (rel, version)」で、タグと合流して
    /// 保護集合になる ([`Self::protected_versions`] を再利用し、gc.rs の保護
    /// ロジックを二重実装しない)。`dry_run = true` なら候補一覧のみ返す。
    pub async fn rotate(
        &self,
        project_root: &Path,
        project_id: &str,
        keep: &[(String, u64)],
        dry_run: bool,
    ) -> io::Result<RotationReport> {
        let rotation_cfg = &self.config.rotation;
        if !rotation_cfg.enabled {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "history.rotation.enabled is false",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        let mut index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        let now = synergos_net::types::now_ms();
        let protected = self.protected_versions(project_root, project_id, keep).await?;

        if dry_run {
            let candidates =
                rotation::select_candidates(&index, rotation_cfg.offload_after_days, now, &protected);
            return Ok(RotationReport {
                candidates,
                ..RotationReport::default()
            });
        }

        let backend = rotation::build_backend(&rotation_cfg.backend)?;
        rotation::rotate(
            &store_dir,
            project_id,
            backend.as_ref(),
            &rotation_cfg.backend,
            &mut index,
            rotation_cfg.offload_after_days,
            now,
            &protected,
            false,
        )
        .await
    }

    /// 退避済み版の一覧 (`synergos history offloaded`)。
    pub async fn offloaded(
        &self,
        project_root: &Path,
        project_id: &str,
        rel: Option<&str>,
    ) -> io::Result<Vec<OffloadedVersion>> {
        let store_dir = self.store_dir(project_root, project_id)?;
        let index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        Ok(rotation::list_offloaded(&index, rel))
    }

    /// 退避済みの版を明示的に取り戻す (`synergos history fetch`)。
    /// backend 不達時は明確なエラーを返し、index / objects を変更しない。
    pub async fn fetch_offloaded(
        &self,
        project_root: &Path,
        project_id: &str,
        rel: &str,
        version: u64,
    ) -> io::Result<()> {
        let rotation_cfg = &self.config.rotation;
        if !rotation_cfg.enabled {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "history.rotation.enabled is false",
            ));
        }
        let store_dir = self.store_dir(project_root, project_id)?;
        let lock = self.lock(project_id);
        let _guard = lock.lock().await;
        let index = HistoryIndex::load_or_rebuild(&store_dir, project_id).await?;
        let mut backend_config = index
            .get(rel, version)
            .and_then(|entry| entry.offloaded.as_ref())
            .and_then(|offloaded| offloaded.config.as_ref())
            .cloned()
            .unwrap_or_else(|| rotation_cfg.backend.clone());
        // 保存先 folder は退避時の値を使いつつ、資格情報ファイルは現在の設定へ
        // ローテーションできるようにする。
        if let (
            synergos_net::config::RotationBackendConfig::Gdrive {
                credentials_file, ..
            },
            synergos_net::config::RotationBackendConfig::Gdrive {
                credentials_file: current_credentials,
                ..
            },
        ) = (&mut backend_config, &rotation_cfg.backend)
        {
            *credentials_file = current_credentials.clone();
        }
        let backend = rotation::build_backend(&backend_config)?;
        let mut index = index;
        rotation::fetch(&store_dir, project_id, backend.as_ref(), &mut index, rel, version).await
    }
}

fn to_stored(rel: &str, version: u64, entry: &IndexEntry, path: PathBuf) -> StoredVersion {
    StoredVersion {
        rel: rel.to_string(),
        version,
        hash: entry.hash.clone(),
        size: entry.size,
        crc: entry.crc,
        stored_at: entry.stored_at,
        publisher: entry.publisher.clone(),
        source: entry.source.clone(),
        path,
    }
}

/// 絶対 root 配下で安全な、衝突耐性のある project ディレクトリ名を作る。
fn project_dir_name(project_id: &str) -> String {
    blake3::hash(project_id.as_bytes()).to_hex().to_string()
}

struct Snapshot {
    path: PathBuf,
    hash: String,
}

/// 先に保管庫内へコピーし、その不変な一時ファイル自身を検証・hash する。
/// 元ファイルが publish 直後やコピー中に編集された場合は、誤った版として
/// 保管せず明示的に失敗する。
async fn create_snapshot(
    src: &Path,
    store_dir: &Path,
    expected_size: u64,
    expected_crc: u32,
) -> io::Result<Snapshot> {
    let staging = store_dir.join(OBJECTS_DIR);
    tokio::fs::create_dir_all(&staging).await?;
    let tmp = staging.join(format!(".archive-{}.tmp", uuid::Uuid::new_v4()));
    if let Err(error) = tokio::fs::copy(src, &tmp).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(error);
    }
    let (actual_crc, actual_size) = match crate::manifest::crc32_of_file(&tmp).await {
        Ok(value) => value,
        Err(error) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
    };
    if actual_size != expected_size || actual_crc != expected_crc {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source changed while the history snapshot was being captured",
        ));
    }
    let (hash, hashed_size, _) = match synergos_net::transfer::hash_file(&tmp).await {
        Ok(value) => value,
        Err(error) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io::Error::other(format!(
                "hash history snapshot: {error}"
            )));
        }
    };
    if hashed_size != expected_size {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "history snapshot changed during hashing",
        ));
    }
    Ok(Snapshot {
        path: tmp,
        hash: blake3::Hash::from_bytes(hash.0).to_hex().to_string(),
    })
}

async fn validate_object(
    path: &Path,
    expected_hash: &str,
    expected_size: u64,
    expected_crc: u32,
) -> io::Result<bool> {
    let (actual_crc, actual_size) = match crate::manifest::crc32_of_file(path).await {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if actual_size != expected_size || actual_crc != expected_crc {
        return Ok(false);
    }
    let (hash, hashed_size, _) = synergos_net::transfer::hash_file(path)
        .await
        .map_err(|error| io::Error::other(format!("hash history object: {error}")))?;
    Ok(hashed_size == expected_size
        && blake3::Hash::from_bytes(hash.0).to_hex().to_string() == expected_hash)
}

/// クラッシュで取り残された一時ファイルを回収し、解放したバイト数を返す。
///
/// 対象は 3 種:
/// - `<store>/index-*.tmp` ([`HistoryIndex::save`])
/// - `<store>/objects/.archive-*.tmp` ([`create_snapshot`] の staging コピー。
///   元ファイルと同サイズなので、放置すると保管庫が際限なく膨らむ)
/// - `<store>/objects/<hh>/<hash>.meta.tmp-*` (sidecar 書き換えの一時ファイル)
///
/// staging コピーは shard ディレクトリではなく `objects/` 直下にあるため
/// [`collect_object_hashes`] の走査には掛からず、gc でしか回収できない。
/// 呼び出し元 (`HistoryStore::gc`) は archive と同じ per-project ロックを
/// 保持しているので、生存中の一時ファイルを消してしまうことはない。
async fn sweep_stale_temporaries(store_dir: &Path) -> io::Result<u64> {
    let mut freed = remove_matching(store_dir, |name| {
        name.starts_with("index-") && name.ends_with(".tmp")
    })
    .await?;
    let objects = store_dir.join(OBJECTS_DIR);
    freed = freed.saturating_add(
        remove_matching(&objects, |name| {
            name.starts_with(".archive-") && name.ends_with(".tmp")
        })
        .await?,
    );
    let mut shards = match tokio::fs::read_dir(&objects).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(freed),
        Err(error) => return Err(error),
    };
    while let Some(shard) = shards.next_entry().await? {
        if !shard.file_type().await?.is_dir() {
            continue;
        }
        freed = freed.saturating_add(
            remove_matching(&shard.path(), |name| name.contains(".meta.tmp-")).await?,
        );
    }
    Ok(freed)
}

/// `dir` 直下で `matches` に該当する通常ファイルを消し、解放したバイト数を返す。
async fn remove_matching(dir: &Path, matches: impl Fn(&str) -> bool) -> io::Result<u64> {
    let mut freed = 0u64;
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if !matches(&name) || !entry.file_type().await?.is_file() {
            continue;
        }
        let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
        if tokio::fs::remove_file(entry.path()).await.is_ok() {
            freed = freed.saturating_add(size);
        }
    }
    Ok(freed)
}

/// `objects/` 配下の object hash (sidecar / tmp を除く) を列挙する。
async fn collect_object_hashes(store_dir: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    let objects = store_dir.join(OBJECTS_DIR);
    let mut shards = match tokio::fs::read_dir(&objects).await {
        Ok(rd) => rd,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(error) => return Err(error),
    };
    while let Some(shard) = shards.next_entry().await? {
        if !shard.file_type().await?.is_dir() {
            continue;
        }
        let mut files = tokio::fs::read_dir(shard.path()).await?;
        while let Some(file) = files.next_entry().await? {
            let name = file.file_name().to_string_lossy().to_string();
            if name.contains('.') || !is_valid_object_hash(&name) {
                continue; // sidecar (.meta.json) / tmp / unknown file
            }
            out.push(name);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &str) -> HistoryStore {
        HistoryStore::new(HistoryConfig {
            enabled: true,
            projects: vec!["*".into()],
            root: root.into(),
            ..HistoryConfig::default()
        })
    }

    async fn temp_project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("synergos-hist-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn archive_lookup_roundtrip_and_dedup() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let f = root.join("a.bin");
        let v1_body = b"version-one";
        let v1_crc = crc32fast::hash(v1_body);
        tokio::fs::write(&f, v1_body).await.unwrap();
        assert_eq!(
            s.archive(
                &root,
                "p",
                "a.bin",
                1,
                v1_body.len() as u64,
                v1_crc,
                "peer-a",
                "published",
                &f,
            )
                .await
                .unwrap(),
            ArchiveOutcome::Stored
        );
        // 同じ内容を別 version として archive → dedup
        assert_eq!(
            s.archive(
                &root,
                "p",
                "a.bin",
                2,
                v1_body.len() as u64,
                v1_crc,
                "peer-a",
                "published",
                &f,
            )
                .await
                .unwrap(),
            ArchiveOutcome::Deduplicated
        );
        // 同じ (rel, version, hash) → 何もしない
        assert_eq!(
            s.archive(
                &root,
                "p",
                "a.bin",
                2,
                v1_body.len() as u64,
                v1_crc,
                "peer-a",
                "published",
                &f,
            )
                .await
                .unwrap(),
            ArchiveOutcome::AlreadyIndexed
        );
        // 作業ツリーを in-place で書き換えても保管した旧版は変わらない (コピー)
        let v3_body = b"version-three!";
        tokio::fs::write(&f, v3_body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            3,
            v3_body.len() as u64,
            crc32fast::hash(v3_body),
            "peer-a",
            "published",
            &f,
        )
            .await
            .unwrap();
        let v1 = s.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&v1.path).await.unwrap(), b"version-one");
        let v3 = s.lookup(&root, "p", "a.bin", 3).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&v3.path).await.unwrap(), b"version-three!");
        assert!(s.lookup(&root, "p", "a.bin", 9).await.unwrap().is_none());
        let all = s.list(&root, "p", None).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(s.list(&root, "p", Some("zzz")).await.unwrap().len(), 0);
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn not_covered_project_is_ignored() {
        let root = temp_project().await;
        let s = HistoryStore::new(HistoryConfig {
            enabled: true,
            projects: vec!["other".into()],
            ..HistoryConfig::default()
        });
        let f = root.join("a.bin");
        tokio::fs::write(&f, b"x").await.unwrap();
        assert_eq!(
            s.archive(&root, "p", "a.bin", 1, 1, 1, "peer", "published", &f)
                .await
                .unwrap(),
            ArchiveOutcome::NotCovered
        );
        assert!(s.lookup(&root, "p", "a.bin", 1).await.unwrap().is_none());
        assert!(!root.join(".synergos").exists());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn archive_rejects_a_source_that_changed_after_publish_validation() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let f = root.join("a.bin");
        let published = b"published";
        tokio::fs::write(&f, b"changed!!").await.unwrap();
        let error = s
            .archive(
                &root,
                "p",
                "a.bin",
                1,
                published.len() as u64,
                crc32fast::hash(published),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(s.list(&root, "p", None).await.unwrap().is_empty());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn archive_does_not_replace_an_existing_version_with_different_content() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let f = root.join("a.bin");
        tokio::fs::write(&f, b"one").await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            3,
            crc32fast::hash(b"one"),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        tokio::fs::write(&f, b"two").await.unwrap();
        let error = s
            .archive(
                &root,
                "p",
                "a.bin",
                1,
                3,
                crc32fast::hash(b"two"),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        let stored = s.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(stored.path).await.unwrap(), b"one");
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn absolute_root_uses_project_subdir() {
        let root = temp_project().await;
        let store_root = temp_project().await;
        let s = store(store_root.to_str().unwrap());
        assert_eq!(
            s.store_dir(&root, "my/proj").unwrap(),
            store_root.join(project_dir_name("my/proj"))
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&store_root).await;
    }

    #[tokio::test]
    async fn gc_purge_and_retention_remove_unreferenced_objects() {
        let root = temp_project().await;
        let mut cfg = HistoryConfig {
            enabled: true,
            ..HistoryConfig::default()
        };
        cfg.max_versions_per_file = 1;
        let s = HistoryStore::new(cfg);
        let f = root.join("a.bin");
        for (v, body) in [(1u64, "one"), (2, "two"), (3, "three")] {
            tokio::fs::write(&f, body).await.unwrap();
            s.archive(
                &root,
                "p",
                "a.bin",
                v,
                body.len() as u64,
                crc32fast::hash(body.as_bytes()),
                "peer",
                "published",
                &f,
            )
                .await
                .unwrap();
        }
        // keep で v1 を保護。ポリシー max_versions=1 → v3 (最新) と v1 (保護) が残り v2 が消える
        let report = s
            .gc(&root, "p", &[("a.bin".to_string(), 1)], false)
            .await
            .unwrap();
        assert_eq!(report.removed_versions, vec![("a.bin".to_string(), 2)]);
        assert_eq!(report.removed_objects, 1);
        assert!(s.lookup(&root, "p", "a.bin", 2).await.unwrap().is_none());
        assert!(s.lookup(&root, "p", "a.bin", 1).await.unwrap().is_some());
        assert!(s.lookup(&root, "p", "a.bin", 3).await.unwrap().is_some());

        let purge = s.gc(&root, "p", &[], true).await.unwrap();
        assert_eq!(purge.removed_versions.len(), 2);
        assert!(s.list(&root, "p", None).await.unwrap().is_empty());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// max_versions_per_file=1 でも、タグが指す旧版は GC で消えない
    /// (spec/tasks/…-2-version-tags.md の完了条件: GC 保護の統合テスト)。
    #[tokio::test]
    async fn gc_protects_tagged_versions_even_under_max_versions_policy() {
        let root = temp_project().await;
        let mut cfg = HistoryConfig {
            enabled: true,
            ..HistoryConfig::default()
        };
        cfg.max_versions_per_file = 1;
        let s = HistoryStore::new(cfg);
        let f = root.join("a.bin");
        for (v, body) in [(1u64, "one"), (2, "two"), (3, "three")] {
            tokio::fs::write(&f, body).await.unwrap();
            s.archive(
                &root,
                "p",
                "a.bin",
                v,
                body.len() as u64,
                crc32fast::hash(body.as_bytes()),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap();
        }
        let mut pins = std::collections::BTreeMap::new();
        pins.insert("a.bin".to_string(), 1);
        s.tag_add(&root, "p", "release-1.0", pins).await.unwrap();

        // keep も --keep-manifest も渡さない。タグだけで v1 が保護されるはず。
        let report = s.gc(&root, "p", &[], false).await.unwrap();
        assert_eq!(report.removed_versions, vec![("a.bin".to_string(), 2)]);
        assert!(s.lookup(&root, "p", "a.bin", 1).await.unwrap().is_some());
        assert!(s.lookup(&root, "p", "a.bin", 2).await.unwrap().is_none());
        assert!(s.lookup(&root, "p", "a.bin", 3).await.unwrap().is_some());

        // タグを外せば通常の保持ポリシーに戻り、v1 も次回 GC の対象になる。
        assert!(s.tag_remove(&root, "p", "release-1.0").await.unwrap());
        let report2 = s.gc(&root, "p", &[], false).await.unwrap();
        assert_eq!(report2.removed_versions, vec![("a.bin".to_string(), 1)]);
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// `--purge` はタグの有無に関わらず保管庫を全消去する。
    #[tokio::test]
    async fn gc_purge_ignores_tags() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let f = root.join("a.bin");
        tokio::fs::write(&f, b"one").await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            3,
            crc32fast::hash(b"one"),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        let mut pins = std::collections::BTreeMap::new();
        pins.insert("a.bin".to_string(), 1);
        s.tag_add(&root, "p", "keep-me", pins).await.unwrap();

        let purge = s.gc(&root, "p", &[], true).await.unwrap();
        assert_eq!(purge.removed_versions.len(), 1);
        assert!(s.list(&root, "p", None).await.unwrap().is_empty());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// `protected_versions` は extra_keep とタグ pins を合流させる。タグが指す版が
    /// 保管庫に無くても (未取得の版など) エラーにはならず、そのまま保護集合に入る。
    #[tokio::test]
    async fn protected_versions_merges_extra_keep_and_tags_and_tolerates_missing_pins() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let mut pins = std::collections::BTreeMap::new();
        pins.insert("missing.bin".to_string(), 9);
        s.tag_add(&root, "p", "future", pins).await.unwrap();

        let mut merged = s
            .protected_versions(&root, "p", &[("manifest.bin".to_string(), 4)])
            .await
            .unwrap();
        merged.sort();
        assert_eq!(
            merged,
            vec![
                ("manifest.bin".to_string(), 4),
                ("missing.bin".to_string(), 9),
            ]
        );
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn non_history_nodes_cannot_read_or_mutate_tags() {
        let root = temp_project().await;
        let s = HistoryStore::new(HistoryConfig::default());

        assert!(s
            .tag_add(&root, "p", "release", std::collections::BTreeMap::new())
            .await
            .is_err());
        assert!(s.tag_list(&root, "p").await.is_err());
        assert!(s.tag_show(&root, "p", "release").await.is_err());
        assert!(s.tag_remove(&root, "p", "release").await.is_err());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// archive 中にプロセスが落ちて残った staging コピー / tmp を gc が回収し、
    /// 保管中の object は消さないこと。
    #[tokio::test]
    async fn gc_reclaims_temporaries_left_behind_by_a_crashed_archive() {
        let root = temp_project().await;
        let s = store(".synergos/history");
        let f = root.join("a.bin");
        let body = b"kept";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();

        let store_dir = s.store_dir(&root, "p").unwrap();
        let stored = s.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        let staging = store_dir
            .join(OBJECTS_DIR)
            .join(".archive-11111111-2222-3333-4444-555555555555.tmp");
        tokio::fs::write(&staging, vec![0u8; 4096]).await.unwrap();
        let index_tmp = store_dir.join("index-abc.tmp");
        tokio::fs::write(&index_tmp, b"partial").await.unwrap();
        let meta_tmp = stored
            .path
            .with_file_name(format!("{}.meta.tmp-abc", stored.hash));
        tokio::fs::write(&meta_tmp, b"partial").await.unwrap();

        let report = s.gc(&root, "p", &[("a.bin".to_string(), 1)], false).await.unwrap();
        assert!(report.removed_versions.is_empty());
        assert!(!staging.exists(), "staging copy must be reclaimed");
        assert!(!index_tmp.exists(), "index tmp must be reclaimed");
        assert!(!meta_tmp.exists(), "sidecar tmp must be reclaimed");
        assert!(report.bytes_freed >= 4096);
        // 保管中の版は無傷
        assert!(s.lookup(&root, "p", "a.bin", 1).await.unwrap().is_some());
        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    // ── rotation (spec: archive-rotation) ──

    fn rotation_store(
        history_root: &str,
        offload_after_days: u64,
        backend_root: &std::path::Path,
    ) -> HistoryStore {
        HistoryStore::new(HistoryConfig {
            enabled: true,
            projects: vec!["*".into()],
            root: history_root.into(),
            rotation: synergos_net::config::HistoryRotationConfig {
                enabled: true,
                offload_after_days,
                interval_hours: 0,
                backend: synergos_net::config::RotationBackendConfig::LocalPath {
                    path: backend_root.to_string_lossy().to_string(),
                },
            },
            ..HistoryConfig::default()
        })
    }

    /// LocalDir backend で退避→index 更新→取り戻し→blake3 一致 の統合テスト。
    #[tokio::test]
    async fn rotate_offloads_old_version_and_fetch_restores_it() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 30, &backend_root);
        let f = root.join("a.bin");
        let body = b"old-version-body";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();

        // stored_at を閾値より古く書き換える (offload_after_days=30 → 60 日前にする)。
        age_stored_at(&s, &root, "p", "a.bin", 1, 60).await;

        let report = s.rotate(&root, "p", &[], false).await.unwrap();
        assert_eq!(report.offloaded, vec![("a.bin".to_string(), 1)]);
        assert!(report.skipped.is_empty());

        // ローカル objects 実体は削除されている。
        let store_dir = s.store_dir(&root, "p").unwrap();
        let index = HistoryIndex::load_or_rebuild(&store_dir, "p").await.unwrap();
        let entry = index.get("a.bin", 1).unwrap();
        assert!(entry.offloaded.is_some());
        assert!(!object_path(&store_dir, &entry.hash).exists());

        let offloaded = s.offloaded(&root, "p", None).await.unwrap();
        assert_eq!(offloaded.len(), 1);
        assert_eq!(offloaded[0].backend, "local_path");
        let purge_error = s.gc(&root, "p", &[], true).await.unwrap_err();
        assert!(purge_error.to_string().contains("must be fetched first"));

        // 取り戻し: fetch_offloaded → objects へ書き戻り、offloaded マークが外れる。
        s.fetch_offloaded(&root, "p", "a.bin", 1).await.unwrap();
        let stored = s.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&stored.path).await.unwrap(), body);
        let index_after = HistoryIndex::load_or_rebuild(&store_dir, "p").await.unwrap();
        assert!(index_after.get("a.bin", 1).unwrap().offloaded.is_none());
        assert!(!backend_root.join(&offloaded[0].key).exists());

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// `lookup` は offloaded な版に自動的に当たったら取り戻して返す
    /// (checkout / restore / FileWant 応答の共通経路)。
    #[tokio::test]
    async fn lookup_transparently_fetches_offloaded_version() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("a.bin");
        let body = b"transparent-fetch";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        age_stored_at(&s, &root, "p", "a.bin", 1, 10).await;
        s.rotate(&root, "p", &[], false).await.unwrap();

        let stored = s.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&stored.path).await.unwrap(), body);

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// 保護除外: 最新版 (keep) とタグが指す版は候補から外れ、ローテーションされない。
    #[tokio::test]
    async fn rotate_protects_keep_and_tagged_versions() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("a.bin");
        for (v, body) in [(1u64, "one"), (2, "two")] {
            tokio::fs::write(&f, body).await.unwrap();
            s.archive(
                &root,
                "p",
                "a.bin",
                v,
                body.len() as u64,
                crc32fast::hash(body.as_bytes()),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap();
            age_stored_at(&s, &root, "p", "a.bin", v, 10).await;
        }
        let mut pins = std::collections::BTreeMap::new();
        pins.insert("a.bin".to_string(), 1);
        s.tag_add(&root, "p", "release", pins).await.unwrap();

        // v1 はタグ保護、v2 は keep (手元 manifest 相当) で保護 → 候補ゼロ。
        let report = s
            .rotate(&root, "p", &[("a.bin".to_string(), 2)], false)
            .await
            .unwrap();
        assert!(report.offloaded.is_empty());

        let store_dir = s.store_dir(&root, "p").unwrap();
        let index = HistoryIndex::load_or_rebuild(&store_dir, "p").await.unwrap();
        assert!(index.get("a.bin", 1).unwrap().offloaded.is_none());
        assert!(index.get("a.bin", 2).unwrap().offloaded.is_none());

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// backend 不達時 (マウントされていない local_path) は put が失敗し、
    /// index / objects を一切変更せずスキップ一覧に積む。
    #[tokio::test]
    async fn rotate_leaves_index_and_objects_untouched_when_backend_unreachable() {
        let root = temp_project().await;
        let missing_backend_root =
            std::env::temp_dir().join(format!("synergos-rot-missing-{}", uuid::Uuid::new_v4()));
        let s = rotation_store(".synergos/history", 1, &missing_backend_root);
        let f = root.join("a.bin");
        let body = b"unreachable-backend";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        age_stored_at(&s, &root, "p", "a.bin", 1, 10).await;

        let report = s.rotate(&root, "p", &[], false).await.unwrap();
        assert!(report.offloaded.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].rel, "a.bin");

        let store_dir = s.store_dir(&root, "p").unwrap();
        let index = HistoryIndex::load_or_rebuild(&store_dir, "p").await.unwrap();
        let entry = index.get("a.bin", 1).unwrap();
        assert!(entry.offloaded.is_none());
        assert!(object_path(&store_dir, &entry.hash).exists());
        // ローカルの実体はまだ取れる。
        assert!(s.lookup(&root, "p", "a.bin", 1).await.unwrap().is_some());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// dry-run は候補一覧だけ返し、何も変更しない。
    #[tokio::test]
    async fn rotate_dry_run_lists_candidates_without_changes() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("a.bin");
        let body = b"dry-run-body";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        age_stored_at(&s, &root, "p", "a.bin", 1, 10).await;

        let report = s.rotate(&root, "p", &[], true).await.unwrap();
        assert_eq!(report.candidates, vec![("a.bin".to_string(), 1)]);
        assert!(report.offloaded.is_empty());

        let store_dir = s.store_dir(&root, "p").unwrap();
        let index = HistoryIndex::load_or_rebuild(&store_dir, "p").await.unwrap();
        assert!(index.get("a.bin", 1).unwrap().offloaded.is_none());

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// 同じ hash を共有する保護版が残る場合、候補版だけを退避してもローカル実体を
    /// 削除してはならない。
    #[tokio::test]
    async fn rotate_keeps_shared_object_for_protected_local_version() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("same.bin");
        let body = b"same-content";
        tokio::fs::write(&f, body).await.unwrap();
        for version in [1, 2] {
            s.archive(
                &root,
                "p",
                "same.bin",
                version,
                body.len() as u64,
                crc32fast::hash(body),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap();
            age_stored_at(&s, &root, "p", "same.bin", version, 10).await;
        }

        let report = s
            .rotate(&root, "p", &[("same.bin".to_string(), 2)], false)
            .await
            .unwrap();
        assert_eq!(report.offloaded, vec![("same.bin".to_string(), 1)]);
        let protected = s.lookup(&root, "p", "same.bin", 2).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(protected.path).await.unwrap(), body);

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// dedup された外部 object は最後の退避参照を取り戻すまで削除しない。
    #[tokio::test]
    async fn fetch_deletes_shared_archive_only_after_last_reference() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("same.bin");
        let body = b"shared-archive";
        tokio::fs::write(&f, body).await.unwrap();
        for version in [1, 2] {
            s.archive(
                &root,
                "p",
                "same.bin",
                version,
                body.len() as u64,
                crc32fast::hash(body),
                "peer",
                "published",
                &f,
            )
            .await
            .unwrap();
            age_stored_at(&s, &root, "p", "same.bin", version, 10).await;
        }
        s.rotate(&root, "p", &[], false).await.unwrap();
        let offloaded = s.offloaded(&root, "p", None).await.unwrap();
        assert_eq!(offloaded.len(), 2);
        let archived_path = backend_root.join(&offloaded[0].key);

        s.fetch_offloaded(&root, "p", "same.bin", 1).await.unwrap();
        assert!(archived_path.exists());
        s.fetch_offloaded(&root, "p", "same.bin", 2).await.unwrap();
        assert!(!archived_path.exists());

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// 退避先設定を変更しても、索引に記録した元の保存先から取り戻せる。
    #[tokio::test]
    async fn fetch_uses_backend_configuration_recorded_at_rotation_time() {
        let root = temp_project().await;
        let backend_a = temp_project().await;
        let backend_b = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_a);
        let f = root.join("a.bin");
        let body = b"original-backend";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        age_stored_at(&s, &root, "p", "a.bin", 1, 10).await;
        s.rotate(&root, "p", &[], false).await.unwrap();

        let reconfigured = rotation_store(".synergos/history", 1, &backend_b);
        reconfigured
            .fetch_offloaded(&root, "p", "a.bin", 1)
            .await
            .unwrap();
        let restored = reconfigured.lookup(&root, "p", "a.bin", 1).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(restored.path).await.unwrap(), body);

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_a).await;
        let _ = tokio::fs::remove_dir_all(&backend_b).await;
    }

    /// offloaded 参照を sidecar に残し、index.json 破損時にも一覧を再構築できる。
    #[tokio::test]
    async fn corrupt_index_rebuild_preserves_offloaded_versions() {
        let root = temp_project().await;
        let backend_root = temp_project().await;
        let s = rotation_store(".synergos/history", 1, &backend_root);
        let f = root.join("a.bin");
        let body = b"recoverable-offload";
        tokio::fs::write(&f, body).await.unwrap();
        s.archive(
            &root,
            "p",
            "a.bin",
            1,
            body.len() as u64,
            crc32fast::hash(body),
            "peer",
            "published",
            &f,
        )
        .await
        .unwrap();
        age_stored_at(&s, &root, "p", "a.bin", 1, 10).await;
        s.rotate(&root, "p", &[], false).await.unwrap();

        let store_dir = s.store_dir(&root, "p").unwrap();
        tokio::fs::write(
            store_dir.join(crate::history::index::INDEX_FILE),
            b"not-json",
        )
            .await
            .unwrap();
        let rebuilt = s.offloaded(&root, "p", None).await.unwrap();
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].rel, "a.bin");

        let _ = tokio::fs::remove_dir_all(&root).await;
        let _ = tokio::fs::remove_dir_all(&backend_root).await;
    }

    /// テスト補助: index エントリの `stored_at` を `days_ago` 日前に書き換える
    /// (ローテーション候補にするための時間経過シミュレーション)。
    async fn age_stored_at(
        s: &HistoryStore,
        root: &std::path::Path,
        project_id: &str,
        rel: &str,
        version: u64,
        days_ago: u64,
    ) {
        let store_dir = s.store_dir(root, project_id).unwrap();
        let mut index = HistoryIndex::load_or_rebuild(&store_dir, project_id)
            .await
            .unwrap();
        let mut entry = index.get(rel, version).unwrap().clone();
        entry.stored_at = entry
            .stored_at
            .saturating_sub(days_ago * 24 * 60 * 60 * 1000 + 1);
        index.insert(rel, version, entry);
        index.save(&store_dir).await.unwrap();
    }
}
