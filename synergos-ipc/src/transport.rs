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
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(runtime_dir)
            .join("synergos")
            .join("synergos.sock")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::IpcCommand;
    use crate::event::{EventCategory, EventFilter, IpcEvent};
    use crate::response::{DaemonStatus, IpcResponse};
    use assert_matches::assert_matches;
    use pretty_assertions::assert_eq;
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;
    use tokio::io::{duplex, AsyncWriteExt};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Sample {
        id: u32,
        name: String,
    }

    /// T-IPC-01: write_read_roundtrip — write_message で書き込んだものが read_message で読み戻せる
    #[tokio::test]
    async fn write_read_roundtrip() {
        let (mut a, mut b) = duplex(4096);
        let sent = Sample {
            id: 42,
            name: "hello".to_string(),
        };

        IpcTransport::write_message(&mut a, &sent).await.unwrap();
        let received: Sample = IpcTransport::read_message(&mut b).await.unwrap();

        assert_eq!(sent, received);
    }

    /// T-IPC-02: read_eof_returns_connection_closed — 書き込み側が閉じると ConnectionClosed が返る
    #[tokio::test]
    async fn read_eof_returns_connection_closed() {
        let (a, mut b) = duplex(64);
        drop(a);

        let result: Result<Sample, _> = IpcTransport::read_message(&mut b).await;

        assert_matches!(result, Err(IpcError::ConnectionClosed));
    }

    /// T-IPC-03: message_too_large_write — 書き込み時の上限超過チェック
    #[tokio::test]
    async fn message_too_large_write() {
        // 16 MiB + α のデータ（MessagePack エンコード後に MAX を超える）
        let big: Vec<u8> = vec![0u8; (MAX_MESSAGE_SIZE as usize) + 1024];

        // duplex バッファは小さいが、サイズ判定で事前に弾かれるため実書き込みは起きない
        let (mut a, _b) = duplex(64);
        let result = IpcTransport::write_message(&mut a, &big).await;

        assert_matches!(result, Err(IpcError::MessageTooLarge { .. }));
    }

    /// T-IPC-04: message_too_large_read — 巨大なヘッダ長が届いたときに MessageTooLarge で弾く
    #[tokio::test]
    async fn message_too_large_read() {
        let (mut a, mut b) = duplex(64);

        // MAX + 1 のヘッダのみ書き込む（payload は届けない）
        let oversized = (MAX_MESSAGE_SIZE + 1).to_le_bytes();
        a.write_all(&oversized).await.unwrap();
        a.flush().await.unwrap();
        drop(a);

        let result: Result<Sample, _> = IpcTransport::read_message(&mut b).await;

        assert_matches!(
            result,
            Err(IpcError::MessageTooLarge { size, max })
                if size == MAX_MESSAGE_SIZE + 1 && max == MAX_MESSAGE_SIZE
        );
    }

    /// T-IPC-05: partial_header_eof — 4 byte ヘッダ未満で EOF → ConnectionClosed
    #[tokio::test]
    async fn partial_header_eof() {
        let (mut a, mut b) = duplex(64);
        a.write_all(&[0x01, 0x02]).await.unwrap(); // 2 byte だけ
        a.flush().await.unwrap();
        drop(a);

        let result: Result<Sample, _> = IpcTransport::read_message(&mut b).await;

        assert_matches!(result, Err(IpcError::ConnectionClosed));
    }

    /// T-IPC-06: multiple_messages_streamed — 同じストリームで複数メッセージが順に読める
    #[tokio::test]
    async fn multiple_messages_streamed() {
        let (mut a, mut b) = duplex(8192);

        let m1 = Sample {
            id: 1,
            name: "one".to_string(),
        };
        let m2 = Sample {
            id: 2,
            name: "two".to_string(),
        };
        let m3 = Sample {
            id: 3,
            name: "three".to_string(),
        };

        IpcTransport::write_message(&mut a, &m1).await.unwrap();
        IpcTransport::write_message(&mut a, &m2).await.unwrap();
        IpcTransport::write_message(&mut a, &m3).await.unwrap();

        let r1: Sample = IpcTransport::read_message(&mut b).await.unwrap();
        let r2: Sample = IpcTransport::read_message(&mut b).await.unwrap();
        let r3: Sample = IpcTransport::read_message(&mut b).await.unwrap();

        assert_eq!((m1, m2, m3), (r1, r2, r3));
    }

    /// T-IPC-07: socket_path_linux_uses_xdg_runtime_dir — XDG_RUNTIME_DIR を反映
    ///
    /// 環境変数を書き換えるため `serial_test` で直列実行。
    #[cfg(target_os = "linux")]
    #[serial_test::serial]
    #[test]
    fn socket_path_linux_uses_xdg_runtime_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let original = std::env::var("XDG_RUNTIME_DIR").ok();

        // SAFETY: serial_test で直列化されているため他テストと競合しない
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", tmp.path());
        }

        let path = socket_path();
        assert_eq!(path, tmp.path().join("synergos").join("synergos.sock"));

        unsafe {
            match original {
                Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }

    /// T-IPC-07b: socket_path_linux_fallback_tmp — XDG_RUNTIME_DIR 未設定時は /tmp
    #[cfg(target_os = "linux")]
    #[serial_test::serial]
    #[test]
    fn socket_path_linux_fallback_tmp() {
        let original = std::env::var("XDG_RUNTIME_DIR").ok();

        unsafe {
            std::env::remove_var("XDG_RUNTIME_DIR");
        }

        let path = socket_path();
        assert_eq!(
            path,
            PathBuf::from("/tmp").join("synergos").join("synergos.sock")
        );

        unsafe {
            if let Some(v) = original {
                std::env::set_var("XDG_RUNTIME_DIR", v);
            }
        }
    }

    /// T-IPC-08: command_response_event_serde_full — 主要な IPC 型の往復シリアライズ
    #[tokio::test]
    async fn command_response_event_serde_full() {
        // IpcCommand::ProjectOpen
        let cmd = IpcCommand::ProjectOpen {
            project_id: "proj-1".to_string(),
            root_path: PathBuf::from("/tmp/proj"),
            display_name: Some("My Project".to_string()),
        };
        let (mut a, mut b) = duplex(8192);
        IpcTransport::write_message(&mut a, &cmd).await.unwrap();
        let decoded: IpcCommand = IpcTransport::read_message(&mut b).await.unwrap();
        assert_matches!(
            decoded,
            IpcCommand::ProjectOpen { ref project_id, ref display_name, .. }
                if project_id == "proj-1" && display_name.as_deref() == Some("My Project")
        );

        // IpcResponse::Status
        let resp = IpcResponse::Status(DaemonStatus {
            pid: 12345,
            started_at: 1_700_000_000,
            project_count: 2,
            active_connections: 3,
            active_transfers: 4,
        });
        let (mut a, mut b) = duplex(8192);
        IpcTransport::write_message(&mut a, &resp).await.unwrap();
        let decoded: IpcResponse = IpcTransport::read_message(&mut b).await.unwrap();
        assert_matches!(
            decoded,
            IpcResponse::Status(s) if s.pid == 12345 && s.active_transfers == 4
        );

        // IpcEvent::ConflictDetected
        let ev = IpcEvent::ConflictDetected {
            project_id: "proj-1".to_string(),
            file_id: "f-1".to_string(),
            file_path: "/a/b.txt".to_string(),
            involved_peers: vec!["peer-a".to_string(), "peer-b".to_string()],
        };
        let (mut a, mut b) = duplex(8192);
        IpcTransport::write_message(&mut a, &ev).await.unwrap();
        let decoded: IpcEvent = IpcTransport::read_message(&mut b).await.unwrap();
        assert_matches!(
            decoded,
            IpcEvent::ConflictDetected { ref involved_peers, .. } if involved_peers.len() == 2
        );

        // EventFilter (全バリアント)
        let filters = vec![
            EventFilter::All,
            EventFilter::Project("p".to_string()),
            EventFilter::Category(EventCategory::Transfer),
        ];
        let (mut a, mut b) = duplex(8192);
        IpcTransport::write_message(&mut a, &filters).await.unwrap();
        let decoded: Vec<EventFilter> = IpcTransport::read_message(&mut b).await.unwrap();
        assert_eq!(decoded.len(), 3);
    }
}
