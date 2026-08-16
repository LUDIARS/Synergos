//! `project checkout` / `project restore` — 作業ツリーを manifest の版に合わせる
//! (docs/versioning-design.md §3.4)。
//!
//! - checkout: 対象 manifest (既定はディスク上の `.synergos/manifest.json` =
//!   `git checkout` 後の状態) の各エントリと作業ツリーを突合し、違う版は
//!   `FileWant(version)` を出す。手元より古い版でも受け入れるよう Exchange に pin する。
//!   実体は履歴ノード (旧版) か publisher (最新版) から非同期に届く。
//! - restore: 1 ファイルを指定版に戻す。自ノードが履歴ノードで保管庫に実体が
//!   あればネットワークを使わずローカルで差し替える。無ければ pin + FileWant。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use synergos_net::types::FileId;

use crate::exchange::{Exchange, FetchRequest, FileSharing, SharedFileRecord, TransferPriority};
use crate::history::HistoryStore;
use crate::manifest::{
    crc32_of_file, replace_file_atomically, safe_join_under_root, safe_rel_to_local,
    ProjectManifest,
};
use crate::project::ProjectManager;

/// checkout の結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckoutReport {
    /// FileWant(version) を出した (rel, version)
    pub requested: Vec<(String, u64)>,
    /// 既に一致していた件数
    pub up_to_date: usize,
    /// 対象 manifest に無いが手元台帳にある rel (触らない)
    pub extra: Vec<String>,
}

/// restore の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// 保管庫から直接差し替えた (履歴ノード自身)
    RestoredLocally,
    /// FileWant(version) を出した。実体は非同期に届く
    Requested,
    /// 作業ツリーが既にその版だった
    AlreadyAtVersion,
}

/// checkout / restore が使う依存の束。
pub struct CheckoutContext<'a> {
    pub projects: &'a Arc<ProjectManager>,
    pub exchange: &'a Arc<Exchange>,
    pub history: &'a Arc<HistoryStore>,
}

/// 作業ツリーを manifest に合わせる。
pub async fn checkout_project(
    ctx: &CheckoutContext<'_>,
    project_id: &str,
    manifest_path: Option<&Path>,
) -> io::Result<CheckoutReport> {
    let root = ctx
        .projects
        .project_root(project_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "project is not open"))?;
    let previous: Vec<String> = ctx
        .projects
        .manifest_entries(project_id)
        .into_iter()
        .map(|(rel, _)| rel)
        .collect();
    let target = match manifest_path {
        Some(path) => {
            let m = ProjectManifest::load_from_file(path, project_id).await?;
            ctx.projects.replace_manifest(project_id, m.clone()).await?;
            m
        }
        None => ctx.projects.reload_manifest(project_id).await?,
    };

    let mut report = CheckoutReport::default();
    for (rel, entry) in &target.files {
        let Some(path) = resolve_local(&root, rel) else {
            tracing::warn!("checkout: unsafe path in manifest ignored: {rel}");
            continue;
        };
        let file_id = FileId::new(rel.clone());
        if matches_entry(&path, entry.size, entry.crc).await {
            // 作業ツリーは既にこの版。台帳 (shared_files) もこの版に揃える。
            ctx.exchange.restore_shared_file(
                file_id,
                SharedFileRecord {
                    project_id: project_id.to_string(),
                    file_path: path,
                    file_size: entry.size,
                    crc: entry.crc,
                    version: entry.version,
                },
            );
            report.up_to_date += 1;
            continue;
        }
        request_version(
            ctx.exchange,
            project_id,
            &file_id,
            entry.version,
            Some((entry.size, entry.crc)),
        )
        .await?;
        report.requested.push((rel.clone(), entry.version));
    }
    report.extra = previous
        .into_iter()
        .filter(|rel| !target.files.contains_key(rel))
        .collect();
    Ok(report)
}

/// 1 ファイルを指定版に戻す。
pub async fn restore_file(
    ctx: &CheckoutContext<'_>,
    project_id: &str,
    rel: &str,
    version: u64,
) -> io::Result<RestoreOutcome> {
    let root = ctx
        .projects
        .project_root(project_id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "project is not open"))?;
    let path = resolve_local(&root, rel)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe relative path"))?;
    let file_id = FileId::new(rel.to_string());

    // 自ノードが履歴ノードで実体を持っていればローカルで差し替える
    if let Some(stored) = ctx.history.lookup(&root, project_id, rel, version).await? {
        if matches_entry(&path, stored.size, stored.crc).await {
            ctx.exchange.restore_shared_file(
                file_id,
                SharedFileRecord {
                    project_id: project_id.to_string(),
                    file_path: path,
                    file_size: stored.size,
                    crc: stored.crc,
                    version,
                },
            );
            ctx.projects
                .record_received_file(
                    project_id,
                    rel,
                    version,
                    stored.size,
                    stored.crc,
                    &stored.publisher,
                    true,
                )
                .await?;
            return Ok(RestoreOutcome::AlreadyAtVersion);
        }
        let incoming = ProjectManifest::incoming_dir(&root).await?;
        // 置き先のディレクトリを先に用意する。コピーを先にすると
        // create_dir_all が失敗したときに .part がそのまま残る。
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = incoming.join(format!("restore-{}.part", uuid::Uuid::new_v4()));
        tokio::fs::copy(&stored.path, &tmp).await?;
        let checked = resolve_local(&root, rel).ok_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, "destination changed")
        })?;
        if checked != path {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "destination changed during restore",
            ));
        }
        if let Err(error) = replace_file_atomically(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        ctx.exchange.restore_shared_file(
            file_id,
            SharedFileRecord {
                project_id: project_id.to_string(),
                file_path: path,
                file_size: stored.size,
                crc: stored.crc,
                version,
            },
        );
        ctx.projects
            .record_received_file(
                project_id,
                rel,
                version,
                stored.size,
                stored.crc,
                &stored.publisher,
                true,
            )
            .await?;
        return Ok(RestoreOutcome::RestoredLocally);
    }

    request_version(ctx.exchange, project_id, &file_id, version, None).await?;
    Ok(RestoreOutcome::Requested)
}

fn resolve_local(root: &Path, rel: &str) -> Option<PathBuf> {
    safe_rel_to_local(rel)?;
    safe_join_under_root(root, rel)
}

async fn matches_entry(path: &Path, size: u64, crc: u32) -> bool {
    if !path.is_file() {
        return false;
    }
    match crc32_of_file(path).await {
        Ok((actual_crc, actual_size)) => actual_size == size && actual_crc == crc,
        Err(_) => false,
    }
}

async fn request_version(
    exchange: &Arc<Exchange>,
    project_id: &str,
    file_id: &FileId,
    version: u64,
    expected_size_crc: Option<(u64, u32)>,
) -> io::Result<()> {
    exchange.pin_fetch_version(project_id, file_id, version, expected_size_crc);
    exchange
        .fetch_file(FetchRequest {
            project_id: project_id.to_string(),
            file_id: file_id.clone(),
            source_peer: None,
            priority: TransferPriority::Interactive,
            version,
        })
        .await
        .map_err(|e| {
            exchange.unpin_fetch_version(project_id, file_id, version);
            io::Error::other(e.to_string())
        })?;
    Ok(())
}
