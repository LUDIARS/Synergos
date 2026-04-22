//! ファイルを chunk に分割し、root DAG ノードで子 CID を繋ぐ最小実装。
//!
//! - chunk ごとに `Block` を作り store に put
//! - 最後に msgpack でシリアライズした `ChunkDag` を root として put
//!
//! 受信側は root CID を引けば ChunkDag → 各 chunk CID を引ける。全 chunk を
//! concat して元ファイルを復元する。

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{Result, SynergosNetError};
use crate::types::Cid;

use super::block::Block;
use super::store::ContentStore;

/// DAG の root ノード。chunk 順序を保持する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkDag {
    /// 元ファイルの総バイト数
    pub total_size: u64,
    /// chunk ごとの CID (分割順)
    pub chunks: Vec<Cid>,
}

/// チャンク分割のパラメータ
#[derive(Debug, Clone)]
pub struct ChunkerOptions {
    /// チャンクサイズ (bytes)。小さいほど dedup しやすいが DAG が太る。
    pub chunk_size: usize,
}

impl Default for ChunkerOptions {
    fn default() -> Self {
        Self {
            chunk_size: 256 * 1024,
        }
    }
}

/// ローカルファイルを chunk に分割して store に流し込み、root CID を返す。
pub async fn add_file<S: ContentStore + ?Sized>(
    store: &S,
    path: &Path,
    opts: ChunkerOptions,
) -> Result<Cid> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; opts.chunk_size];
    let mut chunks: Vec<Cid> = Vec::new();
    let mut total_size: u64 = 0;

    loop {
        let mut filled = 0usize;
        while filled < opts.chunk_size {
            let n = file.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        let chunk_bytes = buf[..filled].to_vec();
        let block = Block::new(chunk_bytes);
        chunks.push(block.cid.clone());
        total_size += filled as u64;
        store.put(block).await?;
        if filled < opts.chunk_size {
            break;
        }
    }

    let dag = ChunkDag { total_size, chunks };
    let dag_bytes = rmp_serde::to_vec(&dag)
        .map_err(|e| SynergosNetError::Serialization(format!("dag encode: {e}")))?;
    let root = Block::new(dag_bytes);
    let root_cid = root.cid.clone();
    store.put(root).await?;
    Ok(root_cid)
}

/// root CID から ChunkDag を引き、それが指す chunk を順に store から取り、
/// 連結して出力パスへ書き出す。
pub async fn get_file<S: ContentStore + ?Sized>(
    store: &S,
    root: &Cid,
    out_path: &Path,
) -> Result<u64> {
    let root_block = store
        .get(root)
        .await?
        .ok_or_else(|| SynergosNetError::Transfer(format!("root cid missing: {root:?}")))?;
    let dag: ChunkDag = rmp_serde::from_slice(&root_block.bytes)
        .map_err(|e| SynergosNetError::Serialization(format!("dag decode: {e}")))?;

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    let mut out = tokio::fs::File::create(out_path).await?;
    let mut written: u64 = 0;

    for cid in &dag.chunks {
        let blk = store
            .get(cid)
            .await?
            .ok_or_else(|| SynergosNetError::Transfer(format!("missing chunk: {cid:?}")))?;
        if !blk.verify() {
            return Err(SynergosNetError::Transfer(format!(
                "chunk {cid:?} verification failed"
            )));
        }
        out.write_all(&blk.bytes).await?;
        written += blk.bytes.len() as u64;
    }
    out.flush().await?;

    if written != dag.total_size {
        return Err(SynergosNetError::Transfer(format!(
            "size mismatch: dag.total_size={} actual={}",
            dag.total_size, written
        )));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::store::MemoryContentStore;

    #[tokio::test]
    async fn add_then_get_roundtrip() {
        let store = MemoryContentStore::new();
        let src = std::env::temp_dir().join(format!("chunker-src-{}", uuid::Uuid::new_v4()));
        let dst = std::env::temp_dir().join(format!("chunker-dst-{}", uuid::Uuid::new_v4()));
        let payload: Vec<u8> = (0..300u32)
            .flat_map(|n| (n as u8).to_le_bytes())
            .cycle()
            .take(1_024_000)
            .collect();
        tokio::fs::write(&src, &payload).await.unwrap();

        let opts = ChunkerOptions { chunk_size: 65536 };
        let root = add_file(&store, &src, opts).await.unwrap();
        // total blocks = chunk count + 1 root
        assert!(store.len().await >= 2);

        let size = get_file(&store, &root, &dst).await.unwrap();
        assert_eq!(size as usize, payload.len());
        let got = tokio::fs::read(&dst).await.unwrap();
        assert_eq!(got, payload);

        let _ = tokio::fs::remove_file(&src).await;
        let _ = tokio::fs::remove_file(&dst).await;
    }

    #[tokio::test]
    async fn get_missing_chunk_errors() {
        let store = MemoryContentStore::new();
        // DAG を作らずに root っぽい CID を引いたら NotFound
        let bogus =
            Cid("blake3-0000000000000000000000000000000000000000000000000000000000000000".into());
        let dst = std::env::temp_dir().join(format!("chunker-miss-{}", uuid::Uuid::new_v4()));
        let err = get_file(&store, &bogus, &dst).await;
        assert!(err.is_err());
    }
}
