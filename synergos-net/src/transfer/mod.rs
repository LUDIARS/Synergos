//! 実ファイル転送の最小実装
//!
//! 目的:
//! - QUIC 双方向ストリーム上でファイルをチャンク化して送受信する
//! - チャンクごとに Blake3 ハッシュで整合性を検証する
//! - 受信側はディスクに書き込み、完了時に全体ハッシュを検証する
//!
//! スコープ:
//! - 本モジュールは **プロトコルと I/O ロジック** を提供する純粋ユーティリティ。
//! - QUIC ストリームとの接続は呼び出し側 (Exchange) が行う。
//! - 再送 / 並列取得 / resume は追って拡張する。現バージョンは単一ストリームで
//!   順次送る最小仕様。

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Result, SynergosNetError};
use crate::types::Blake3Hash;

/// 1 チャンクあたりのバイト数 (64 KiB)。
/// QUIC の congestion window を越えない範囲で十分大きく、かつ
/// メモリ確保の一度あたりのサイズを抑える値。
pub const CHUNK_SIZE: usize = 64 * 1024;

/// 転送ヘッダ (ストリームの先頭に 1 回だけ送る)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHeader {
    /// 識別子 (ログ用)
    pub transfer_id: String,
    /// プロジェクト ID
    pub project_id: String,
    /// ファイル ID
    pub file_id: String,
    /// 全体サイズ (bytes)
    pub total_size: u64,
    /// チャンク総数
    pub chunk_count: u64,
    /// 全体 Blake3 ハッシュ (受信側で最終検証)
    pub total_hash: Blake3Hash,
}

/// 各チャンクのフレーム。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkFrame {
    /// 0-origin のチャンク index
    pub index: u64,
    /// 当該チャンク単体の Blake3 ハッシュ
    pub hash: Blake3Hash,
    /// 生データ (msgpack の bin として直列化するため serde_bytes を使う)
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// 終端マーカ (ヘッダで chunk_count 分送った後に 1 度だけ送る)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFooter {
    /// 全体 Blake3 ハッシュを再度送ることで、パス上の改竄を検出する。
    pub total_hash: Blake3Hash,
}

/// 送信側: `reader` からデータを読み、`writer` (QUIC bidi の send side) に書く。
///
/// ヘッダの `total_hash` / `total_size` / `chunk_count` は呼び出し側が
/// 事前に計算して渡す。(ローカルで `hash_file` を事前実行する想定)
pub async fn send_stream<R, W>(mut reader: R, mut writer: W, header: TransferHeader) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    write_frame(&mut writer, &FrameKind::Header(header.clone())).await?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut index: u64 = 0;
    let mut remaining = header.total_size;
    while remaining > 0 {
        let want = (remaining as usize).min(CHUNK_SIZE);
        let mut filled = 0usize;
        while filled < want {
            let n = reader.read(&mut buf[filled..want]).await?;
            if n == 0 {
                return Err(SynergosNetError::Transfer(
                    "unexpected EOF while reading source".into(),
                ));
            }
            filled += n;
        }
        let hash = Blake3Hash(*blake3::hash(&buf[..filled]).as_bytes());
        let frame = ChunkFrame {
            index,
            hash,
            data: buf[..filled].to_vec(),
        };
        write_frame(&mut writer, &FrameKind::Chunk(frame)).await?;
        remaining -= filled as u64;
        index += 1;
    }
    write_frame(
        &mut writer,
        &FrameKind::Footer(TransferFooter {
            total_hash: header.total_hash,
        }),
    )
    .await?;
    writer.shutdown().await.ok();
    Ok(())
}

/// 受信側: `reader` (QUIC bidi の recv side) から読み、検証しながら `out_path`
/// へ書く。完了時にヘッダと一致する全体ハッシュを返す。
pub async fn receive_stream<R>(mut reader: R, out_path: &Path) -> Result<TransferHeader>
where
    R: AsyncRead + Unpin,
{
    let header = match read_frame(&mut reader).await? {
        FrameKind::Header(h) => h,
        _ => {
            return Err(SynergosNetError::Transfer(
                "expected header as first frame".into(),
            ));
        }
    };

    // 親ディレクトリが無ければ作成
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let mut file = tokio::fs::File::create(out_path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut received_chunks: u64 = 0;

    loop {
        let frame = read_frame(&mut reader).await?;
        match frame {
            FrameKind::Chunk(c) => {
                let expected_hash = Blake3Hash(*blake3::hash(&c.data).as_bytes());
                if expected_hash != c.hash {
                    return Err(SynergosNetError::Transfer(format!(
                        "chunk {} hash mismatch",
                        c.index
                    )));
                }
                if c.index != received_chunks {
                    return Err(SynergosNetError::Transfer(format!(
                        "chunk {} arrived out of order (expected {})",
                        c.index, received_chunks
                    )));
                }
                hasher.update(&c.data);
                file.write_all(&c.data).await?;
                received_chunks += 1;
            }
            FrameKind::Footer(f) => {
                let total = Blake3Hash(*hasher.finalize().as_bytes());
                if total != f.total_hash {
                    return Err(SynergosNetError::Transfer(
                        "total hash mismatch at footer".into(),
                    ));
                }
                if total != header.total_hash {
                    return Err(SynergosNetError::Transfer(
                        "total hash mismatch with header".into(),
                    ));
                }
                if received_chunks != header.chunk_count {
                    return Err(SynergosNetError::Transfer(format!(
                        "chunk count mismatch: expected {} got {}",
                        header.chunk_count, received_chunks
                    )));
                }
                file.flush().await?;
                return Ok(header);
            }
            FrameKind::Header(_) => {
                return Err(SynergosNetError::Transfer("duplicate header frame".into()));
            }
        }
    }
}

/// QUIC ストリーム上の転送セッションを示すマジックバイト列。
/// Daemon のストリーム受信ディスパッチャはこの 4 byte を先読みして
/// DHT / Transfer / その他を振り分ける。
pub const TRANSFER_STREAM_MAGIC: &[u8; 4] = b"TXFR";

/// 送信ラッパ: QUIC bidi send half の先頭に `TXFR` を書き、続けて send_stream 本体。
/// 受信側はディスパッチャで magic を消費した後の recv half を `receive_stream` に
/// そのまま流す。
pub async fn send_over_quic<R>(
    mut send: quinn::SendStream,
    reader: R,
    header: TransferHeader,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    send.write_all(TRANSFER_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("write magic: {e}")))?;
    send_stream(reader, send, header).await
}

