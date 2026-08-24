//! `HookRunner` を `Exchange` の `post_receive_hook` に束ねる。
//! daemon 起動時に呼ばれる (`history::wiring::build_hooks` と同じ形)。

use std::sync::Arc;

use crate::exchange::PostReceiveHook;
use crate::project::ProjectManager;

use super::{HookEvent, HookRunner};

/// 受信完了フックを組み立てる。`project_root` / `rel_path` は
/// `ProjectManager` (resolve_file_path) から引く。
pub fn build_post_receive_hook(
    runner: Arc<HookRunner>,
    projects: Arc<ProjectManager>,
) -> PostReceiveHook {
    Arc::new(move |project_id, file_id, version, sender| {
        let Some(root) = projects.project_root(&project_id) else {
            tracing::debug!(
                "post-receive hook skipped: project {project_id} is not open"
            );
            return;
        };
        let Some(abs_path) = projects.resolve_file_path(&project_id, &file_id) else {
            tracing::debug!(
                "post-receive hook skipped: cannot resolve path for {}/{}",
                project_id,
                file_id
            );
            return;
        };
        let Ok(rel_path) = abs_path.strip_prefix(&root) else {
            return;
        };
        let rel_path = crate::manifest::normalize_rel_path(rel_path);
        runner.spawn_post_hooks(
            root,
            HookEvent::PostReceive,
            project_id,
            rel_path,
            version,
            Some(sender.0),
        );
    })
}
