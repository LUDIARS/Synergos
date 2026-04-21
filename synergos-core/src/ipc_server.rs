//! IPC サーバー
//!
//! クロスプラットフォーム IPC サーバー。
//! - Linux / macOS: Unix Domain Socket
//! - Windows: Named Pipe（将来実装）
//!
//! クライアント（GUI / CLI / Ars Plugin）からのコマンドを受け付け、
//! EventBus と連携してレスポンス・イベントプッシュを行う。

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

use synergos_ipc::command::IpcCommand;
use synergos_ipc::event::{EventCategory, EventFilter, IpcEvent};
use synergos_ipc::response::{
    DaemonStatus, IpcResponse, NetworkStatusInfo, PeerInfo, TransferInfo,
};
use synergos_ipc::transport::{IpcError, IpcTransport, ServerMessage};
use synergos_net::types::{FileId, PeerId, TransferId};

use crate::conflict::ConflictManager;
use crate::event_bus::{
    ConflictDetectedEvent, NetworkStatusEvent, PeerConnectedEvent, PeerDisconnectedEvent,
    SharedEventBus, TransferCompletedEvent, TransferProgressEvent,
};
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

        // ソケットを `chmod 0600`: uid_check と併せて多層防御。
        // デーモンを起動した UID 以外の書込みを OS レベルで遮る。
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = tokio::fs::set_permissions(&path, perms).await {
                tracing::warn!("failed to chmod 0600 {}: {}", path.display(), e);
            }
        }

        tracing::info!("IPC server listening on {}", path.display());

        let mut shutdown_rx = self.ctx.shutdown_tx.subscribe();
        // accept エラー時の指数バックオフ上限 (fd 枯渇時のタイトループ防止)
        let mut backoff_ms = 0u64;

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            backoff_ms = 0;

                            // peer uid 検証: 起動ユーザ以外を拒絶。
                            if let Err(reason) = verify_peer_uid(&stream) {
                                tracing::warn!("rejecting client: {reason}");
                                drop(stream);
                                continue;
                            }

                            let ctx = self.ctx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, ctx).await {
                                    tracing::warn!("Client connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                            // 指数バックオフ (最大 1s)
                            backoff_ms = (backoff_ms * 2).max(10).min(1000);
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
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

/// 接続中クライアントの writer。Response / Event を多重化するため Mutex でガードする。
#[cfg(unix)]
type SharedWriter = Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>;

/// 接続元 UID が自プロセス UID と一致するか確認する。
///
/// 非 Unix や UCred 非対応プラットフォームでは許容する (Windows は別経路)。
#[cfg(unix)]
fn verify_peer_uid(stream: &tokio::net::UnixStream) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `stream` は生きている UnixStream。借用期間内でのみ fd を触る。
    // std の UnixStream::peer_cred を借りるために unsafe FromRawFd + ManuallyDrop パターン。
    let fd = stream.as_raw_fd();
    let std_stream = unsafe {
        use std::os::unix::io::FromRawFd;
        std::mem::ManuallyDrop::new(std::os::unix::net::UnixStream::from_raw_fd(fd))
    };
    let cred = match std_stream.peer_cred() {
        Ok(c) => c,
        Err(e) => {
            // peer_cred が取れない OS (macOS 等旧バージョン) は allow (ベストエフォート)
            tracing::debug!("peer_cred unavailable, skipping uid check: {e}");
            return Ok(());
        }
    };
    let peer_uid = cred.uid();
    // 自プロセス UID
    // SAFETY: libc call with no side effects; always succeeds.
    let self_uid = unsafe { libc::geteuid() };
    if peer_uid != self_uid {
        return Err(format!(
            "peer uid {peer_uid} does not match daemon uid {self_uid}"
        ));
    }
    Ok(())
}

/// クライアント接続のハンドリング
#[cfg(unix)]
async fn handle_client(
    stream: tokio::net::UnixStream,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError> {
    let (mut reader, writer) = stream.into_split();
    let writer: SharedWriter = Arc::new(Mutex::new(writer));

    // Subscribe 起動時にここへタスクハンドルを保持。Unsubscribe / 切断時に abort。
    let mut event_relay: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let command: IpcCommand = match IpcTransport::read_message(&mut reader).await {
            Ok(cmd) => cmd,
            Err(IpcError::ConnectionClosed) => {
                tracing::debug!("Client disconnected");
                break;
            }
            Err(e) => {
                if let Some(h) = event_relay.take() {
                    h.abort();
                }
                return Err(e);
            }
        };

        tracing::debug!("Received command: {:?}", command);

        match command {
            IpcCommand::Subscribe { filter } => {
                // 既存リレーがあれば停止してから再起動
                if let Some(h) = event_relay.take() {
                    h.abort();
                }
                let subscription_id = uuid::Uuid::new_v4().to_string();
                let resp = IpcResponse::Subscribed {
                    subscription_id: subscription_id.clone(),
                };
                send_server_message(&writer, ServerMessage::Response(resp)).await?;

                let writer_clone = writer.clone();
                let ctx_clone = ctx.clone();
                event_relay = Some(tokio::spawn(async move {
                    relay_events(ctx_clone, writer_clone, filter).await;
                }));
            }
            IpcCommand::Unsubscribe { .. } => {
                if let Some(h) = event_relay.take() {
                    h.abort();
                }
                send_server_message(&writer, ServerMessage::Response(IpcResponse::Ok)).await?;
            }
            other => {
                let response = dispatch_command(other, &ctx).await;
                send_server_message(&writer, ServerMessage::Response(response)).await?;
            }
        }
    }

    if let Some(h) = event_relay.take() {
        h.abort();
    }
    Ok(())
}

#[cfg(unix)]
async fn send_server_message(writer: &SharedWriter, msg: ServerMessage) -> Result<(), IpcError> {
    let mut guard = writer.lock().await;
    IpcTransport::write_message(&mut *guard, &msg).await
}

/// EventBus → クライアントへ IpcEvent を中継する per-client タスク。
/// `filter` に合致しないイベントはスキップ。
#[cfg(unix)]
async fn relay_events(ctx: Arc<ServiceContext>, writer: SharedWriter, filter: EventFilter) {
    let mut rx_peer_connected = ctx.event_bus.subscribe::<PeerConnectedEvent>();
    let mut rx_peer_disconnected = ctx.event_bus.subscribe::<PeerDisconnectedEvent>();
    let mut rx_transfer_progress = ctx.event_bus.subscribe::<TransferProgressEvent>();
    let mut rx_transfer_completed = ctx.event_bus.subscribe::<TransferCompletedEvent>();
    let mut rx_conflict = ctx.event_bus.subscribe::<ConflictDetectedEvent>();
    let mut rx_network = ctx.event_bus.subscribe::<NetworkStatusEvent>();

    loop {
        let event: Option<IpcEvent> = tokio::select! {
            r = rx_peer_connected.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Peer, Some(&ev.project_id),
                    IpcEvent::PeerConnected {
                        project_id: ev.project_id,
                        peer_id: ev.peer_id,
                        display_name: ev.display_name,
                        route: ev.route,
                        rtt_ms: ev.rtt_ms,
                    },
                ),
                Err(_) => continue,
            },
            r = rx_peer_disconnected.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Peer, Some(&ev.project_id),
                    IpcEvent::PeerDisconnected {
                        project_id: ev.project_id,
                        peer_id: ev.peer_id,
                        reason: ev.reason,
                    },
                ),
                Err(_) => continue,
            },
            r = rx_transfer_progress.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Transfer, None,
                    IpcEvent::TransferProgress {
                        transfer_id: ev.transfer_id,
                        peer_id: String::new(),
                        file_name: ev.file_name,
                        bytes_transferred: ev.bytes_transferred,
                        total_bytes: ev.total_bytes,
                        speed_bps: ev.speed_bps,
                    },
                ),
                Err(_) => continue,
            },
            r = rx_transfer_completed.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Transfer, None,
                    IpcEvent::TransferCompleted {
                        transfer_id: ev.transfer_id,
                        peer_id: String::new(),
                        file_name: ev.file_name,
                        file_path: ev.file_path,
                    },
                ),
                Err(_) => continue,
            },
            r = rx_conflict.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Conflict, Some(&ev.project_id),
                    IpcEvent::ConflictDetected {
                        project_id: ev.project_id,
                        file_id: ev.file_id,
                        file_path: ev.file_path,
                        involved_peers: ev.involved_peers,
                    },
                ),
                Err(_) => continue,
            },
            r = rx_network.recv() => match r {
                Ok(ev) => filter_event(
                    &filter, EventCategory::Network, None,
                    IpcEvent::NetworkStatusUpdated {
                        active_connections: ev.active_connections,
                        total_bandwidth_bps: ev.total_bandwidth_bps,
                        used_bandwidth_bps: ev.used_bandwidth_bps,
                        avg_latency_ms: ev.avg_latency_ms,
                    },
                ),
                Err(_) => continue,
            },
        };

        let Some(ipc_event) = event else { continue };

        if let Err(e) = send_server_message(&writer, ServerMessage::Event(ipc_event)).await {
            tracing::debug!("event relay write failed (client likely gone): {e}");
            break;
        }
    }
}

