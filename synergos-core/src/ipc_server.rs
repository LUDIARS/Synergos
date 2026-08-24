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
    /// 設定スナップショット (NetworkStatus の max_connections 算出等で使う)。
    /// ホット更新は今のところ未対応なので起動時の値を保持する。
    pub net_config: Option<Arc<synergos_net::config::NetConfig>>,
    /// 開いているプロジェクトの CatalogManager (project_id → CatalogManager)。
    /// ProjectOpen 時に生成、Close で remove。gossip CatalogUpdate 受信時に
    /// ローカル root_crc と比較して差分を検出する (#26)。
    pub catalogs: Arc<dashmap::DashMap<String, Arc<synergos_net::catalog::CatalogManager>>>,
    /// Bitswap 用 content-addressed store。`publish_updates` で作った
    /// RootCatalog スナップショットや DAG blocks はここに入り、相手ピアからの
    /// BSW1 リクエストで引き出される (#25 + #26)。
    pub content_store: Arc<synergos_net::content::MemoryContentStore>,
    /// QUIC マネージャ (peer add-url / 直接接続用)。Daemon::new で bind 済み。
    pub quic: Arc<synergos_net::quic::QuicManager>,
    /// ノード identity (招待トークン署名用)。テスト等では None 可
    /// (その場合は従来型トークンにフォールバックする)。
    pub identity: Option<Arc<synergos_net::identity::Identity>>,
    /// 履歴ノードの保管庫 (無効なノードでも実体はあり、`enabled()` が false)。
    pub history: Arc<crate::history::HistoryStore>,
    /// publish / 受信時フックランナー (docs/hooks.md)。
    pub hooks: Arc<crate::hooks::HookRunner>,
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
///
/// 接続直後に **caller SID と daemon プロセスの owner SID を比較** して、
/// 同一ユーザでなければ即切断する。Named Pipe は `first_pipe_instance` で
/// 名前占有はしているが、ACL を細かく設定していないため Windows 上の任意
/// ユーザが接続できてしまう穴があった (CWE-269)。
#[cfg(windows)]
async fn handle_client_windows(
    pipe: tokio::net::windows::named_pipe::NamedPipeServer,
    ctx: Arc<ServiceContext>,
) -> Result<(), IpcError> {
    if let Err(reason) = verify_windows_caller(&pipe) {
        tracing::warn!("rejecting client (Windows): {reason}");
        // pipe を drop して接続を切る
        return Ok(());
    }
    let (reader, writer) = tokio::io::split(pipe);
    let writer: Arc<Mutex<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeServer>>> =
        Arc::new(Mutex::new(writer));
    handle_client_generic(reader, writer, ctx).await
}

