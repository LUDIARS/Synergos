//! IPC トランスポート抽象化
//!
//! クロスプラットフォーム IPC トランスポートを提供する。
//! - Linux / macOS: Unix Domain Socket
//! - Windows: Named Pipe
//!
//! プロトコル: 長さプレフィクス付きメッセージフレーム (4 byte LE length + MessagePack payload)

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::event::IpcEvent;
use crate::response::IpcResponse;

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

/// 最大メッセージサイズ (1 MiB) — DoS 対策のため小さめに設定
pub const MAX_MESSAGE_SIZE: u32 = 1 * 1024 * 1024;

/// 1 回の read で確保するチャンクサイズ上限（ピーク確保量制限）
const READ_CHUNK: usize = 64 * 1024;

/// Core デーモン → クライアントへ送るメッセージの封筒
///
/// 同一の Unix Domain Socket 上で、コマンドに対する応答 (`Response`) と
/// 非同期にプッシュされるイベント (`Event`) を多重化する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    Response(IpcResponse),
    Event(IpcEvent),
}

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
    ///
    /// DoS 対策として、長さチェック後でも一度に確保するバッファは `READ_CHUNK`
    /// 単位で段階的に拡張する。
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

        // 段階的に読み込み: 事前に一括アロケートしないことで、
        // 攻撃者が len=MAX を繰り返し送って即座にメモリを確保させる経路を塞ぐ
        let target = len as usize;
        let mut payload = Vec::with_capacity(target.min(READ_CHUNK));
        let mut remaining = target;
        let mut chunk = vec![0u8; READ_CHUNK.min(target.max(1))];
        while remaining > 0 {
            let want = chunk.len().min(remaining);
            let n = reader.read(&mut chunk[..want]).await?;
            if n == 0 {
                return Err(IpcError::ConnectionClosed);
            }
            payload.extend_from_slice(&chunk[..n]);
            remaining -= n;
        }
        let message = rmp_serde::from_slice(&payload)?;
        Ok(message)
    }
}
