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
    /// `[history.rotation]` の設定で外部ストレージへ旧版を退避する
    Rotate {
        /// プロジェクトID
        id: String,
        /// 実際には退避せず候補一覧だけ表示する
        #[arg(long)]
        dry_run: bool,
        /// この manifest が参照する版は退避しない (複数可)
        #[arg(long = "keep-manifest")]
        keep_manifests: Vec<PathBuf>,
    },
    /// 退避済み一覧 (path / version / backend / key / size)
    Offloaded {
        /// プロジェクトID
        id: String,
        /// 特定ファイル (プロジェクトルート相対、`/` 区切り) だけ表示
        path: Option<String>,
    },
    /// 退避済みの版を明示的に取り戻す
    Fetch {
        /// プロジェクトID
        id: String,
        /// 対象ファイル (プロジェクトルート相対、`/` 区切り)
        path: String,
        /// 取り戻す版番号
        #[arg(long)]
        version: u64,
    },
}

/// `synergos tag ...` — 版タグ (GC・ローテーション保護)。docs/versioning-design.md §3.5。
#[derive(Subcommand)]
pub enum TagCommand {
    /// タグを作成/上書きする。ピン集合の指定方法は 3 通り (排他):
    /// 何も指定しなければ現在の manifest、`--manifest` なら指定 manifest、
    /// `--file` + `--version` なら単一ファイル版だけをピンする。
    Add {
        /// プロジェクトID
        project: String,
        /// タグ名 (`[A-Za-z0-9._-]{1,64}`)
        name: String,
        /// この manifest の全 (path, version) をピン (git の過去コミット等から取り出した manifest)
        #[arg(long, conflicts_with = "file")]
        manifest: Option<PathBuf>,
        /// 単一ファイル版だけをピンする対象パス (`--version` と併用)
        #[arg(long, requires = "version")]
        file: Option<String>,
        /// `--file` と併用する版番号
        #[arg(long, requires = "file")]
        version: Option<u64>,
    },
    /// タグ一覧 (name / created_at / pin 数)
    Ls {
        /// プロジェクトID
        project: String,
    },
    /// ピン内容の一覧
    Show {
        /// プロジェクトID
        project: String,
        /// タグ名
        name: String,
    },
    /// タグ削除 (実体は消さない。以後 GC 対象に戻るだけ)
    Rm {
        /// プロジェクトID
        project: String,
        /// タグ名
        name: String,
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
        HistoryCommand::Rotate {
            id,
            dry_run,
            keep_manifests,
        } => {
            let resp = client
                .send(IpcCommand::HistoryRotate {
                    project_id: id,
                    dry_run,
                    keep_manifests,
                })
                .await?;
            match resp {
                IpcResponse::HistoryRotationReport(r) => {
                    if dry_run {
                        println!("History rotate (dry-run): {} candidate(s)", r.candidates.len());
                        for (rel, version) in &r.candidates {
                            println!("  {rel} v{version}");
                        }
                    } else {
                        println!(
                            "History rotate: offloaded {} version(s), {} bytes, {} skipped",
                            r.offloaded.len(),
                            r.bytes_offloaded,
                            r.skipped.len()
                        );
                        for (rel, version, reason) in &r.skipped {
                            println!("  skipped {rel} v{version}: {reason}");
                        }
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected history rotate response"),
            }
        }
        HistoryCommand::Offloaded { id, path } => {
            let resp = client
                .send(IpcCommand::HistoryOffloaded {
                    project_id: id,
                    rel_path: path,
                })
                .await?;
            match resp {
                IpcResponse::HistoryOffloaded(items) => {
                    if items.is_empty() {
                        println!("No offloaded versions.");
                    }
                    for v in items {
                        println!(
                            "  {} v{} {} bytes backend={} key={}",
                            v.rel_path, v.version, v.size, v.backend, v.key
                        );
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected history offloaded response"),
            }
        }
        HistoryCommand::Fetch { id, path, version } => {
            let resp = client
                .send(IpcCommand::HistoryFetch {
                    project_id: id,
                    rel_path: path.clone(),
                    version,
                })
                .await?;
            match resp {
                IpcResponse::Ok => println!("Fetched {path} v{version} from rotation backend."),
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected history fetch response"),
            }
        }
    }
    Ok(())
}

/// `tag ...`
pub async fn handle_tag(cmd: TagCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        TagCommand::Add {
            project,
            name,
            manifest,
            file,
            version,
        } => {
            let pins = match (file, version) {
                (Some(path), Some(v)) => vec![(path, v)],
                _ => Vec::new(),
            };
            let resp = client
                .send(IpcCommand::TagAdd {
                    project_id: project,
                    name,
                    manifest_path: manifest,
                    pins,
                })
                .await?;
            match resp {
                IpcResponse::Tag(tag) => {
                    println!(
                        "Tag {} created ({} pin(s), at {})",
                        tag.name,
                        tag.pins.len(),
                        tag.created_at
                    );
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected tag add response"),
            }
        }
        TagCommand::Ls { project } => {
            let resp = client
                .send(IpcCommand::TagLs { project_id: project })
                .await?;
            match resp {
                IpcResponse::TagList(items) => {
                    if items.is_empty() {
                        println!("No tags.");
                    }
                    for t in items {
                        println!("  {} {} pin(s) at {}", t.name, t.pin_count, t.created_at);
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected tag ls response"),
            }
        }
        TagCommand::Show { project, name } => {
            let resp = client
                .send(IpcCommand::TagShow {
                    project_id: project,
                    name,
                })
                .await?;
            match resp {
                IpcResponse::Tag(tag) => {
                    println!("Tag {} ({} at {})", tag.name, tag.pins.len(), tag.created_at);
                    for (rel, v) in &tag.pins {
                        println!("  {rel} v{v}");
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected tag show response"),
            }
        }
        TagCommand::Rm { project, name } => {
            let resp = client
                .send(IpcCommand::TagRm {
                    project_id: project,
                    name: name.clone(),
                })
                .await?;
            match resp {
                IpcResponse::Ok => println!("Tag {name} removed."),
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected tag rm response"),
            }
        }
    }
    Ok(())
}

fn escape_terminal(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