/// 接続中の Named Pipe に対し、caller プロセスの SID が現在の daemon プロセスの
/// owner SID と一致するか確認する。
///
/// 手順:
///   1. `GetNamedPipeClientProcessId` で caller PID を取得
///   2. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` で caller プロセスをオープン
///   3. `OpenProcessToken(TOKEN_QUERY)` でアクセス token を取得
///   4. `GetTokenInformation(TokenUser)` で caller の SID を取得
///   5. 自プロセスでも 2-4 を行い `EqualSid` で比較
#[cfg(windows)]
fn verify_windows_caller(
    pipe: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::EqualSid;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;

    let pipe_handle = pipe.as_raw_handle() as HANDLE;
    if pipe_handle.is_null() || pipe_handle == INVALID_HANDLE_VALUE {
        return Err("invalid pipe handle".into());
    }

    let mut caller_pid: u32 = 0;
    // SAFETY: pipe_handle は今回のスコープで生きている valid な handle。
    let ok = unsafe { GetNamedPipeClientProcessId(pipe_handle, &mut caller_pid) };
    if ok == 0 {
        return Err(format!(
            "GetNamedPipeClientProcessId failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    let caller_sid = read_process_sid(caller_pid).map_err(|e| format!("caller sid: {e}"))?;
    let self_sid = read_self_sid().map_err(|e| format!("self sid: {e}"))?;

    // SAFETY: 双方の TOKEN_USER バッファは valid。
    let equal = unsafe { EqualSid(caller_sid.token_user.User.Sid, self_sid.token_user.User.Sid) };

    if equal == 0 {
        return Err(format!("caller SID mismatch (pid {caller_pid})"));
    }
    Ok(())
}

/// TOKEN_USER のバッキングストアを保持する RAII。`token_user.User.Sid` は
/// `_backing` の中を指している。
#[cfg(windows)]
struct TokenUserBuf {
    token_user: windows_sys::Win32::Security::TOKEN_USER,
    _backing: Vec<u8>,
}

#[cfg(windows)]
fn read_process_sid(pid: u32) -> std::io::Result<TokenUserBuf> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: PID は信頼できる入力 (Named Pipe 経由で取得済) として扱う。
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let res = read_token_user(process);
    // SAFETY: process はこのスコープで取得した valid handle。
    unsafe { CloseHandle(process) };
    res
}

#[cfg(windows)]
fn read_self_sid() -> std::io::Result<TokenUserBuf> {
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: GetCurrentProcess は擬似ハンドルで close 不要。
    let h = unsafe { GetCurrentProcess() };
    read_token_user(h)
}

/// `OpenProcessToken` + `GetTokenInformation(TokenUser)` の rust ラッパ。
#[cfg(windows)]
fn read_token_user(
    process_handle: windows_sys::Win32::Foundation::HANDLE,
) -> std::io::Result<TokenUserBuf> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::OpenProcessToken;

    let mut token_handle: HANDLE = std::ptr::null_mut();
    // SAFETY: process_handle は呼び出し側保証で valid。
    if unsafe { OpenProcessToken(process_handle, TOKEN_QUERY, &mut token_handle) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // 1 回目: 必要サイズを取得
    let mut required: u32 = 0;
    // SAFETY: 1st call は intentional に NULL/0 を渡してサイズだけ取る。
    unsafe {
        GetTokenInformation(
            token_handle,
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut required,
        );
    }
    if required == 0 {
        let err = std::io::Error::last_os_error();
        unsafe { CloseHandle(token_handle) };
        return Err(err);
    }

    let mut buf = vec![0u8; required as usize];
    // SAFETY: buf は required 以上のサイズを持つ。
    let res = unsafe {
        GetTokenInformation(
            token_handle,
            TokenUser,
            buf.as_mut_ptr() as *mut _,
            required,
            &mut required,
        )
    };
    unsafe { CloseHandle(token_handle) };
    if res == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: buf 先頭は TOKEN_USER 構造体として WinAPI が書き込んだもの。
    let token_user = unsafe { *(buf.as_ptr() as *const TOKEN_USER) };
    Ok(TokenUserBuf {
        token_user,
        _backing: buf,
    })
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
    let mut rx_peer_stream = ctx
        .event_bus
        .subscribe::<crate::event_bus::PeerStreamReceivedEvent>();

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
            r = rx_peer_stream.recv() => match r {
                Ok(ev) => filter_events(
                    filters_ref, EventCategory::PeerStream, None,
                    IpcEvent::PeerStreamReceived {
                        peer_id: ev.peer_id,
                        magic: ev.magic,
                        payload: ev.payload,
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
/// project_id に対応する CatalogManager が無ければ作る (ProjectOpen / Join 共通)。
/// chunk_max_files / chain_max_depth は net_config があればそれ、なければ既定値。
fn ensure_catalog_manager(ctx: &ServiceContext, project_id: &str) {
    if ctx.catalogs.contains_key(project_id) {
        return;
    }
    let (chunk_max, chain_max) = ctx
        .net_config
        .as_ref()
        .map(|c| (c.catalog.chunk_max_files, c.catalog.chain_max_depth))
        .unwrap_or((128, 32));
    ctx.catalogs.insert(
        project_id.to_string(),
        Arc::new(synergos_net::catalog::CatalogManager::new(
            project_id.to_string(),
            chunk_max,
            chain_max,
        )),
    );
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct ResolvedPublishPath {
    canonical: std::path::PathBuf,
    relative: std::path::PathBuf,
    relative_key: String,
}

/// Resolve a publish path at the I/O boundary and reject symlink/path traversal escapes.
/// This is called both before and after pre-publish hooks because hooks may replace paths.
async fn resolve_publish_path(
    project_root: &std::path::Path,
    requested: &std::path::Path,
) -> Result<ResolvedPublishPath, String> {
    use synergos_net::types::redact_path;

    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        project_root.join(requested)
    };
    let canonical = tokio::fs::canonicalize(&absolute).await.map_err(|error| {
        format!(
            "file not found or unreadable: {}: {error}",
            redact_path(project_root, &absolute)
        )
    })?;
    if !canonical.starts_with(project_root) {
        return Err(format!(
            "file outside project root: {}",
            redact_path(project_root, &canonical)
        ));
    }

    let metadata = tokio::fs::metadata(&canonical).await.map_err(|error| {
        format!(
            "metadata failed {}: {error}",
            redact_path(project_root, &canonical)
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "not a regular file: {}",
            redact_path(project_root, &canonical)
        ));
    }

    let relative = canonical
        .strip_prefix(project_root)
        .map_err(|_| "file escaped project root during path resolution".to_string())?
        .to_path_buf();
    let relative_key = crate::manifest::normalize_rel_path(&relative);
    if relative_key == crate::manifest::META_DIR
        || relative_key.starts_with(&format!("{}/", crate::manifest::META_DIR))
    {
        return Err(format!("cannot publish Synergos metadata: {relative_key}"));
    }

    Ok(ResolvedPublishPath {
        canonical,
        relative,
        relative_key,
    })
}

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
        } => {
            let pid_clone = project_id.clone();
            match ctx
                .project_manager
                .open_project(project_id, root_path, display_name)
                .await
            {
                Ok(()) => {
                    // CatalogManager を同じ project_id で立ち上げる (#26)。
                    ensure_catalog_manager(ctx, &pid_clone);
                    IpcResponse::Ok
                }
                Err(e) => IpcResponse::Error {
                    code: 1,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::ProjectClose { project_id } => {
            match ctx.project_manager.close_project(&project_id).await {
                Ok(()) => {
                    ctx.catalogs.remove(&project_id);
                    IpcResponse::Ok
                }
                Err(e) => IpcResponse::Error {
                    code: 1,
                    message: e.to_string(),
                },
            }
        }

        IpcCommand::ProjectList => {
            let mut projects = ctx.project_manager.list_projects();
            // ProjectManager は転送の状態を持たないので、ここで
            // Exchange から補う。Running / Queued 転送数を反映する。
            let transfers = ctx.exchange.list_transfers(None).await;
            for p in &mut projects {
                p.active_transfers = transfers
                    .iter()
                    .filter(|t| {
                        t.project_id == p.project_id
                            && matches!(t.state, TransferState::Running | TransferState::Queued)
                    })
                    .count();
            }
            IpcResponse::ProjectList(projects)
        }

        IpcCommand::ProjectGet { project_id } => {
            match ctx.project_manager.get_project(&project_id).await {
                Ok(mut detail) => {
                    let transfers = ctx.exchange.list_transfers(Some(&project_id)).await;
                    detail.active_transfers = transfers
                        .iter()
                        .filter(|t| {
                            matches!(t.state, TransferState::Running | TransferState::Queued)
                        })
                        .count();
                    IpcResponse::ProjectDetail(detail)
                }
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
            peer_info_url,
        } => {
            if ctx.project_manager.project_root(&project_id).is_none() {
                return IpcResponse::Error {
                    code: 1,
                    message: format!("project not found: {project_id}"),
                };
            }
            let advertised = crate::peer_join::resolve_advertised_peer_info_url(
                peer_info_url,
                ctx.net_config.as_deref(),
            );
            if let Some(url) = advertised.as_deref() {
                if let Err(error) = crate::peer_bootstrap::validate_bootstrap_url(url) {
                    return IpcResponse::Error {
                        code: 1,
                        message: error.to_string(),
                    };
                }
            }
            match (advertised, ctx.identity.as_ref()) {
                // 自己完結型: 別マシンの daemon で join できる
                (Some(url), Some(identity)) => {
                    let expires_at = match expires_in_secs {
                        Some(seconds) => match now_epoch_secs().checked_add(seconds) {
                            Some(value) => Some(value),
                            None => {
                                return IpcResponse::Error {
                                    code: 1,
                                    message: "invite expiration is too large".into(),
                                }
                            }
                        },
                        None => None,
                    };
                    let display_name = ctx
                        .project_manager
                        .list_projects()
                        .into_iter()
                        .find(|p| p.project_id == project_id)
                        .map(|p| p.display_name);
                    let (token, _) = crate::invite_token::issue(
                        identity,
                        &project_id,
                        display_name,
                        &url,
                        expires_at,
                    );
                    IpcResponse::InviteToken { token, expires_at }
                }
                // 従来型 (同一 daemon 内限定)。別マシンで使えない旨を警告する
                _ => {
                    tracing::warn!(
                        "invite for {project_id}: no advertised /peer-info URL (pass --url or set peer_info_advertised_url / peer_info_listen_addr); issuing a local-only token"
                    );
                    match ctx
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
                    }
                }
            }
        }

        IpcCommand::ProjectJoin {
            invite_token,
            root_path,
        } => {
            if !crate::invite_token::is_self_contained(&invite_token) {
                // 従来型: 発行 daemon と同じプロセス内でのみ有効
                return match ctx
                    .project_manager
                    .join_project(&invite_token, root_path)
                    .await
                {
                    Ok(_project_id) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: 1,
                        message: format!(
                            "{e} (a token without the `syn1.` prefix only works on the daemon that issued it; ask the host to run `project invite --url ...`)"
                        ),
                    },
                };
            }
            let payload = match crate::invite_token::decode(&invite_token, now_epoch_secs()) {
                Ok(p) => p,
                Err(e) => {
                    return IpcResponse::Error {
                        code: 1,
                        message: e.to_string(),
                    }
                }
            };
            // Validate all network-controlled bootstrap input before open_project,
            // so a malformed or redirect-oriented token has no local side effect.
            if let Err(error) =
                crate::peer_bootstrap::validate_bootstrap_url(&payload.peer_info_url)
            {
                return IpcResponse::Error {
                    code: 1,
                    message: error.to_string(),
                };
            }
            // 1. 同じ project_id でローカルに open (既に open 済みならそのまま)
            if ctx
                .project_manager
                .project_root(&payload.project_id)
                .is_none()
            {
                let root_path = match tokio::fs::canonicalize(&root_path).await {
                    Ok(path) if path.is_dir() => path,
                    _ => {
                        return IpcResponse::Error {
                            code: 1,
                            message: "join root must be an existing directory".into(),
                        }
                    }
                };
                if let Err(e) = ctx
                    .project_manager
                    .open_project(
                        payload.project_id.clone(),
                        root_path,
                        payload.display_name.clone(),
                    )
                    .await
                {
                    return IpcResponse::Error {
                        code: 1,
                        message: format!("open project {}: {e}", payload.project_id),
                    };
                }
                ensure_catalog_manager(ctx, &payload.project_id);
            }
            // 2. ホストへ bootstrap (QUIC 接続) + Presence 登録。相手 peer_id を照合
            let host = PeerId::new(payload.host_peer_id.clone());
            match crate::peer_join::bootstrap_and_register(
                ctx,
                &payload.project_id,
                &payload.peer_info_url,
                Some(&host),
            )
            .await
            {
                Ok(_) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 2,
                    message: format!(
                        "project {} opened locally but could not reach host at {}: {e}. Retry with `peer add-url {} {}` once the host is reachable",
                        payload.project_id,
                        payload.peer_info_url,
                        payload.project_id,
                        payload.peer_info_url
                    ),
                },
            }
        }

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
                    synergos_version: n.synergos_version,
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
                synergos_version: String::new(),
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

        IpcCommand::PeerAddByUrl { project_id, url } => {
            // プロジェクトが open 済みであることを確認 (URL 経由でも所属が必要)。
            if ctx.project_manager.project_root(&project_id).is_none() {
                return IpcResponse::Error {
                    code: 3,
                    message: format!("unknown project: {project_id}"),
                };
            }
            // /peer-info GET → QUIC connect (S1 真性認証込み) → Presence 登録
            match crate::peer_join::bootstrap_and_register(ctx, &project_id, &url, None).await {
                Ok(_) => IpcResponse::Ok,
                Err(message) => IpcResponse::Error { code: 2, message },
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

            // Validate the complete batch before running any hook. A malformed later path
            // must not cause side effects from an earlier hook.
            let mut prepared = Vec::with_capacity(file_paths.len());
            for path in &file_paths {
                match resolve_publish_path(&project_root, path).await {
                    Ok(resolved) => prepared.push(resolved),
                    Err(message) => {
                        return IpcResponse::Error { code: 3, message };
                    }
                }
            }

            // Run every pre-publish hook before manifest/version/history mutation. Thus a
            // failure on file N leaves all files in the batch unpublished.
            for entry in &prepared {
                if let Err(error) = ctx
                    .hooks
                    .run_pre_publish(&project_root, &project_id, &entry.relative_key)
                    .await
                {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!(
                            "pre-publish hook rejected {}: {error}",
                            entry.relative_key
                        ),
                    };
                }
            }

            // Hooks may rewrite or replace files. Re-resolve every path, re-check that it
            // remains the same project-relative file, and compute every CRC before mutation.
            let mut candidates = Vec::with_capacity(prepared.len());
            for entry in prepared {
                let resolved = match resolve_publish_path(&project_root, &entry.relative).await {
                    Ok(resolved) => resolved,
                    Err(message) => return IpcResponse::Error { code: 3, message },
                };
                if resolved.relative_key != entry.relative_key {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!(
                            "pre-publish hook changed path identity: {}",
                            entry.relative_key
                        ),
                    };
                }
                // 全体を RAM に載せずにストリーミングで CRC を取る (数百 MB のアセット対策)。
                // pre-publish フックが書き換えた後の内容を読む。
                let (crc, file_size) =
                    match crate::manifest::crc32_of_file(&resolved.canonical).await {
                        Ok(v) => v,
                        Err(e) => {
                            return IpcResponse::Error {
                                code: 3,
                                message: format!(
                                    "read failed {}: {e}",
                                    synergos_net::types::redact_path(
                                        &project_root,
                                        &resolved.canonical
                                    )
                                ),
                            };
                        }
                    };
                candidates.push((
                    resolved.canonical,
                    resolved.relative,
                    resolved.relative_key,
                    file_size,
                    crc,
                ));
            }

            let mut notifications: Vec<PublishNotification> =
                Vec::with_capacity(candidates.len());
            let mut published: Vec<(String, u64)> = Vec::with_capacity(candidates.len());
            for (canonical, rel, rel_key, file_size, crc) in candidates {
                // マニフェストでバージョン発番 (内容が同じなら据え置き = 再送しない)。
                // ProjectManager は node-local state の観測済み最大版を下限にする。
                let version = match ctx
                    .project_manager
                    .bump_file_version(
                        &project_id,
                        &rel_key,
                        file_size,
                        crc,
                        &ctx.exchange.local_peer_id().0,
                    )
                    .await
                {
                    Ok(crate::manifest::BumpOutcome::Bumped(v)) => v,
                    Ok(crate::manifest::BumpOutcome::Unchanged(v)) => {
                        tracing::info!(
                            "publish: {rel_key} unchanged (v{v}); re-offering without bump"
                        );
                        v
                    }
                    Err(e) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("manifest update failed for {rel_key}: {e}"),
                        };
                    }
                };

                let file_id = FileId::new(rel_key.clone());
                // ProjectManager に file_id → rel 相対パスを登録して、
                // 受信側の out_path_resolver が確実に解決できるようにする。
                ctx.project_manager
                    .register_file(&project_id, file_id.clone(), rel.clone());
                // 履歴ノードなら自分の publish 版も保管庫へ
                if let Err(error) = ctx
                    .exchange
                    .archive_to_history(crate::exchange::ArchiveRequest {
                        project_id: project_id.clone(),
                        file_id: file_id.clone(),
                        version,
                        size: file_size,
                        crc,
                        publisher: ctx.exchange.local_peer_id().0.clone(),
                        source: "published",
                        path: canonical.clone(),
                    })
                    .await
                {
                    return IpcResponse::Error {
                        code: 3,
                        message: format!("history archive failed for {rel_key}: {error}"),
                    };
                }

                published.push((rel_key, version));
                notifications.push(PublishNotification {
                    project_id: project_id.clone(),
                    file_id,
                    file_path: canonical,
                    file_size,
                    crc,
                    version,
                });
            }
            match ctx.exchange.publish_updates(notifications).await {
                Ok(()) => {
                    // post-publish: manifest 更新・Offer 送出の後。spawn するだけで待たない。
                    for (rel_key, version) in published {
                        ctx.hooks.spawn_post_hooks(
                            project_root.clone(),
                            crate::hooks::HookEvent::PostPublish,
                            project_id.clone(),
                            rel_key,
                            version,
                            None,
                        );
                    }
                    IpcResponse::Ok
                }
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

            // used_bandwidth: 実行中転送の speed_bps を合算
            let used_bw: u64 = ctx
                .exchange
                .list_transfers(None)
                .await
                .iter()
                .filter(|t| matches!(t.state, TransferState::Running))
                .map(|t| t.speed_bps)
                .sum();

            // max_connections: QUIC の max_concurrent_streams (設定由来)
            let max_connections = ctx
                .net_config
                .as_ref()
                .map(|cfg| cfg.quic.max_concurrent_streams.min(u32::from(u16::MAX)) as u16)
                .unwrap_or(0);

            IpcResponse::NetworkStatus(NetworkStatusInfo {
                primary_route,
                total_bandwidth_bps: total_bw,
                used_bandwidth_bps: used_bw,
                active_connections: connected_peers.len() as u16,
                max_connections,
                avg_latency_ms: avg_latency,
            })
        }

        // ── checkout / restore / 履歴ノード ──
        IpcCommand::ProjectCheckout {
            project_id,
            manifest_path,
        } => {
            let cctx = crate::checkout::CheckoutContext {
                projects: &ctx.project_manager,
                exchange: &ctx.exchange,
                history: &ctx.history,
            };
            match crate::checkout::checkout_project(&cctx, &project_id, manifest_path.as_deref())
                .await
            {
                Ok(report) => {
                    IpcResponse::CheckoutReport(synergos_ipc::response::CheckoutReportDto {
                        requested: report.requested,
                        up_to_date: report.up_to_date,
                        extra: report.extra,
                    })
                }
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("checkout failed: {e}"),
                },
            }
        }
        IpcCommand::ProjectRestore {
            project_id,
            rel_path,
            version,
        } => {
            let cctx = crate::checkout::CheckoutContext {
                projects: &ctx.project_manager,
                exchange: &ctx.exchange,
                history: &ctx.history,
            };
            match crate::checkout::restore_file(&cctx, &project_id, &rel_path, version).await {
                Ok(outcome) => {
                    tracing::info!("restore {project_id}/{rel_path} v{version}: {outcome:?}");
                    IpcResponse::Ok
                }
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("restore failed: {e}"),
                },
            }
        }
        IpcCommand::HistoryList {
            project_id,
            rel_path,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            match ctx
                .history
                .list(&root, &project_id, rel_path.as_deref())
                .await
            {
                Ok(items) => IpcResponse::HistoryList(
                    items
                        .into_iter()
                        .map(|v| synergos_ipc::response::HistoryVersionDto {
                            rel_path: v.rel,
                            version: v.version,
                            hash: v.hash,
                            size: v.size,
                            crc: v.crc,
                            stored_at: v.stored_at,
                            publisher: v.publisher,
                            source: v.source,
                        })
                        .collect(),
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("history list failed: {e}"),
                },
            }
        }
        IpcCommand::HistoryGc {
            project_id,
            purge,
            keep_manifests,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            if !ctx.history.covers(&project_id) {
                return IpcResponse::Error {
                    code: 4,
                    message: "this node is not a history node for the project".into(),
                };
            }
            // purge でなければ、手元 manifest + --keep-manifest の参照版を保護する。
            let mut keep = Vec::new();
            if !purge {
                keep.extend(
                    ctx.project_manager
                        .manifest_entries(&project_id)
                        .into_iter()
                        .map(|(rel, entry)| (rel, entry.version)),
                );
                for path in &keep_manifests {
                    match crate::manifest::ProjectManifest::load_from_file(path, &project_id).await {
                        Ok(manifest) => keep.extend(
                            manifest
                                .files
                                .into_iter()
                                .map(|(rel, entry)| (rel, entry.version)),
                        ),
                        Err(error) => {
                            return IpcResponse::Error {
                                code: 3,
                                message: format!(
                                    "keep manifest {} unreadable: {error}",
                                    path.display()
                                ),
                            }
                        }
                    }
                }
            }
            match ctx.history.gc(&root, &project_id, &keep, purge).await {
                Ok(report) => {
                    IpcResponse::HistoryGcReport(synergos_ipc::response::HistoryGcReportDto {
                        removed_versions: report.removed_versions,
                        removed_objects: report.removed_objects,
                        bytes_freed: report.bytes_freed,
                    })
                }
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("history gc failed: {e}"),
                },
            }
        }

        IpcCommand::HistoryRotate {
            project_id,
            dry_run,
            keep_manifests,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            if !ctx.history.covers(&project_id) {
                return IpcResponse::Error {
                    code: 4,
                    message: "this node is not a history node for the project".into(),
                };
            }
            let mut keep: Vec<(String, u64)> = ctx
                .project_manager
                .manifest_entries(&project_id)
                .into_iter()
                .map(|(rel, entry)| (rel, entry.version))
                .collect();
            for path in &keep_manifests {
                match crate::manifest::ProjectManifest::load_from_file(path, &project_id).await {
                    Ok(manifest) => keep.extend(
                        manifest
                            .files
                            .into_iter()
                            .map(|(rel, entry)| (rel, entry.version)),
                    ),
                    Err(error) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("keep manifest {} unreadable: {error}", path.display()),
                        }
                    }
                }
            }
            match ctx.history.rotate(&root, &project_id, &keep, dry_run).await {
                Ok(report) => IpcResponse::HistoryRotationReport(
                    synergos_ipc::response::HistoryRotationReportDto {
                        offloaded: report.offloaded,
                        bytes_offloaded: report.bytes_offloaded,
                        candidates: report.candidates,
                        skipped: report
                            .skipped
                            .into_iter()
                            .map(|s| (s.rel, s.version, s.reason))
                            .collect(),
                    },
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("history rotate failed: {e}"),
                },
            }
        }
        IpcCommand::HistoryOffloaded {
            project_id,
            rel_path,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            if !ctx.history.covers(&project_id) {
                return IpcResponse::Error {
                    code: 4,
                    message: "this node is not a history node for the project".into(),
                };
            }
            match ctx.history.offloaded(&root, &project_id, rel_path.as_deref()).await {
                Ok(items) => IpcResponse::HistoryOffloaded(
                    items
                        .into_iter()
                        .map(|v| synergos_ipc::response::HistoryOffloadedDto {
                            rel_path: v.rel,
                            version: v.version,
                            size: v.size,
                            backend: v.backend,
                            key: v.key,
                        })
                        .collect(),
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("history offloaded failed: {e}"),
                },
            }
        }
        IpcCommand::HistoryFetch {
            project_id,
            rel_path,
            version,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            if !ctx.history.covers(&project_id) {
                return IpcResponse::Error {
                    code: 4,
                    message: "this node is not a history node for the project".into(),
                };
            }
            match ctx
                .history
                .fetch_offloaded(&root, &project_id, &rel_path, version)
                .await
            {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("history fetch failed: {e}"),
                },
            }
        }

        IpcCommand::TagAdd {
            project_id,
            name,
            manifest_path,
            pins,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            if !ctx.history.covers(&project_id) {
                return IpcResponse::Error {
                    code: 4,
                    message: "this node is not a history node for the project".into(),
                };
            }
            let resolved: std::collections::BTreeMap<String, u64> = if !pins.is_empty() {
                pins.into_iter().collect()
            } else if let Some(path) = &manifest_path {
                match crate::manifest::ProjectManifest::load_from_file(path, &project_id).await {
                    Ok(manifest) => manifest
                        .files
                        .into_iter()
                        .map(|(rel, entry)| (rel, entry.version))
                        .collect(),
                    Err(error) => {
                        return IpcResponse::Error {
                            code: 3,
                            message: format!("manifest {} unreadable: {error}", path.display()),
                        }
                    }
                }
            } else {
                ctx.project_manager
                    .manifest_entries(&project_id)
                    .into_iter()
                    .map(|(rel, entry)| (rel, entry.version))
                    .collect()
            };
            if resolved.is_empty() {
                return IpcResponse::Error {
                    code: 3,
                    message: "nothing to pin: manifest has no entries".into(),
                };
            }
            match ctx.history.tag_add(&root, &project_id, &name, resolved).await {
                Ok(tag) => IpcResponse::Tag(synergos_ipc::response::TagDto {
                    name: tag.name,
                    created_at: tag.created_at,
                    pins: tag.pins.into_iter().collect(),
                }),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("tag add failed: {e}"),
                },
            }
        }
        IpcCommand::TagLs { project_id } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            match ctx.history.tag_list(&root, &project_id).await {
                Ok(items) => IpcResponse::TagList(
                    items
                        .into_iter()
                        .map(|t| synergos_ipc::response::TagSummaryDto {
                            name: t.name,
                            created_at: t.created_at,
                            pin_count: t.pin_count,
                        })
                        .collect(),
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("tag ls failed: {e}"),
                },
            }
        }
        IpcCommand::TagShow { project_id, name } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            match ctx.history.tag_show(&root, &project_id, &name).await {
                Ok(Some(tag)) => IpcResponse::Tag(synergos_ipc::response::TagDto {
                    name: tag.name,
                    created_at: tag.created_at,
                    pins: tag.pins.into_iter().collect(),
                }),
                Ok(None) => IpcResponse::Error {
                    code: 2,
                    message: format!("tag not found: {name}"),
                },
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("tag show failed: {e}"),
                },
            }
        }
        IpcCommand::TagRm { project_id, name } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            match ctx.history.tag_remove(&root, &project_id, &name).await {
                Ok(true) => IpcResponse::Ok,
                Ok(false) => IpcResponse::Error {
                    code: 2,
                    message: format!("tag not found: {name}"),
                },
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("tag rm failed: {e}"),
                },
            }
        }

        // ── publish / 受信時フック (docs/hooks.md) ──
        IpcCommand::HooksList { project_id } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            match ctx.hooks.effective_hooks(&root).await {
                Ok(hooks) => IpcResponse::HooksList(
                    hooks
                        .into_iter()
                        .map(|h| synergos_ipc::response::HookInfoDto {
                            source: h.source.as_str().to_string(),
                            event: h.def.event,
                            command: h.def.command,
                            r#match: h.def.r#match,
                            timeout_sec: h.def.timeout_sec,
                            disabled_by_opt_in: h.disabled_by_opt_in,
                        })
                        .collect(),
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("hooks list failed: {e}"),
                },
            }
        }
        IpcCommand::HooksRun {
            project_id,
            event,
            rel_path,
        } => {
            let Some(root) = ctx.project_manager.project_root(&project_id) else {
                return IpcResponse::Error {
                    code: 2,
                    message: format!("project not open: {project_id}"),
                };
            };
            let event = match event.as_str() {
                "pre-publish" => crate::hooks::HookEvent::PrePublish,
                "post-publish" => crate::hooks::HookEvent::PostPublish,
                "post-receive" => crate::hooks::HookEvent::PostReceive,
                other => {
                    return IpcResponse::Error {
                        code: 1,
                        message: format!("unknown hook event: {other}"),
                    };
                }
            };
            match ctx.hooks.run_manual(&root, event, &project_id, &rel_path).await {
                Ok(outcomes) => IpcResponse::HooksRunReport(
                    outcomes
                        .into_iter()
                        .map(|o| {
                            let (status, exit_code, detail) = match o.status {
                                crate::hooks::HookStatus::Success => {
                                    ("success".to_string(), None, None)
                                }
                                crate::hooks::HookStatus::Failed { exit_code } => {
                                    ("failed".to_string(), exit_code, None)
                                }
                                crate::hooks::HookStatus::TimedOut => {
                                    ("timed_out".to_string(), None, None)
                                }
                                crate::hooks::HookStatus::SpawnError(e) => {
                                    ("spawn_error".to_string(), None, Some(e))
                                }
                            };
                            synergos_ipc::response::HookRunResultDto {
                                source: o.source.as_str().to_string(),
                                command: o.command,
                                status,
                                exit_code,
                                detail,
                            }
                        })
                        .collect(),
                ),
                Err(e) => IpcResponse::Error {
                    code: 3,
                    message: format!("hooks run failed: {e}"),
                },
            }
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
        IpcCommand::PeerSendStream {
            peer_id,
            magic,
            payload,
        } => {
            let pid = PeerId::new(peer_id);
            let conn = match ctx.quic.raw_connection(&pid) {
                Some(c) => c,
                None => {
                    return IpcResponse::Error {
                        code: 5,
                        message: format!("peer {pid} not connected"),
                    }
                }
            };
            let res = async {
                let (mut send, _recv) =
                    conn.open_bi().await.map_err(|e| format!("open_bi: {e}"))?;
                send.write_all(&magic)
                    .await
                    .map_err(|e| format!("write magic: {e}"))?;
                send.write_all(&payload)
                    .await
                    .map_err(|e| format!("write payload: {e}"))?;
                send.finish().map_err(|e| format!("finish: {e}"))?;
                Ok::<_, String>(())
            }
            .await;
            match res {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    code: 5,
                    message: e,
                },
            }
        }
    }
}
