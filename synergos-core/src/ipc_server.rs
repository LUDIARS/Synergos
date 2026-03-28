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
use synergos_ipc::event::IpcEvent;
use synergos_ipc::response::{DaemonStatus, IpcResponse, NetworkStatusInfo};
use synergos_ipc::transport::{IpcError, IpcTransport};

use crate::event_bus::SharedEventBus;
use crate::project::ProjectManager;

/// IPC サーバー
pub struct IpcServer {
    event_bus: SharedEventBus,
    project_manager: Arc<ProjectManager>,
    shutdown_tx: broadcast::Sender<()>,
}

impl IpcServer {
    pub fn new(
        event_bus: SharedEventBus,
        project_manager: Arc<ProjectManager>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            event_bus,
            project_manager,
            shutdown_tx,
        }
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

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            let event_bus = self.event_bus.clone();
                            let project_manager = self.project_manager.clone();
                            let shutdown_tx = self.shutdown_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, event_bus, project_manager, shutdown_tx).await {
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
        // Named Pipe サーバーは将来実装
        tracing::warn!("Windows Named Pipe server not yet implemented");
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let _ = shutdown_rx.recv().await;
        Ok(())
    }
}

/// クライアント接続のハンドリング
#[cfg(unix)]
async fn handle_client(
    stream: tokio::net::UnixStream,
    event_bus: SharedEventBus,
    project_manager: Arc<ProjectManager>,
    shutdown_tx: broadcast::Sender<()>,
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

        let response = dispatch_command(
            command,
            &event_bus,
            &project_manager,
            &shutdown_tx,
        )
        .await;

        IpcTransport::write_message(&mut writer, &response).await?;
    }

    Ok(())
}

/// コマンドをディスパッチしてレスポンスを生成
async fn dispatch_command(
    command: IpcCommand,
    _event_bus: &SharedEventBus,
    project_manager: &ProjectManager,
    shutdown_tx: &broadcast::Sender<()>,
) -> IpcResponse {
    match command {
        IpcCommand::Ping => IpcResponse::Pong,

        IpcCommand::Shutdown => {
            tracing::info!("Shutdown requested via IPC");
            let _ = shutdown_tx.send(());
            IpcResponse::Ok
        }

        IpcCommand::Status => {
            let status = DaemonStatus {
                pid: std::process::id(),
                started_at: 0, // TODO: 起動時刻を記録
                project_count: project_manager.count(),
                active_connections: 0, // TODO: 実装
                active_transfers: 0,   // TODO: 実装
            };
            IpcResponse::Status(status)
        }

        IpcCommand::ProjectOpen {
            project_id,
            root_path,
        } => {
            project_manager.open(project_id, root_path).await;
            IpcResponse::Ok
        }

        IpcCommand::ProjectClose { project_id } => {
            project_manager.close(&project_id).await;
            IpcResponse::Ok
        }

        IpcCommand::ProjectList => {
            let projects = project_manager.list();
            IpcResponse::ProjectList(projects)
        }

        IpcCommand::PeerList { .. } => {
            // TODO: PresenceService 経由で実装
            IpcResponse::PeerList(vec![])
        }

        IpcCommand::PeerConnect { .. } => {
            // TODO: Conduit 経由で実装
            IpcResponse::Ok
        }

        IpcCommand::PeerDisconnect { .. } => {
            // TODO: Conduit 経由で実装
            IpcResponse::Ok
        }

        IpcCommand::TransferRequest { .. } => {
            // TODO: Exchange 経由で実装
            IpcResponse::Ok
        }

        IpcCommand::TransferList { .. } => {
            // TODO: Exchange 経由で実装
            IpcResponse::TransferList(vec![])
        }

        IpcCommand::TransferCancel { .. } => {
            // TODO: Exchange 経由で実装
            IpcResponse::Ok
        }

        IpcCommand::PublishUpdate { .. } => {
            // TODO: Exchange + Chain 経由で実装
            IpcResponse::Ok
        }

        IpcCommand::NetworkStatus => {
            // TODO: NetworkMonitor 経由で実装
            IpcResponse::NetworkStatus(NetworkStatusInfo {
                primary_route: "none".to_string(),
                total_bandwidth_bps: 0,
                used_bandwidth_bps: 0,
                active_connections: 0,
                max_connections: 0,
                avg_latency_ms: 0,
            })
        }

        IpcCommand::Subscribe { .. } => {
            // TODO: イベント購読の実装
            IpcResponse::Subscribed {
                subscription_id: uuid::Uuid::new_v4().to_string(),
            }
        }

        IpcCommand::Unsubscribe { .. } => {
            // TODO: 購読解除の実装
            IpcResponse::Ok
        }
    }
}
