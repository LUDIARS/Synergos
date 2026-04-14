//! IPC サーバー
//!
//! クロスプラットフォーム IPC サーバー。
//! - Linux / macOS: Unix Domain Socket
//! - Windows: Named Pipe（将来実装）
//!
//! クライアント（GUI / CLI / Ars Plugin）からのコマンドを受け付け、
//! EventBus と連携してレスポンス・イベントプッシュを行う。

use std::sync::Arc;
use tokio::sync::broadcast;

use synergos_ipc::command::IpcCommand;
use synergos_ipc::response::{
    DaemonStatus, IpcResponse, NetworkStatusInfo, PeerInfo, TransferInfo,
};
use synergos_ipc::transport::{IpcError, IpcTransport};
use synergos_net::types::{FileId, PeerId, TransferId};

use crate::conflict::ConflictManager;
use crate::event_bus::SharedEventBus;
use crate::exchange::{
    Exchange, FetchRequest, FileSharing, PublishNotification, TransferDirection, TransferPriority,
    TransferState,
};
use crate::presence::{NodeRegistry, PeerState, PresenceService};
use crate::project::{ProjectConfiguration, ProjectManager, ProjectSettingsPatch};

/// サービスへの共有参照をまとめた構造体
pub struct ServiceContext {
    pub event_bus: SharedEventBus,
    pub project_manager: Arc<ProjectManager>,
    pub exchange: Arc<Exchange>,
    pub presence: Arc<PresenceService>,
    pub conflict_manager: Arc<ConflictManager>,
    pub shutdown_tx: broadcast::Sender<()>,
    pub started_at: u64,
}

/// IPC サーバー
pub struct IpcServer {
    ctx: Arc<ServiceContext>,
}

impl IpcServer {
    pub fn new(ctx: Arc<ServiceContext>) -> Self {
        Self { ctx }
    }

    /// IPC サーバーを起動する
    #[cfg(unix)]
    pub async fn run(&self) -> Result<(), IpcError> {
        let path = synergos_ipc::transport::socket_path();

        // ソケットディレクトリを作成
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 既存ソケットファイルを削除
        let _ = tokio::fs::remove_file(&path).await;

        let listener = tokio::net::UnixListener::bind(&path)?;
        tracing::info!("IPC server listening on {}", path.display());

        let mut shutdown_rx = self.ctx.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let ctx = self.ctx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, ctx).await {
                                    tracing::warn!("Client connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("IPC server shutting down");
                    break;
                }
            }
        }

        // ソケットファイルをクリーンアップ
        let _ = tokio::fs::remove_file(&path).await;
        Ok(())
    }

    /// Windows 用の IPC サーバー（スタブ）
    #[cfg(windows)]
    pub async fn run(&self) -> Result<(), IpcError> {
        tracing::warn!("Windows Named Pipe server not yet implemented");
        let mut shutdown_rx = self.ctx.shutdown_tx.subscribe();
        let _ = shutdown_rx.recv().await;
        Ok(())
    }
}

/// クライアント接続のハンドリング
#[cfg(unix)]
async fn handle_client(
    stream: tokio::net::UnixStream,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError> {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        let command: IpcCommand = match IpcTransport::read_message(&mut reader).await {
            Ok(cmd) => cmd,
            Err(IpcError::ConnectionClosed) => {
                tracing::debug!("Client disconnected");
                break;
            }
            Err(e) => return Err(e),
        };

        tracing::debug!("Received command: {:?}", command);

        let response = dispatch_command(command, &ctx).await;

        IpcTransport::write_message(&mut writer, &response).await?;
    }

    Ok(())
}

