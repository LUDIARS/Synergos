//! CLI: `project status` / `project publish --all`。
//! daemon からプロジェクトルートを引き、作業ツリーと manifest の突合
//! (`worktree_status`) は CLI プロセス側で行う (daemon と同じマシンで動く前提)。

use std::path::PathBuf;

use synergos_ipc::{IpcClient, IpcCommand, IpcResponse};

use crate::worktree_status::{worktree_status, WorktreeStatus};

/// IPC の件数上限は 1024 だが、各パスが最大長でも 1 MiB の transport 上限を
/// 超えないよう、`--all` はさらに小さい単位で送る。
const PUBLISH_BATCH_SIZE: usize = 128;

/// `project status <id>`
pub async fn status(client: &mut IpcClient, id: String) -> anyhow::Result<()> {
    let root = project_root(client, &id).await?;
    let status = worktree_status(&root, &id).await?;
    print_status(&status);
    Ok(())
}

/// `project publish <id> --all`: 変更・新規のファイルだけを publish する。
pub async fn publish_all(client: &mut IpcClient, id: String) -> anyhow::Result<()> {
    let root = project_root(client, &id).await?;
    let status = worktree_status(&root, &id).await?;
    let files = status.publishable();
    if files.is_empty() {
        println!("Nothing to publish (working tree matches manifest).");
        return Ok(());
    }
    let count = files.len();
    let mut published = 0;
    for batch in files.chunks(PUBLISH_BATCH_SIZE) {
        let resp = client
            .send(IpcCommand::PublishUpdate {
                project_id: id.clone(),
                file_paths: batch.iter().map(|rel| root.join(rel)).collect(),
            })
            .await?;
        match resp {
            IpcResponse::Ok => published += batch.len(),
            IpcResponse::Error { message, .. } => anyhow::bail!(
                "publish failed after {published} of {count} file(s) were accepted: {message}"
            ),
            _ => anyhow::bail!(
                "unexpected publish response after {published} of {count} file(s) were accepted"
            ),
        }
    }
    println!("Published {count} file(s):");
    for rel in &files {
        println!("  {rel}");
    }
    if !status.missing.is_empty() {
        println!(
            "note: {} file(s) in manifest are missing locally (not published as deletions). \
             Run `project status` for the list, `project checkout` to fetch them back.",
            status.missing.len()
        );
    }
    Ok(())
}

async fn project_root(client: &mut IpcClient, id: &str) -> anyhow::Result<PathBuf> {
    let resp = client
        .send(IpcCommand::ProjectGet {
            project_id: id.to_string(),
        })
        .await?;
    match resp {
        IpcResponse::ProjectDetail(d) => Ok(PathBuf::from(d.root_path)),
        IpcResponse::Error { message, .. } => anyhow::bail!(message),
        _ => anyhow::bail!("unexpected project response"),
    }
}

fn print_status(status: &WorktreeStatus) {
    if status.is_clean() {
        println!(
            "Working tree matches manifest ({} file(s)).",
            status.unchanged
        );
        return;
    }
    print_section(
        "Modified (publish with `project publish --all`)",
        &status.modified,
    );
    print_section(
        "Untracked (publish with `project publish --all`)",
        &status.untracked,
    );
    print_section("Missing (fetch with `project checkout`)", &status.missing);
    println!("Unchanged: {}", status.unchanged);
}

fn print_section(title: &str, files: &[String]) {
    if files.is_empty() {
        return;
    }
    println!("{title}: {}", files.len());
    for rel in files {
        println!("  {rel}");
    }
}
