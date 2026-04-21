//! IPC クライアント
//!
//! synergos-core デーモンに接続するクライアント側ユーティリティ。
//! GUI / CLI / Ars Plugin から利用する。
//!
//! 接続後、背後のバックグラウンドタスクが ServerMessage を連続的に読み取り、
//! コマンドへの応答は `send()` の oneshot へ、イベントは内部 mpsc へ振り分ける。

use crate::command::IpcCommand;
use crate::event::IpcEvent;
use crate::response::IpcResponse;
use crate::transport::{IpcError, IpcTransport, ServerMessage};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// IPC クライアント
///
/// 1 接続 1 クライアント。`send()` はシリアライズされ、背後の reader タスクから
/// 応答を受け取る。イベントは `recv_event()` で取得する。
pub struct IpcClient {
    inner: Option<Arc<ClientInner>>,
    event_rx: mpsc::UnboundedReceiver<IpcEvent>,
}

struct ClientInner {
    /// 送信側ストリーム（書き込みは排他）
    #[cfg(unix)]
    writer: Mutex<tokio::net::unix::OwnedWriteHalf>,
    /// 応答待ちキュー（FIFO）
    pending: Mutex<std::collections::VecDeque<oneshot::Sender<IpcResponse>>>,
}

impl IpcClient {
    /// synergos-core デーモンに接続する
    #[cfg(unix)]
    pub async fn connect() -> Result<Self, IpcError> {
        let path = crate::transport::socket_path();
        let stream = tokio::net::UnixStream::connect(&path)
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::NotFound
                {
                    IpcError::DaemonNotRunning
                } else {
                    IpcError::Io(e)
                }
            })?;

        let (reader, writer) = stream.into_split();
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

    /// Windows 用の接続（未実装）
    #[cfg(windows)]
    pub async fn connect() -> Result<Self, IpcError> {
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        Ok(Self {
            inner: None,
            event_rx,
        })
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

        #[cfg(unix)]
        {
            // 書き込みは排他ロックで順序保証。pending への push は書き込みと同時にアトミックに行う
            let mut writer = inner.writer.lock().await;
            {
                let mut pending = inner.pending.lock().await;
                pending.push_back(resp_tx);
            }
            IpcTransport::write_message(&mut *writer, &command).await?;
        }
        #[cfg(not(unix))]
        {
            let _ = &inner;
            let _ = resp_tx;
            return Err(IpcError::DaemonNotRunning);
        }

        // reader タスクから応答が来るのを待つ（接続切断で drop されうる）
        resp_rx.await.map_err(|_| IpcError::ConnectionClosed)
    }

    /// イベントを受信する
    pub async fn recv_event(&mut self) -> Result<IpcEvent, IpcError> {
        self.event_rx
            .recv()
            .await
            .ok_or(IpcError::ConnectionClosed)
    }

    /// デーモンが稼働中かチェック
    pub async fn is_daemon_running() -> bool {
        let path = crate::transport::socket_path();
        #[cfg(unix)]
        {
            tokio::net::UnixStream::connect(&path).await.is_ok()
        }
        #[cfg(windows)]
        {
            let _ = path;
            false
        }
    }
}

#[cfg(unix)]
async fn reader_task(
    mut reader: tokio::net::unix::OwnedReadHalf,
    inner: Arc<ClientInner>,
    event_tx: mpsc::UnboundedSender<IpcEvent>,
) {
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
