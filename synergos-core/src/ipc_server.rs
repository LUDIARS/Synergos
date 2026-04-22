//! IPC サーバー
//!
//! クロスプラットフォーム IPC サーバー。
//! - Linux / macOS: Unix Domain Socket
//! - Windows: Named Pipe（将来実装）
//!
//! クライアント（GUI / CLI / Ars Plugin）からのコマンドを受け付け、
//! EventBus と連携してレスポンス・イベントプッシュを行う。

use std::sync::Arc;
#[cfg(unix)]
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
                            backoff_ms = (backoff_ms * 2).clamp(10, 1000);
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

    /// Windows 用の IPC サーバー（Named Pipe）。
    ///
    /// `tokio::net::windows::named_pipe::NamedPipeServer` を 1 インスタンス
    /// ずつ create → wait_for_client → 切り離して次インスタンスを create、
    /// という標準パターン。`FIRST_PIPE_INSTANCE` でパイプ名を占有して
    /// なりすましインスタンスの作成を拒む。
    #[cfg(windows)]
    pub async fn run(&self) -> Result<(), IpcError> {
        use std::time::Duration;
        use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

        let path = synergos_ipc::transport::socket_path();
        let pipe_name = path.to_string_lossy().to_string();

        let mut shutdown_rx = self.ctx.shutdown_tx.subscribe();

        // 初回インスタンスだけ first_pipe_instance(true) で作成してパイプ名を予約する。
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .pipe_mode(PipeMode::Byte)
            .create(&pipe_name)
            .map_err(IpcError::Io)?;
        tracing::info!("IPC named pipe listening on {pipe_name}");

        // accept エラー時の指数バックオフ上限 (fd 枯渇時のタイトループ防止に相当)
        let mut backoff_ms = 0u64;

        loop {
            let connect_result = tokio::select! {
                r = server.connect() => r,
                _ = shutdown_rx.recv() => {
                    tracing::info!("IPC server shutting down");
                    return Ok(());
                }
            };

            match connect_result {
                Ok(()) => {
                    backoff_ms = 0;
                    let next = ServerOptions::new()
                        .pipe_mode(PipeMode::Byte)
                        .create(&pipe_name);
                    let next = match next {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::error!("failed to create next pipe instance: {e}");
                            backoff_ms = (backoff_ms * 2).clamp(10, 1000);
                            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            continue;
                        }
                    };
                    // `server.connect()` の借用は既に resolved。mem::replace で
                    // 現インスタンスを取り出し、次インスタンスを server にセット。
                    let connected = std::mem::replace(&mut server, next);
                    let ctx = self.ctx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client_windows(connected, ctx).await {
                            tracing::warn!("Client connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Named pipe accept error: {e}");
                    backoff_ms = (backoff_ms * 2).clamp(10, 1000);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }
}

/// 接続中クライアントの writer。Response / Event を多重化するため Mutex でガードする。
#[cfg(unix)]
type SharedWriter = Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>;

/// 接続元 UID が自プロセス UID と一致するか確認する。
///
/// `std::os::unix::net::UCred::uid()` は現在 nightly 限定の unstable API
/// なので libc 直接呼び出しで実装する:
/// - Linux: `getsockopt(SO_PEERCRED)` → `struct ucred { pid, uid, gid }`
/// - macOS / iOS / FreeBSD: `getpeereid(fd, &uid, &gid)`
/// - それ以外の Unix は uid 取得を諦めて許容 (ベストエフォート)。
#[cfg(unix)]
fn verify_peer_uid(stream: &tokio::net::UnixStream) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();

    let peer_uid = match peer_uid_of_fd(fd) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!("peer_cred unavailable, skipping uid check: {e}");
            return Ok(());
        }
    };
    // SAFETY: libc::geteuid is side-effect-free and always succeeds.
    let self_uid = unsafe { libc::geteuid() };
    if peer_uid != self_uid {
        return Err(format!(
            "peer uid {peer_uid} does not match daemon uid {self_uid}"
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn peer_uid_of_fd(fd: std::os::unix::io::RawFd) -> std::io::Result<libc::uid_t> {
    // SAFETY: libc::ucred は POD。getsockopt が成功した場合のみ書き込まれる。
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ret = libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        );
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(cred.uid)
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
))]
fn peer_uid_of_fd(fd: std::os::unix::io::RawFd) -> std::io::Result<libc::uid_t> {
    // SAFETY: getpeereid fills uid/gid when the call succeeds.
    unsafe {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let ret = libc::getpeereid(fd, &mut uid, &mut gid);
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(uid)
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))
))]
fn peer_uid_of_fd(_fd: std::os::unix::io::RawFd) -> std::io::Result<libc::uid_t> {
    // illumos / solaris / hermit など未対応 Unix はベストエフォートで自プロセス UID を返す。
    // SAFETY: geteuid is side-effect-free.
    unsafe { Ok(libc::geteuid()) }
}