/// コマンドをディスパッチしてレスポンスを生成
async fn dispatch_command(command: IpcCommand, ctx: &ServiceContext) -> IpcResponse {
    match command {
        IpcCommand::Ping => IpcResponse::Pong,

        IpcCommand::Shutdown => {
            tracing::info!("Shutdown requested via IPC");
            let _ = ctx.shutdown_tx.send(());
            IpcResponse::Ok
        }

        IpcCommand::Status => {
            let transfers = ctx.exchange.list_transfers(None).await;
            let active_transfers = transfers
                .iter()
                .filter(|t| t.state == TransferState::Running || t.state == TransferState::Queued)
                .count();

            let peers = ctx.presence.list_nodes(None).await;
            let active_connections = peers
                .iter()
                .filter(|p| p.state == PeerState::Connected || p.state == PeerState::Idle)
                .count();

            let status = DaemonStatus {
                pid: std::process::id(),
                started_at: ctx.started_at,
                project_count: ctx.project_manager.count(),
                active_connections,
                active_transfers,
            };
            IpcResponse::Status(status)
        }

        // ── プロジェクト管理 ──
        IpcCommand::ProjectOpen {
            project_id,
            root_path,
            display_name,
        } => match ctx
            .project_manager
            .open_project(project_id, root_path, display_name)
            .await
        {
            Ok(()) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error {
                code: 1,
                message: e.to_string(),
            },
        },

        IpcCommand::ProjectClose { project_id } => {
            match ctx.project_manager.close_project(&project_id).await {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 1,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::ProjectList => {
            let projects = ctx.project_manager.list_projects();
            IpcResponse::ProjectList(projects)
        }

        IpcCommand::ProjectGet { project_id } => {
            match ctx.project_manager.get_project(&project_id).await {
                Ok(detail) => IpcResponse::ProjectDetail(detail),
                Err(e) => IpcResponse::Error {
                    code: 1,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::ProjectUpdate {
            project_id,
            display_name,
            description,
            sync_mode,
            max_peers,
        } => {
            let patch = ProjectSettingsPatch {
                display_name,
                description,
                sync_mode,
                max_peers,
            };
            match ctx.project_manager.update_project(&project_id, patch).await {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 1,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::ProjectCreateInvite {
            project_id,
            expires_in_secs,
        } => match ctx
            .project_manager
            .create_invite(&project_id, expires_in_secs)
            .await
        {
            Ok(invite) => IpcResponse::InviteToken {
                token: invite.token,
                expires_at: invite.expires_at,
            },
            Err(e) => IpcResponse::Error {
                code: 1,
                message: e.to_string(),
            },
        },

        IpcCommand::ProjectJoin {
            invite_token,
            root_path,
        } => match ctx
            .project_manager
            .join_project(&invite_token, root_path)
            .await
        {
            Ok(_project_id) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error {
                code: 1,
                message: e.to_string(),
            },
        },

        // ── ピア管理 ──
        IpcCommand::PeerList { project_id } => {
            let nodes = ctx.presence.list_nodes(Some(&project_id)).await;
            let peers: Vec<PeerInfo> = nodes
                .into_iter()
                .map(|n| PeerInfo {
                    peer_id: n.peer_id.to_string(),
                    display_name: n.display_name,
                    route: format!("{:?}", n.endpoints.first()),
                    rtt_ms: n.rtt_ms.unwrap_or(0),
                    bandwidth_bps: n.bandwidth_bps,
                    state: format!("{:?}", n.state),
                })
                .collect();
            IpcResponse::PeerList(peers)
        }

        IpcCommand::PeerConnect {
            project_id,
            peer_id,
        } => {
            let registration = crate::presence::NodeRegistration {
                peer_id: PeerId::new(&peer_id),
                display_name: peer_id.clone(),
                endpoints: vec![],
                project_ids: vec![project_id],
            };
            match ctx.presence.register_node(registration).await {
                Ok(_) => {
                    // ノードを Connected 状態に更新
                    let _ = ctx
                        .presence
                        .update_node_state(&PeerId::new(&peer_id), PeerState::Connected)
                        .await;
                    IpcResponse::Ok
                }
                Err(e) => IpcResponse::Error {
                    code: 2,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::PeerDisconnect { peer_id } => {
            match ctx.presence.unregister_node(&PeerId::new(&peer_id)).await {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 2,
                    message: e.to_string(),
                },
            }
        }

        // ── ファイル転送 ──
        IpcCommand::TransferRequest {
            project_id,
            file_id,
            peer_id,
        } => {
            let request = FetchRequest {
                project_id,
                file_id: FileId::new(file_id),
                source_peer: Some(PeerId::new(peer_id)),
                priority: TransferPriority::Interactive,
            };
            match ctx.exchange.fetch_file(request).await {
                Ok(tid) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::TransferList { project_id } => {
            let transfers = ctx.exchange.list_transfers(project_id.as_deref()).await;
            let infos: Vec<TransferInfo> = transfers
                .into_iter()
                .map(|t| TransferInfo {
                    transfer_id: t.transfer_id.0,
                    file_name: t.file_name,
                    file_size: t.file_size,
                    bytes_transferred: t.bytes_transferred,
                    speed_bps: t.speed_bps,
                    direction: match t.direction {
                        TransferDirection::Send => "upload".to_string(),
                        TransferDirection::Receive => "download".to_string(),
                    },
                    peer_id: t.peer_id.to_string(),
                    state: format!("{:?}", t.state),
                })
                .collect();
            IpcResponse::TransferList(infos)
        }

        IpcCommand::TransferCancel { transfer_id } => {
            match ctx.exchange.cancel_transfer(&TransferId(transfer_id)).await {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::PublishUpdate {
            project_id,
            file_paths,
        } => {
            let notifications: Vec<PublishNotification> = file_paths
                .iter()
                .map(|path| {
                    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    let crc = crc32fast::hash(path.to_string_lossy().as_bytes());
                    PublishNotification {
                        project_id: project_id.clone(),
                        file_id: FileId::new(path.to_string_lossy().to_string()),
                        file_path: path.clone(),
                        file_size,
                        crc,
                        version: 1,
                    }
                })
                .collect();
            match ctx.exchange.publish_updates(notifications).await {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: e.to_string(),
                },
            }
        }

        // ── モニタリング ──
        IpcCommand::NetworkStatus => {
            let peers = ctx.presence.list_nodes(None).await;
            let connected_peers: Vec<_> = peers
                .iter()
                .filter(|p| p.state == PeerState::Connected || p.state == PeerState::Idle)
                .collect();

            let total_bw: u64 = connected_peers.iter().map(|p| p.bandwidth_bps).sum();
            let avg_latency = if connected_peers.is_empty() {
                0
            } else {
                let total_rtt: u32 = connected_peers.iter().filter_map(|p| p.rtt_ms).sum();
                let count = connected_peers
                    .iter()
                    .filter(|p| p.rtt_ms.is_some())
                    .count() as u32;
                if count > 0 {
                    total_rtt / count
                } else {
                    0
                }
            };

            let primary_route = connected_peers
                .first()
                .and_then(|p| p.endpoints.first())
                .map(|r| format!("{:?}", r.kind()))
                .unwrap_or_else(|| "none".to_string());

            IpcResponse::NetworkStatus(NetworkStatusInfo {
                primary_route,
                total_bandwidth_bps: total_bw,
                used_bandwidth_bps: 0,
                active_connections: connected_peers.len() as u16,
                max_connections: 0,
                avg_latency_ms: avg_latency,
            })
        }

        IpcCommand::Subscribe { .. } => {
            // イベント購読: subscription_id を返し、別途イベントプッシュで通知
            IpcResponse::Subscribed {
                subscription_id: uuid::Uuid::new_v4().to_string(),
            }
        }

        IpcCommand::Unsubscribe { .. } => IpcResponse::Ok,
    }
}
