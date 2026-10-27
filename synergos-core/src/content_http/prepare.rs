//! Streaming preparation: preserve ChunkDag format, cache each distinct block once.
use super::{codec, BlockCache, BlockDescription, Encoding, Manifest, MAX_BLOCK_BYTES, MAX_CHUNKS};
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use synergos_net::content::{Block, ChunkDag};
use tokio::io::AsyncReadExt;

pub async fn prepare(cache: &BlockCache, source: &Path, chunk_bytes: usize) -> Result<Manifest> {
    ensure!(
        (64 * 1024..=8 * 1024 * 1024).contains(&chunk_bytes),
        "chunk size must be 64 KiB to 8 MiB"
    );
    let mut file = tokio::fs::File::open(source).await?;
    let before = file.metadata().await?;
    ensure!(
        before.is_file() && before.len() <= chunk_bytes as u64 * MAX_CHUNKS as u64,
        "input exceeds chunk-count limit or is not a regular file"
    );
    let mut buffer = vec![0; chunk_bytes];
    let mut chunks = Vec::new();
    let mut descriptions = BTreeMap::new();
    let mut total_size = 0u64;
    loop {
        let mut used = 0;
        while used < buffer.len() {
            let count = file.read(&mut buffer[used..]).await?;
            if count == 0 {
                break;
            }
            used += count;
        }
        if used == 0 {
            break;
        }
        ensure!(chunks.len() < MAX_CHUNKS, "input grew beyond chunk limit");
        let block = Block::new(buffer[..used].to_vec());
        chunks.push(block.cid.clone());
        total_size += used as u64;
        if !descriptions.contains_key(&block.cid.0) {
            let (encoding, wire) = codec::choose(&block.bytes)?;
            descriptions.insert(block.cid.0.clone(), describe(&block, encoding, &wire));
            cache.save(block).await?;
        }
    }
    let after = file.metadata().await?;
    ensure!(
        before.len() == after.len()
            && before.modified()? == after.modified()?
            && total_size == before.len(),
        "source changed while preparing; retry from a stable file"
    );
    let root_block = Block::new(rmp_serde::to_vec(&ChunkDag { total_size, chunks })?);
    ensure!(
        root_block.bytes.len() <= MAX_BLOCK_BYTES,
        "root block too large"
    );
    let root = root_block.cid.0.clone();
    descriptions.insert(
        root.clone(),
        describe(&root_block, Encoding::Identity, &root_block.bytes),
    );
    cache.save(root_block).await?;
    let manifest = Manifest {
        version: 1,
        root,
        file_bytes: total_size,
        blocks: descriptions.into_values().collect(),
    };
    manifest.validate(&manifest.root)?;
    Ok(manifest)
}

fn describe(block: &Block, encoding: Encoding, wire: &[u8]) -> BlockDescription {
    BlockDescription {
        cid: block.cid.0.clone(),
        decoded_bytes: block.bytes.len(),
        wire_bytes: wire.len(),
        wire_sha256: wire_digest(wire),
        encoding,
    }
}

pub(crate) fn wire_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
