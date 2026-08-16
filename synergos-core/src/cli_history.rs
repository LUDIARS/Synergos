//! CLI: `project checkout` / `project restore` / `history ls` / `history gc`
//! (docs/versioning-design.md §3.4)。IPC を叩いて結果を整形するだけ。

use std::path::PathBuf;

use clap::Subcommand;
use synergos_ipc::{IpcCommand, IpcResponse};

#[derive(Subcommand)]
pub enum HistoryCommand {
    /// 履歴ノード上の保持版一覧 (このノードが履歴ノードでなければ空)
    Ls {
        /// プロジェクトID
        id: String,
        /// 特定ファイル (プロジェクトルート相対、`/` 区切り) だけ表示
        path: Option<String>,
    },
    /// 保持ポリシー ([history] max_versions_per_file / max_age_days / max_bytes) を適用する
    Gc {
        /// プロジェクトID
        id: String,
        /// 保管庫を全消去する
        #[arg(long)]
        purge: bool,
        /// この manifest が参照する版は削らない (複数可。例: git の各タグ時点の manifest)
        #[arg(long = "keep-manifest")]
        keep_manifests: Vec<PathBuf>,
    },
}

/// `project checkout`
pub async fn checkout(
    client: &mut synergos_ipc::IpcClient,
    id: String,
    manifest: Option<PathBuf>,
) -> anyhow::Result<()> {
    let resp = client
        .send(IpcCommand::ProjectCheckout {
            project_id: id,
            manifest_path: manifest,
        })
        .await?;
    match resp {
        IpcResponse::CheckoutReport(report) => {
            println!(
                "Checkout: {} up to date, {} requested, {} extra",
                report.up_to_date,
                report.requested.len(),
                report.extra.len()
            );
            for (rel, version) in &report.requested {
                println!("  requested {rel} v{version} (arrives from a history node / publisher)");
            }
            for rel in &report.extra {
                println!("  extra     {rel} (not in manifest; left untouched)");
            }
        }
        IpcResponse::Error { message, .. } => anyhow::bail!(message),
        _ => anyhow::bail!("unexpected checkout response"),
    }
    Ok(())
}

/// `project restore`
pub async fn restore(
    client: &mut synergos_ipc::IpcClient,
    id: String,
    path: String,
    version: u64,
) -> anyhow::Result<()> {
    let resp = client
        .send(IpcCommand::ProjectRestore {
            project_id: id,
            rel_path: path.clone(),
            version,
        })
        .await?;
    match resp {
        IpcResponse::Ok => println!("Restore of {path} v{version} requested / applied."),
        IpcResponse::Error { message, .. } => anyhow::bail!(message),
        _ => anyhow::bail!("unexpected restore response"),
    }
    Ok(())
}

/// `history ...`
pub async fn handle_history(cmd: HistoryCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        HistoryCommand::Ls { id, path } => {
            let resp = client
                .send(IpcCommand::HistoryList {
                    project_id: id,
                    rel_path: path,
                })
                .await?;
            match resp {
                IpcResponse::HistoryList(items) => {
                    if items.is_empty() {
                        println!("No stored versions (is this node a history node?).");
                    }
                    for v in items {
                        let source = escape_terminal(&v.source);
                        let publisher = escape_terminal(&v.publisher);
                        println!(
                            "  {} v{} {} bytes crc={:08x} {} {} at {}",
                            v.rel_path,
                            v.version,
                            v.size,
                            v.crc,
                            source,
                            publisher,
                            v.stored_at
                        );
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected history list response"),
            }
        }
        HistoryCommand::Gc {
            id,
            purge,
            keep_manifests,
        } => {
            let resp = client
                .send(IpcCommand::HistoryGc {
                    project_id: id,
                    purge,
                    keep_manifests,
                })
                .await?;
            match resp {
                IpcResponse::HistoryGcReport(r) => {
                    println!(
                        "History gc: removed {} version(s), {} object(s), {} bytes freed",
                        r.removed_versions.len(),
                        r.removed_objects,
                        r.bytes_freed
                    );
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected history gc response"),
            }
        }
    }
    Ok(())
}

fn escape_terminal(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
