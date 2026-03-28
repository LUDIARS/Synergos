//! IPC トランスポート抽象化
//!
//! クロスプラットフォーム IPC トランスポートを提供する。
//! - Linux / macOS: Unix Domain Socket
//! - Windows: Named Pipe
//!
//! プロトコル: 長さプレフィクス付きメッセージフレーム (4 byte LE length + MessagePack payload)

use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// IPC エラー
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialize(#[from] rmp_serde::encode::Error),

    #[error("Deserialization error: {0}")]
    Deserialize(#[from] rmp_serde::decode::Error),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Message too large: {size} bytes (max {max})")]
    MessageTooLarge { size: u32, max: u32 },

    #[error("Daemon not running")]
    DaemonNotRunning,
}

/// 最大メッセージサイズ (16 MiB)
const MAX_MESSAGE_SIZE: u32 = 16 * 1024 * 1024;

/// IPC ソケットパスを取得（プラットフォーム依存）
pub fn socket_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir).join("synergos").join("synergos.sock")
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Synergos")
            .join("synergos.sock")
    }

    #[cfg(target_os = "windows")]
    {
        PathBuf::from(r"\\.\pipe\synergos")
    }
}

/// IPC トランスポート trait
///
/// 長さプレフィクス付きメッセージフレームの読み書きを抽象化する。
pub struct IpcTransport;

impl IpcTransport {
    /// メッセージを書き込む（4 byte LE length + MessagePack payload）
    pub async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), IpcError>
    where
        W: AsyncWriteExt + Unpin,
        T: Serialize,
    {
        let payload = rmp_serde::to_vec(message)?;
        let len = payload.len() as u32;
        if len > MAX_MESSAGE_SIZE {
            return Err(IpcError::MessageTooLarge {
                size: len,
                max: MAX_MESSAGE_SIZE,
            });
        }
        writer.write_all(&len.to_le_bytes()).await?;
        writer.write_all(&payload).await?;
        writer.flush().await?;
        Ok(())
    }

    /// メッセージを読み取る（4 byte LE length + MessagePack payload）
    pub async fn read_message<R, T>(reader: &mut R) -> Result<T, IpcError>
    where
        R: AsyncReadExt + Unpin,
        T: DeserializeOwned,
    {
        let mut len_buf = [0u8; 4];
        match reader.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(IpcError::ConnectionClosed);
            }
            Err(e) => return Err(IpcError::Io(e)),
        }
        let len = u32::from_le_bytes(len_buf);
        if len > MAX_MESSAGE_SIZE {
            return Err(IpcError::MessageTooLarge {
                size: len,
                max: MAX_MESSAGE_SIZE,
            });
        }
        let mut payload = vec![0u8; len as usize];
        reader.read_exact(&mut payload).await?;
        let message = rmp_serde::from_slice(&payload)?;
        Ok(message)
    }
}
