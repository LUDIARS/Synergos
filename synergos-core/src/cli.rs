//! CLI コマンドハンドラ
//!
//! synergos-core のコマンドラインインターフェース。
//! デーモン起動・停止やプロジェクト管理などのサブコマンドを提供する。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::daemon::Daemon;

/// Synergos Core Daemon
#[derive(Parser)]
#[command(name = "synergos-core", about = "Synergos core daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// デーモンを起動する（フォアグラウンド）
    Start {
        /// 設定ファイルパス
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// 稼働中のデーモンを停止する
    Stop,

    /// デーモンの状態を確認する
    Status,

    /// プロジェクト管理
    #[command(subcommand)]
    Project(ProjectCommand),

    /// ピア管理
    #[command(subcommand)]
    Peer(PeerCommand),

    /// 転送管理
    #[command(subcommand)]
    Transfer(TransferCommand),

    /// ネットワーク状態
    Network,
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// プロジェクトを開く
    Open {
        /// プロジェクトID
        id: String,
        /// プロジェクトルートパス
        path: PathBuf,
        /// 表示名
        #[arg(short = 'n', long)]
        name: Option<String>,
    },
    /// プロジェクトを閉じる
    Close {
        /// プロジェクトID
        id: String,
    },
    /// プロジェクト一覧
    List,
    /// プロジェクト詳細
    Get {
        /// プロジェクトID
        id: String,
    },
    /// プロジェクト設定を更新
    Update {
        /// プロジェクトID
        id: String,
        /// 表示名
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// 説明
        #[arg(short, long)]
        description: Option<String>,
        /// 同期モード (full / manual / selective)
        #[arg(short, long)]
        sync_mode: Option<String>,
        /// 最大接続ピア数
        #[arg(short, long)]
        max_peers: Option<u16>,
    },
    /// 招待トークンを生成
    Invite {
        /// プロジェクトID
        id: String,
        /// 有効期限（秒）
        #[arg(short, long)]
        expires: Option<u64>,
    },
    /// 招待トークンで参加
    Join {
        /// 招待トークン
        token: String,
        /// ローカルのプロジェクトルートパス
        path: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum PeerCommand {
    /// ピア一覧
    List {
        /// プロジェクトID
        project: String,
    },
    /// ピアに接続
    Connect {
        /// プロジェクトID
        project: String,
        /// ピアID
        peer: String,
    },
    /// ピアを切断
    Disconnect {
        /// ピアID
        peer: String,
    },
}

#[derive(Subcommand)]
pub enum TransferCommand {
    /// 転送一覧
    List {
        /// プロジェクトIDでフィルタ
        #[arg(short, long)]
        project: Option<String>,
    },
    /// 転送をキャンセル
    Cancel {
        /// 転送ID
        id: String,
    },
}

/// CLI コマンドを実行する
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Start { config } => {
            tracing::info!("Starting Synergos core daemon...");
            let daemon_config = crate::daemon::DaemonConfig {
                config_path: config,
                ..Default::default()
            };
            let daemon = Daemon::new(daemon_config).await?;
            daemon.run().await?;
        }
        Command::Stop => {
            tracing::info!("Stopping Synergos core daemon...");
            let mut client = synergos_ipc::IpcClient::connect().await?;
            client
                .send(synergos_ipc::IpcCommand::Shutdown)
                .await?;
            println!("Daemon stopped.");
        }
        Command::Status => {
            let mut client = synergos_ipc::IpcClient::connect().await?;
            let resp = client
                .send(synergos_ipc::IpcCommand::Status)
                .await?;
            match resp {
                synergos_ipc::IpcResponse::Status(status) => {
                    println!("Synergos Core Daemon");
                    println!("  PID:         {}", status.pid);
                    println!("  Projects:    {}", status.project_count);
                    println!("  Connections: {}", status.active_connections);
                    println!("  Transfers:   {}", status.active_transfers);
                }
                _ => println!("Unexpected response"),
            }
        }
        Command::Project(cmd) => handle_project(cmd).await?,
        Command::Peer(cmd) => handle_peer(cmd).await?,
        Command::Transfer(cmd) => handle_transfer(cmd).await?,
        Command::Network => {
            let mut client = synergos_ipc::IpcClient::connect().await?;
            let resp = client
                .send(synergos_ipc::IpcCommand::NetworkStatus)
                .await?;
            match resp {
                synergos_ipc::IpcResponse::NetworkStatus(info) => {
                    println!("Network Status");
                    println!("  Route:       {}", info.primary_route);
                    println!("  Connections: {}/{}", info.active_connections, info.max_connections);
                    println!("  Bandwidth:   {} bps", info.total_bandwidth_bps);
                    println!("  Latency:     {} ms", info.avg_latency_ms);
                }
                _ => println!("Unexpected response"),
            }
        }
    }
    Ok(())
}

