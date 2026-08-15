//! 起動時復元 — プロジェクトマニフェスト (`.synergos/manifest.json`) から
//! `Exchange` の shared_files を再構築する。
//!
//! これが無いと publisher が再起動した瞬間に「持っているのに FileWant に
//! 応答できない」「受信側が同じバージョンを再 pull する」状態になる。

use std::sync::Arc;

use synergos_net::types::FileId;

use crate::exchange::{Exchange, SharedFileRecord};
use crate::manifest::{crc32_of_file, safe_join_under_root, safe_rel_to_local};
use crate::project::{ProjectConfiguration, ProjectManager};

/// 全 open プロジェクトのマニフェストを走査して shared_files に登録する。
/// 実ファイルが消えているエントリはスキップ (存在しないものを Offer しない)。
/// 戻り値は登録件数。
pub async fn restore_shared_files_from_manifests(
    project_manager: &Arc<ProjectManager>,
    exchange: &Arc<Exchange>,
) -> usize {
    let mut restored = 0usize;
    for info in ProjectConfiguration::list_projects(&**project_manager) {
        let Some(root) = project_manager.project_root(&info.project_id) else {
            continue;
        };
        for (rel, entry) in project_manager.manifest_entries(&info.project_id) {
            let Some(local) = safe_rel_to_local(&rel) else {
                tracing::warn!("restore: unsafe path in manifest ignored: {}", rel);
                continue;
            };
            let Some(path) = safe_join_under_root(&root, &rel) else {
                tracing::warn!("restore: symlinked path in manifest ignored: {}", rel);
                continue;
            };
            if !path.is_file() {
                tracing::debug!(
                    "restore: {}/{} listed in manifest but missing on disk; skipped",
                    info.project_id,
                    rel
                );
                continue;
            }
            let Ok((actual_crc, actual_size)) = crc32_of_file(&path).await else {
                tracing::warn!(
                    "restore: could not verify {}/{}; skipped",
                    info.project_id,
                    rel
                );
                continue;
            };
            if actual_size != entry.size || actual_crc != entry.crc {
                tracing::warn!(
                    "restore: {}/{} differs from its published manifest; skipped until republished",
                    info.project_id,
                    rel
                );
                continue;
            }
            let file_id = FileId::new(rel.clone());
            project_manager.register_file(&info.project_id, file_id.clone(), local);
            exchange.restore_shared_file(
                file_id,
                SharedFileRecord {
                    project_id: info.project_id.clone(),
                    file_path: path,
                    file_size: entry.size,
                    crc: entry.crc,
                    version: entry.version,
                },
            );
            restored += 1;
        }
    }
    if restored > 0 {
        tracing::info!("restored {restored} shared file record(s) from project manifests");
    }
    restored
}