/// 受信ラッパ: magic は呼び出し側で既に消費済みの前提。QUIC recv half を
/// そのまま `receive_stream` に渡す。
pub async fn receive_over_quic(
    recv: quinn::RecvStream,
    out_path: &Path,
) -> Result<TransferHeader> {
    receive_stream(recv, out_path).await
}

/// ディスク上のファイルの全体 Blake3 ハッシュとサイズ + チャンク数を計算する。
pub async fn hash_file(path: &Path) -> Result<(Blake3Hash, u64, u64)> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let chunk_count = if total == 0 {
        0
    } else {
        total.div_ceil(CHUNK_SIZE as u64)
    };
    Ok((
        Blake3Hash(*hasher.finalize().as_bytes()),
        total,
        chunk_count,
    ))
}

// ---- 内部 I/O ----

#[derive(Debug, Clone, Serialize, Deserialize)]
enum FrameKind {
    Header(TransferHeader),
    Chunk(ChunkFrame),
    Footer(TransferFooter),
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &FrameKind) -> Result<()> {
    let payload = rmp_serde::to_vec(frame)
        .map_err(|e| SynergosNetError::Transfer(format!("encode frame: {e}")))?;
    let len = (payload.len() as u32).to_le_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&payload).await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<FrameKind> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    // 64KiB (CHUNK_SIZE) + MessagePack オーバーヘッド上限。
    // 上限を超える frame は悪意ある送信者の可能性があるので弾く。
    const MAX_FRAME: usize = CHUNK_SIZE + 1024;
    if len > MAX_FRAME {
        return Err(SynergosNetError::Transfer(format!(
            "frame too large: {len} bytes"
        )));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    rmp_serde::from_slice(&payload)
        .map_err(|e| SynergosNetError::Transfer(format!("decode frame: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn roundtrip_small_file() {
        // 10 KiB のランダムデータをメモリ上の双方向パイプで送受信する。
        let tmp_src =
            std::env::temp_dir().join(format!("synergos-tx-src-{}", uuid::Uuid::new_v4()));
        let tmp_dst =
            std::env::temp_dir().join(format!("synergos-tx-dst-{}", uuid::Uuid::new_v4()));
        let data = vec![42u8; 10 * 1024];
        tokio::fs::write(&tmp_src, &data).await.unwrap();

        let (h, size, chunks) = hash_file(&tmp_src).await.unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(chunks, 1);

        let header = TransferHeader {
            transfer_id: "t1".into(),
            project_id: "p".into(),
            file_id: "f".into(),
            total_size: size,
            chunk_count: chunks,
            total_hash: h,
        };

        let (tx, rx) = duplex(64 * 1024);
        let src_reader = tokio::fs::File::open(&tmp_src).await.unwrap();
        let send_task =
            tokio::spawn(async move { send_stream(src_reader, tx, header.clone()).await });
        let dst_path = tmp_dst.clone();
        let recv_task = tokio::spawn(async move { receive_stream(rx, &dst_path).await });

        send_task.await.unwrap().unwrap();
        let recv_header = recv_task.await.unwrap().unwrap();
        assert_eq!(recv_header.total_size, data.len() as u64);

        let received = tokio::fs::read(&tmp_dst).await.unwrap();
        assert_eq!(received, data);

        let _ = tokio::fs::remove_file(&tmp_src).await;
        let _ = tokio::fs::remove_file(&tmp_dst).await;
    }

    #[tokio::test]
    async fn tampered_chunk_fails() {
        // 正しく送信し始めるが、途中で hash を書き換えたチャンクを混ぜる。
        let (mut tx, rx) = duplex(64 * 1024);
        let recv = tokio::spawn(async move {
            receive_stream(
                rx,
                &std::env::temp_dir().join(format!("synergos-tx-tamper-{}", uuid::Uuid::new_v4())),
            )
            .await
        });

        // 適当なヘッダ
        let header = TransferHeader {
            transfer_id: "t".into(),
            project_id: "p".into(),
            file_id: "f".into(),
            total_size: 5,
            chunk_count: 1,
            total_hash: Blake3Hash(*blake3::hash(b"hello").as_bytes()),
        };
        write_frame(&mut tx, &FrameKind::Header(header))
            .await
            .unwrap();

        // データは "hello" だが hash は "WORLD" 版にすり替える (改竄)。
        let bad = ChunkFrame {
            index: 0,
            hash: Blake3Hash(*blake3::hash(b"WORLD").as_bytes()),
            data: b"hello".to_vec(),
        };
        write_frame(&mut tx, &FrameKind::Chunk(bad)).await.unwrap();
        drop(tx);

        let err = recv.await.unwrap();
        assert!(err.is_err());
    }
}
