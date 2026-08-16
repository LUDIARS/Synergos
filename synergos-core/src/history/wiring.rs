//! `HistoryStore` を Exchange のフック (`exchange::history_hooks`) に束ねる。
//! daemon 起動時に `history.enabled` のときだけ呼ばれる。

use std::sync::Arc;

use crate::exchange::history_hooks::{ArchiveHook, HistoryHit, HistoryHooks, HistoryLookup};
use crate::project::ProjectManager;

use super::HistoryStore;

/// 保管フック + lookup フックを組み立てる。
pub fn build_hooks(store: Arc<HistoryStore>, projects: Arc<ProjectManager>) -> HistoryHooks {
    let archive: ArchiveHook = {
        let store = store.clone();
        let projects = projects.clone();
        Arc::new(move |request| {
            let store = store.clone();
            let projects = projects.clone();
            Box::pin(async move {
                let Some(root) = projects.project_root(&request.project_id) else {
                    tracing::debug!(
                        "history archive skipped: project {} is not open",
                        request.project_id
                    );
                    return Ok(());
                };
                match store
                    .archive(
                        &root,
                        &request.project_id,
                        &request.file_id.0,
                        request.version,
                        request.size,
                        request.crc,
                        &request.publisher,
                        request.source,
                        &request.path,
                    )
                    .await
                {
                    Ok(outcome) => {
                        tracing::debug!(
                            "history archive {} v{} ({}): {:?}",
                            request.file_id,
                            request.version,
                            request.source,
                            outcome
                        );
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            })
        })
    };
    let lookup: HistoryLookup = Arc::new(move |project_id, file_id, version| {
        let store = store.clone();
        let projects = projects.clone();
        Box::pin(async move {
            let root = projects.project_root(&project_id)?;
            match store.lookup(&root, &project_id, &file_id.0, version).await {
                Ok(Some(hit)) => Some(HistoryHit {
                    path: hit.path,
                    size: hit.size,
                    crc: hit.crc,
                }),
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(
                        "history lookup failed for {project_id}/{file_id} v{version}: {error}"
                    );
                    None
                }
            }
        })
    });
    HistoryHooks { archive, lookup }
}