async fn handle_project(cmd: ProjectCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        ProjectCommand::Open { id, path, name } => {
            client
                .send(synergos_ipc::IpcCommand::ProjectOpen {
                    project_id: id,
                    root_path: path,
                    display_name: name,
                })
                .await?;
            println!("Project opened.");
        }
        ProjectCommand::Close { id } => {
            client
                .send(synergos_ipc::IpcCommand::ProjectClose {
                    project_id: id,
                })
                .await?;
            println!("Project closed.");
        }
        ProjectCommand::List => {
            let resp = client
                .send(synergos_ipc::IpcCommand::ProjectList)
                .await?;
            if let synergos_ipc::IpcResponse::ProjectList(projects) = resp {
                if projects.is_empty() {
                    println!("No active projects.");
                } else {
                    for p in projects {
                        println!(
                            "  {} [{}] ({}) — {} peers",
                            p.display_name, p.project_id, p.root_path, p.peer_count
                        );
                    }
                }
            }
        }
        ProjectCommand::Get { id } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::ProjectGet { project_id: id })
                .await?;
            match resp {
                synergos_ipc::IpcResponse::ProjectDetail(d) => {
                    println!("Project: {} [{}]", d.display_name, d.project_id);
                    println!("  Path:        {}", d.root_path);
                    println!("  Description: {}", d.description);
                    println!("  Sync mode:   {}", d.sync_mode);
                    println!("  Max peers:   {}", if d.max_peers == 0 { "unlimited".to_string() } else { d.max_peers.to_string() });
                    println!("  Peers:       {}", d.peer_count);
                    println!("  Transfers:   {}", d.active_transfers);
                    if !d.connected_peer_ids.is_empty() {
                        println!("  Connected:   {}", d.connected_peer_ids.join(", "));
                    }
                }
                synergos_ipc::IpcResponse::Error { message, .. } => {
                    eprintln!("Error: {}", message);
                }
                _ => println!("Unexpected response"),
            }
        }
        ProjectCommand::Update {
            id,
            name,
            description,
            sync_mode,
            max_peers,
        } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::ProjectUpdate {
                    project_id: id,
                    display_name: name,
                    description,
                    sync_mode,
                    max_peers,
                })
                .await?;
            match resp {
                synergos_ipc::IpcResponse::Ok => println!("Project updated."),
                synergos_ipc::IpcResponse::Error { message, .. } => {
                    eprintln!("Error: {}", message);
                }
                _ => println!("Unexpected response"),
            }
        }
        ProjectCommand::Invite { id, expires } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::ProjectCreateInvite {
                    project_id: id,
                    expires_in_secs: expires,
                })
                .await?;
            match resp {
                synergos_ipc::IpcResponse::InviteToken { token, expires_at } => {
                    println!("Invite token: {}", token);
                    if let Some(exp) = expires_at {
                        println!("Expires at:   {} (epoch)", exp);
                    } else {
                        println!("Expires:      never");
                    }
                }
                synergos_ipc::IpcResponse::Error { message, .. } => {
                    eprintln!("Error: {}", message);
                }
                _ => println!("Unexpected response"),
            }
        }
        ProjectCommand::Join { token, path } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::ProjectJoin {
                    invite_token: token,
                    root_path: path,
                })
                .await?;
            match resp {
                synergos_ipc::IpcResponse::Ok => println!("Joined project."),
                synergos_ipc::IpcResponse::Error { message, .. } => {
                    eprintln!("Error: {}", message);
                }
                _ => println!("Unexpected response"),
            }
        }
    }
    Ok(())
}

async fn handle_peer(cmd: PeerCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        PeerCommand::List { project } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::PeerList {
                    project_id: project,
                })
                .await?;
            if let synergos_ipc::IpcResponse::PeerList(peers) = resp {
                if peers.is_empty() {
                    println!("No connected peers.");
                } else {
                    for p in peers {
                        println!(
                            "  {} ({}) — {} | {} ms | {} bps",
                            p.display_name, p.peer_id, p.route, p.rtt_ms, p.bandwidth_bps
                        );
                    }
                }
            }
        }
        PeerCommand::Connect { project, peer } => {
            client
                .send(synergos_ipc::IpcCommand::PeerConnect {
                    project_id: project,
                    peer_id: peer,
                })
                .await?;
            println!("Connection initiated.");
        }
        PeerCommand::Disconnect { peer } => {
            client
                .send(synergos_ipc::IpcCommand::PeerDisconnect { peer_id: peer })
                .await?;
            println!("Peer disconnected.");
        }
    }
    Ok(())
}

async fn handle_transfer(cmd: TransferCommand) -> anyhow::Result<()> {
    let mut client = synergos_ipc::IpcClient::connect().await?;
    match cmd {
        TransferCommand::List { project } => {
            let resp = client
                .send(synergos_ipc::IpcCommand::TransferList {
                    project_id: project,
                })
                .await?;
            if let synergos_ipc::IpcResponse::TransferList(transfers) = resp {
                if transfers.is_empty() {
                    println!("No active transfers.");
                } else {
                    for t in transfers {
                        let pct = if t.file_size > 0 {
                            (t.bytes_transferred as f64 / t.file_size as f64 * 100.0) as u32
                        } else {
                            0
                        };
                        println!(
                            "  {} {} — {}% ({} bps) [{}]",
                            t.direction, t.file_name, pct, t.speed_bps, t.state
                        );
                    }
                }
            }
        }
        TransferCommand::Cancel { id } => {
            client
                .send(synergos_ipc::IpcCommand::TransferCancel { transfer_id: id })
                .await?;
            println!("Transfer cancelled.");
        }
    }
    Ok(())
}