/// フィルタに一致しないイベントを落とす。一致すれば `Some(event)` を返す。
fn filter_event(
    filter: &EventFilter,
    category: EventCategory,
    project_id: Option<&str>,
    event: IpcEvent,
) -> Option<IpcEvent> {
    match filter {
        EventFilter::All => Some(event),
        EventFilter::Project(target) => match project_id {
            Some(p) if p == target => Some(event),
            Some(_) => None,
            // プロジェクト非依存イベント (Network / Transfer 等) は透過
            None => Some(event),
        },
        EventFilter::Category(target) => {
            if std::mem::discriminant(target) == std::mem::discriminant(&category) {
                Some(event)
            } else {
                None
            }
        }
    }
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
                .filter(|t| {
                    t.state == TransferState::Running || t.state == TransferState::Queued
                })
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
                Ok(_tid) => IpcResponse::Ok,
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
            match ctx
                .exchange
                .cancel_transfer(&TransferId(transfer_id))
                .await
            {
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
            // プロジェクトルートを引き当て、与えられたパスがその配下に収まるか検証。
            // SSRF 様の「任意絶対パスに `metadata` できる」問題 (S11) の対策。
            let project_root = match ctx.project_manager.project_root(&project_id) {
                Some(p) => p,
                None => {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!("unknown project: {project_id}"),
                    };
                }
            };
            let project_root = match tokio::fs::canonicalize(&project_root).await {
                Ok(p) => p,
                Err(e) => {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!("project root canonicalize failed: {e}"),
                    };
                }
            };

            let mut notifications: Vec<PublishNotification> = Vec::with_capacity(file_paths.len());
            for path in &file_paths {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    project_root.join(path)
                };
                let canonical = match tokio::fs::canonicalize(&absolute).await {
                    Ok(p) => p,
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("file not found or unreadable: {:?}: {e}", absolute),
                        };
                    }
                };
                if !canonical.starts_with(&project_root) {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!(
                            "file outside project root: {:?} (root: {:?})",
                            canonical, project_root
                        ),
                    };
                }

                let metadata = match tokio::fs::metadata(&canonical).await {
                    Ok(m) => m,
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("metadata failed {:?}: {e}", canonical),
                        };
                    }
                };
                let file_size = metadata.len();
                let bytes = match tokio::fs::read(&canonical).await {
                    Ok(b) => b,
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("read failed {:?}: {e}", canonical),
                        };
                    }
                };
                let crc = crc32fast::hash(&bytes);

                let rel = canonical
                    .strip_prefix(&project_root)
                    .map(|r| r.to_path_buf())
                    .unwrap_or(canonical.clone());

                notifications.push(PublishNotification {
                    project_id: project_id.clone(),
                    file_id: FileId::new(rel.to_string_lossy().to_string()),
                    file_path: canonical,
                    file_size,
                    crc,
                    version: 1,
                });
            }
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
                let total_rtt: u32 = connected_peers
                    .iter()
                    .filter_map(|p| p.rtt_ms)
                    .sum();
                let count = connected_peers
                    .iter()
                    .filter(|p| p.rtt_ms.is_some())
                    .count() as u32;
                if count > 0 { total_rtt / count } else { 0 }
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

        // Subscribe / Unsubscribe は handle_client 側で per-client タスクとして
        // 処理するため、ここに届くことはない。保険として Ok を返す。
        IpcCommand::Subscribe { .. } => IpcResponse::Ok,
        IpcCommand::Unsubscribe { .. } => IpcResponse::Ok,
    }
}
