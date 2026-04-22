//! IPC クライアント
//!
//! synergos-core デーモンに接続するクライアント側ユーティリティ。
//! GUI / CLI / Ars Plugin から利用する。
//!
//! 接続後、背後のバックグラウンドタスクが ServerMessage を連続的に読み取り、
//! コマンドへの応答は `send()` の oneshot へ、イベントは内部 mpsc へ振り分ける。
//!
//! プラットフォーム:
//! - Unix (Linux / macOS): Unix Domain Socket
//! - Windows: Named Pipe (`\\.\pipe\synergos`)

use crate::command::IpcCommand;
use crate::event::IpcEvent;
use crate::response::IpcResponse;
use crate::transport::{IpcError, IpcTransport, ServerMessage};
use std::sync::Arc;
use tokio::io::AsyncRead;
#[cfg(test)]
use tokio::io::AsyncWrite;
use tokio::sync::{mpsc, oneshot, Mutex};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

/// IPC クライアント
///
/// 1 接続 1 クライアント。`send()` はシリアライズされ、背後の reader タスクから
/// 応答を受け取る。イベントは `recv_event()` で取得する。
pub struct IpcClient {
    inner: Option<Arc<ClientInner>>,
    event_rx: mpsc::UnboundedReceiver<IpcEvent>,
}

#[cfg(unix)]
type WriterHalf = tokio::net::unix::OwnedWriteHalf;
#[cfg(unix)]
type ReaderHalf = tokio::net::unix::OwnedReadHalf;

#[cfg(windows)]
type WriterHalf = tokio::io::WriteHalf<NamedPipeClient>;
#[cfg(windows)]
type ReaderHalf = tokio::io::ReadHalf<NamedPipeClient>;

struct ClientInner {
    /// 送信側ストリーム（書き込みは排他）
    writer: Mutex<WriterHalf>,
    /// 応答待ちキュー（FIFO）
    pending: Mutex<std::collections::VecDeque<oneshot::Sender<IpcResponse>>>,
}

impl IpcClient {
    /// synergos-core デーモンに接続する
    pub async fn connect() -> Result<Self, IpcError> {
        let (reader, writer) = Self::open_stream().await?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let inner = Arc::new(ClientInner {
            writer: Mutex::new(writer),
            pending: Mutex::new(std::collections::VecDeque::new()),
        });

        // バックグラウンドで応答/イベントを振り分ける reader タスク
        tokio::spawn(reader_task(reader, inner.clone(), event_tx));

        Ok(Self {
            inner: Some(inner),
            event_rx,
        })
    }

    #[cfg(unix)]
    async fn open_stream() -> Result<(ReaderHalf, WriterHalf), IpcError> {
        let path = crate::transport::socket_path();
        let stream = tokio::net::UnixStream::connect(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::ConnectionRefused
                || e.kind() == std::io::ErrorKind::NotFound
            {
                IpcError::DaemonNotRunning
            } else {
                IpcError::Io(e)
            }
        })?;
        Ok(stream.into_split())
    }

    #[cfg(windows)]
    async fn open_stream() -> Result<(ReaderHalf, WriterHalf), IpcError> {
        let path = crate::transport::socket_path();
        let path_str = path.to_string_lossy().to_string();
        // Named Pipe サーバが busy / 未起動の場合は DaemonNotRunning に丸める。
        let client = ClientOptions::new().open(&path_str).map_err(|e| {
            use std::io::ErrorKind;
            match e.kind() {
                ErrorKind::NotFound => IpcError::DaemonNotRunning,
                _ => {
                    // Named Pipe 特有の "pipe is busy" (ERROR_PIPE_BUSY=231)
                    // もデーモン未起動と同じ扱いにする。
                    if e.raw_os_error() == Some(231) {
                        IpcError::DaemonNotRunning
                    } else {
                        IpcError::Io(e)
                    }
                }
            }
        })?;
        Ok(tokio::io::split(client))
    }

    /// コマンドを送信し、レスポンスを受信する
    pub async fn send(&mut self, command: IpcCommand) -> Result<IpcResponse, IpcError> {
        let inner = self
            .inner
            .as_ref()
            .ok_or(IpcError::DaemonNotRunning)?
            .clone();

        // 応答の待ち口を pending に積む
        let (resp_tx, resp_rx) = oneshot::channel();

        // 書き込みは排他ロックで順序保証。pending への push は書き込みと同時にアトミックに行う
        let mut writer = inner.writer.lock().await;
        {
            let mut pending = inner.pending.lock().await;
            pending.push_back(resp_tx);
        }
        IpcTransport::write_message(&mut *writer, &command).await?;
        drop(writer);

        // reader タスクから応答が来るのを待つ（接続切断で drop されうる）
        resp_rx.await.map_err(|_| IpcError::ConnectionClosed)
    }

    /// イベントを受信する
    pub async fn recv_event(&mut self) -> Result<IpcEvent, IpcError> {
        self.event_rx.recv().await.ok_or(IpcError::ConnectionClosed)
    }

    /// デーモンが稼働中かチェック
    pub async fn is_daemon_running() -> bool {
        Self::open_stream().await.is_ok()
    }
}

async fn reader_task<R>(
    mut reader: R,
    inner: Arc<ClientInner>,
    event_tx: mpsc::UnboundedSender<IpcEvent>,
) where
    R: AsyncRead + Unpin,
{
    loop {
        let msg: ServerMessage = match IpcTransport::read_message(&mut reader).await {
            Ok(m) => m,
            Err(IpcError::ConnectionClosed) => break,
            Err(e) => {
                tracing::warn!("IPC read error: {}", e);
                break;
            }
        };
        match msg {
            ServerMessage::Response(resp) => {
                let waiter = {
                    let mut pending = inner.pending.lock().await;
                    pending.pop_front()
                };
                if let Some(tx) = waiter {
                    let _ = tx.send(resp);
                } else {
                    tracing::warn!("Dropped orphan IPC response (no waiter)");
                }
            }
            ServerMessage::Event(event) => {
                if event_tx.send(event).is_err() {
                    // 受信側が閉じていたら無視
                }
            }
        }
    }

    // 接続切断時は保留中の send をすべて失敗にする
    let mut pending = inner.pending.lock().await;
    pending.clear();
}

// AsyncWrite bound 静的確認 (コンパイル時のみ実行される保証 check)。
#[cfg(test)]
const _: fn() = || {
    fn _is_write<W: AsyncWrite + Unpin>() {}
    _is_write::<WriterHalf>();
};
