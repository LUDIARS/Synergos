//! IPC クライアント
//!
//! synergos-core デーモンに接続するクライアント側ユーティリティ。
//! GUI / CLI / Ars Plugin から利用する。

use crate::command::IpcCommand;
use crate::event::IpcEvent;
use crate::response::IpcResponse;
use crate::transport::{IpcError, IpcTransport};
use tokio::sync::mpsc;

/// IPC クライアント
///
/// synergos-core デーモンへの接続を管理し、
/// コマンド送信・レスポンス受信・イベント受信を提供する。
pub struct IpcClient {
    /// イベント受信チャンネル
    event_rx: mpsc::UnboundedReceiver<IpcEvent>,
    /// 内部状態
    _state: ClientState,
}

enum ClientState {
    Disconnected,
    #[cfg(unix)]
    Connected {
        stream: tokio::net::UnixStream,
    },
    // Windows Named Pipe は将来実装
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

        let (_event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            event_rx,
            _state: ClientState::Connected { stream },
        })
    }

    /// Windows 用の接続（スタブ）
    #[cfg(windows)]
    pub async fn connect() -> Result<Self, IpcError> {
        // Named Pipe 接続は Phase 2 で実装
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        Ok(Self {
            event_rx,
            _state: ClientState::Disconnected,
        })
    }

    /// コマンドを送信し、レスポンスを受信する
    pub async fn send(&mut self, command: IpcCommand) -> Result<IpcResponse, IpcError> {
        match &mut self._state {
            #[cfg(unix)]
            ClientState::Connected { stream } => {
                let (mut reader, mut writer) = stream.split();
                IpcTransport::write_message(&mut writer, &command).await?;
                let response: IpcResponse =
                    IpcTransport::read_message(&mut reader).await?;
                Ok(response)
            }
            _ => Err(IpcError::DaemonNotRunning),
        }
    }

    /// イベントを受信する（購読中のみ）
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
            false // TODO: Named Pipe チェック
        }
    }
}