/// Windows Named Pipe 接続のハンドリング。Unix 版と共通の dispatch/relay
/// ロジックをトレイト越しに呼び出すラッパ。
#[cfg(windows)]
async fn handle_client_windows(
    pipe: tokio::net::windows::named_pipe::NamedPipeServer,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError> {
    let (reader, writer) = tokio::io::split(pipe);
    let writer: Arc<Mutex<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeServer>>> =
        Arc::new(Mutex::new(writer));
    handle_client_generic(reader, writer, ctx).await
}

/// クライアント接続のハンドリング
#[cfg(unix)]
async fn handle_client(
    stream: tokio::net::UnixStream,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError> {
    let (reader, writer) = stream.into_split();
    let writer: SharedWriter = Arc::new(Mutex::new(writer));
    handle_client_generic(reader, writer, ctx).await
}

/// Unix / Windows 共通のクライアント処理。
pub async fn handle_client_generic<R, W>(
    mut reader: R,
    writer: Arc<Mutex<W>>,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
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

        // dispatcher 前に共通の入力バリデーションを通す。空文字 / 過長 ID 等を弾く。
        if let Err(reason) = command.validate() {
            send_server_message(
                &writer,
                ServerMessage::Response(IpcResponse::Error {
                    code: 400,
                    message: format!("invalid command: {reason}"),
                }),
            )
            .await?;
            continue;
        }

        match command {
            IpcCommand::Subscribe { events } => {
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
                // 複数フィルタが来た場合はいずれかに match すれば配信する OR 合成。
                // None (Vec が空) の場合は All とみなす。
                let filters = if events.is_empty() {
                    vec![EventFilter::All]
                } else {
                    events
                };
                event_relay = Some(tokio::spawn(async move {
                    relay_events(ctx_clone, writer_clone, filters).await;
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

async fn send_server_message<W>(writer: &Arc<Mutex<W>>, msg: ServerMessage) -> Result<(), IpcError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut guard = writer.lock().await;
    IpcTransport::write_message(&mut *guard, &msg).await
}

/// EventBus → クライアントへ IpcEvent を中継する per-client タスク。
/// `filter` に合致しないイベントはスキップ。
async fn relay_events<W>(ctx: Arc<ServiceContext>, writer: Arc<Mutex<W>>, filters: Vec<EventFilter>)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // `filter_event` に渡す際に参照渡しにしたいので slice で借用する形に。
    let filters_ref = filters.as_slice();
    // 可変参照渡しを避けるため各マッチで filters_ref をそのまま使う。
    // NB: filter_event は下で Vec<EventFilter> を受け取るよう定義を併せて変更。
    let mut rx_peer_connected = ctx.event_bus.subscribe::<PeerConnectedEvent>();
    let mut rx_peer_disconnected = ctx.event_bus.subscribe::<PeerDisconnectedEvent>();
    let mut rx_transfer_progress = ctx.event_bus.subscribe::<TransferProgressEvent>();
    let mut rx_transfer_completed = ctx.event_bus.subscribe::<TransferCompletedEvent>();
    let mut rx_conflict = ctx.event_bus.subscribe::<ConflictDetectedEvent>();
    let mut rx_network = ctx.event_bus.subscribe::<NetworkStatusEvent>();

    loop {
        let event: Option<IpcEvent> = tokio::select! {
            r = rx_peer_connected.recv() => match r {
                Ok(ev) => {
                    let pid = ev.project_id.clone();
                    filter_events(
                        filters_ref, EventCategory::Peer, Some(&pid),
                        IpcEvent::PeerConnected {
                            project_id: ev.project_id,
                            peer_id: ev.peer_id,
                            display_name: ev.display_name,
                            route: ev.route,
                            rtt_ms: ev.rtt_ms,
                        },
                    )
                }
                Err(_) => continue,
            },
            r = rx_peer_disconnected.recv() => match r {
                Ok(ev) => {
                    let pid = ev.project_id.clone();
                    filter_events(
                        filters_ref, EventCategory::Peer, Some(&pid),
                        IpcEvent::PeerDisconnected {
                            project_id: ev.project_id,
                            peer_id: ev.peer_id,
                            reason: ev.reason,
                        },
                    )
                }
                Err(_) => continue,
            },
            r = rx_transfer_progress.recv() => match r {
                Ok(ev) => filter_events(
                    filters_ref, EventCategory::Transfer, None,
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
                Ok(ev) => filter_events(
                    filters_ref, EventCategory::Transfer, None,
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
                Ok(ev) => {
                    let pid = ev.project_id.clone();
                    filter_events(
                        filters_ref, EventCategory::Conflict, Some(&pid),
                        IpcEvent::ConflictDetected {
                            project_id: ev.project_id,
                            file_id: ev.file_id,
                            file_path: ev.file_path,
                            involved_peers: ev.involved_peers,
                        },
                    )
                }
                Err(_) => continue,
            },
            r = rx_network.recv() => match r {
                Ok(ev) => filter_events(
                    filters_ref, EventCategory::Network, None,
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

/// 複数 filter の OR 合成。いずれかが受け入れるなら `Some(event)`。
fn filter_events(
    filters: &[EventFilter],
    category: EventCategory,
    project_id: Option<&str>,
    event: IpcEvent,
) -> Option<IpcEvent> {
    for f in filters {
        if filter_event_one(f, &category, project_id).is_some() {
            return Some(event);
        }
    }
    None
}

/// 1 本の filter で判定。`Some(())` なら受理。
fn filter_event_one(
    filter: &EventFilter,
    category: &EventCategory,
    project_id: Option<&str>,
) -> Option<()> {
    match filter {
        EventFilter::All => Some(()),
        EventFilter::Project(target) => match project_id {
            Some(p) if p == target => Some(()),
            Some(_) => None,
            None => Some(()),
        },
        EventFilter::Category(target) => {
            if std::mem::discriminant(target) == std::mem::discriminant(category) {
                Some(())
            } else {
                None
            }
        }
    }
}

/// コマンドをディスパッチしてレスポンスを生成
pub async fn dispatch_command(command: IpcCommand, ctx: &ServiceContext) -> IpcResponse {
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
                // IPC 経由の要求は「任意の最新」として 0 を渡す。
                // 呼出側が具体バージョンを指定したい場合は IpcCommand を拡張する。
                version: 0,
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

            use synergos_net::types::redact_path;
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
                            message: format!(
                                "file not found or unreadable: {}: {e}",
                                redact_path(&project_root, &absolute)
                            ),
                        };
                    }
                };
                if !canonical.starts_with(&project_root) {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!(
                            "file outside project root: {}",
                            redact_path(&project_root, &canonical)
                        ),
                    };
                }

                let metadata = match tokio::fs::metadata(&canonical).await {
                    Ok(m) => m,
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!(
                                "metadata failed {}: {e}",
                                redact_path(&project_root, &canonical)
                            ),
                        };
                    }
                };
                let file_size = metadata.len();
                let bytes = match tokio::fs::read(&canonical).await {
                    Ok(b) => b,
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!(
                                "read failed {}: {e}",
                                redact_path(&project_root, &canonical)
                            ),
                        };
                    }
                };
                let crc = crc32fast::hash(&bytes);

                let rel = canonical
                    .strip_prefix(&project_root)
                    .map(|r| r.to_path_buf())
                    .unwrap_or(canonical.clone());

                let file_id = FileId::new(rel.to_string_lossy().to_string());
                // ProjectManager に file_id → rel 相対パスを登録して、
                // 受信側の out_path_resolver が確実に解決できるようにする。
                ctx.project_manager
                    .register_file(&project_id, file_id.clone(), rel.clone());

                notifications.push(PublishNotification {
                    project_id: project_id.clone(),
                    file_id,
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
                let total_rtt: u32 = connected_peers.iter().filter_map(|p| p.rtt_ms).sum();
                let count = connected_peers
                    .iter()
                    .filter(|p| p.rtt_ms.is_some())
                    .count() as u32;
                total_rtt.checked_div(count).unwrap_or(0)
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

        IpcCommand::ConflictList { project_id } => {
            let items = ctx
                .conflict_manager
                .list_conflicts(project_id.as_deref())
                .into_iter()
                .map(|c| synergos_ipc::response::ConflictInfoDto {
                    file_id: c.file_id.to_string(),
                    file_path: c.file_path,
                    project_id: c.project_id,
                    local_version: c.local_version,
                    local_author: c.local_author.to_string(),
                    remote_version: c.remote_version,
                    remote_author: c.remote_author.to_string(),
                    detected_at: c.detected_at,
                    state: match c.state {
                        crate::conflict::ConflictState::Active => "active".into(),
                        crate::conflict::ConflictState::Resolved { resolution } => {
                            format!("resolved:{:?}", resolution)
                        }
                    },
                })
                .collect();
            IpcResponse::ConflictList(items)
        }
        IpcCommand::ConflictResolve {
            file_id,
            resolution,
        } => {
            let res = match resolution.as_str() {
                "keep_local" => crate::conflict::ConflictResolution::KeepLocal,
                "accept_remote" => crate::conflict::ConflictResolution::AcceptRemote,
                "manual_merge" => crate::conflict::ConflictResolution::ManualMerge,
                other => {
                    return IpcResponse::Error {
                        code: 4,
                        message: format!("invalid resolution: {other}"),
                    }
                }
            };
            match ctx
                .conflict_manager
                .resolve_conflict(&synergos_net::types::FileId::new(file_id), res)
            {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 4,
                    message: e.to_string(),
                },
            }
        }
        IpcCommand::ConfigUpdate { .. } => {
            // 現状はホット差替えなしで、将来の完全対応までは受理のみ。
            // デーモンを再起動するかどうかは呼び出し側が決定する。
            tracing::info!("ConfigUpdate received; no hot-swap implemented yet");
            IpcResponse::Ok
        }
    }
}
