//! Validate a legacy ChunkDag closure and publish only a complete verified file.
use super::{cache::TemporaryFile, BlockCache, Manifest, MAX_CHUNKS};
use anyhow::{ensure, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};
use synergos_net::content::ChunkDag;
use tokio::io::AsyncWriteExt;

pub(crate) async fn validated_dag(cache: &BlockCache, manifest: &Manifest) -> Result<ChunkDag> {
    let root = cache
        .load(&manifest.root)
        .await?
        .context("verified root missing")?;
    let dag: ChunkDag = rmp_serde::from_slice(&root.bytes).context("invalid ChunkDag root")?;
    ensure!(
        dag.chunks.len() <= MAX_CHUNKS && dag.total_size == manifest.file_bytes,
        "DAG size/count mismatch"
    );
    let descriptions: HashMap<_, _> = manifest
        .blocks
        .iter()
        .map(|b| (b.cid.as_str(), b))
        .collect();
    let mut used = HashSet::from([manifest.root.as_str()]);
    let mut size = 0u64;
    for cid in &dag.chunks {
        super::validate_cid(&cid.0)?;
        ensure!(cid.0 != manifest.root, "recursive root is forbidden");
        let block = descriptions
            .get(cid.0.as_str())
            .context("DAG block description missing")?;
        ensure!(block.decoded_bytes > 0, "empty data chunk");
        size = size
            .checked_add(block.decoded_bytes as u64)
            .context("DAG size overflow")?;
        used.insert(cid.0.as_str());
    }
    ensure!(
        size == dag.total_size && used.len() == descriptions.len(),
        "manifest is not the exact DAG closure"
    );
    Ok(dag)
}

pub(crate) async fn assemble(cache: &BlockCache, dag: &ChunkDag, destination: &Path) -> Result<()> {
    ensure!(
        tokio::fs::symlink_metadata(destination)
            .await
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "destination already exists or cannot be inspected"
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let temp = TemporaryFile(parent.join(format!(".synergos-{}.part", uuid::Uuid::new_v4())));
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp.0)
        .await?;
    let mut written = 0u64;
    for cid in &dag.chunks {
        let block = cache
            .load(&cid.0)
            .await?
            .context("verified data chunk missing")?;
        output.write_all(&block.bytes).await?;
        written = written
            .checked_add(block.bytes.len() as u64)
            .context("output size overflow")?;
    }
    ensure!(written == dag.total_size, "assembled size mismatch");
    output.sync_all().await?;
    drop(output);
    // A same-directory hard link atomically publishes without replacing a raced-in target.
    // It links a disposable assembly file, NEVER a block or historical object to user data.
    tokio::fs::hard_link(&temp.0, destination)
        .await
        .context("cannot publish output without replacing an existing file")?;
    Ok(())
}
