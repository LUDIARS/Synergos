//! CLI: `hooks ls` / `hooks run` (docs/hooks.md)。IPC を叩いて結果を整形するだけ。

use clap::Subcommand;
use synergos_ipc::{IpcCommand, IpcResponse};

#[derive(Subcommand)]
pub enum HooksCommand {
    /// 有効なフック一覧 (定義元 daemon/project の別と opt-in 状態を表示)
    Ls {
        /// プロジェクトID
        project: String,
    },
    /// 手動発火 (デバッグ用)
    Run {
        /// プロジェクトID
        project: String,
        /// `pre-publish` | `post-publish` | `post-receive`
        event: String,
        /// プロジェクトルート相対パス (`/` 区切り)
        file: String,
    },
}

pub async fn handle_hooks(cmd: HooksCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        HooksCommand::Ls { project } => {
            let resp = client
                .send(IpcCommand::HooksList {
                    project_id: project,
                })
                .await?;
            match resp {
                IpcResponse::HooksList(hooks) => {
                    if hooks.is_empty() {
                        println!("No hooks configured.");
                    }
                    for h in hooks {
                        let disabled = if h.disabled_by_opt_in {
                            " [disabled: allow_project_hooks=false]"
                        } else {
                            ""
                        };
                        let matches = if h.r#match.is_empty() {
                            "*".to_string()
                        } else {
                            h.r#match.join(", ")
                        };
                        println!(
                            "  [{}] {} — {} (match={matches}, timeout={}s){disabled}",
                            h.source, h.event, h.command, h.timeout_sec
                        );
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected hooks list response"),
            }
        }
        HooksCommand::Run {
            project,
            event,
            file,
        } => {
            let resp = client
                .send(IpcCommand::HooksRun {
                    project_id: project,
                    event,
                    rel_path: file,
                })
                .await?;
            match resp {
                IpcResponse::HooksRunReport(results) => {
                    if results.is_empty() {
                        println!("No matching hooks ran.");
                    }
                    for r in results {
                        match r.status.as_str() {
                            "success" => println!("  [{}] {} — ok", r.source, r.command),
                            "failed" => println!(
                                "  [{}] {} — failed (exit={:?})",
                                r.source, r.command, r.exit_code
                            ),
                            "timed_out" => {
                                println!("  [{}] {} — timed out", r.source, r.command)
                            }
                            _ => println!(
                                "  [{}] {} — spawn error: {}",
                                r.source,
                                r.command,
                                r.detail.unwrap_or_default()
                            ),
                        }
                    }
                }
                IpcResponse::Error { message, .. } => anyhow::bail!(message),
                _ => anyhow::bail!("unexpected hooks run response"),
            }
        }
    }
    Ok(())
}
