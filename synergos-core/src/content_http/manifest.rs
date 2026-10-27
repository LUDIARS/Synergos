//! Public block descriptions and short-lived transfer grants are separate artifacts.
use super::{validate_cid, Encoding, MAX_BLOCK_BYTES, MAX_CHUNKS};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDescription {
    pub cid: String,
    pub decoded_bytes: usize,
    pub wire_bytes: usize,
    pub wire_sha256: String,
    pub encoding: Encoding,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub root: String,
    pub file_bytes: u64,
    /// Unique blocks, including the uncompressed ChunkDag root. No URLs or credentials.
    pub blocks: Vec<BlockDescription>,
}

impl Manifest {
    pub fn validate(&self, expected_root: &str) -> Result<()> {
        validate_cid(expected_root)?;
        ensure!(
            self.version == 1 && self.root == expected_root,
            "unexpected manifest version or root"
        );
        ensure!(
            !self.blocks.is_empty() && self.blocks.len() <= MAX_CHUNKS + 1,
            "invalid block count"
        );
        let mut seen = HashSet::new();
        for block in &self.blocks {
            validate_cid(&block.cid)?;
            ensure!(seen.insert(&block.cid), "duplicate block description");
            ensure!(
                block.decoded_bytes <= MAX_BLOCK_BYTES && block.wire_bytes <= MAX_BLOCK_BYTES,
                "block exceeds limit"
            );
            ensure!(
                block.wire_sha256.len() == 64
                    && block
                        .wire_sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid wire digest"
            );
        }
        ensure!(seen.contains(&self.root), "root description missing");
        Ok(())
    }
}

// Deliberately no Debug: URLs and headers can contain short-lived capabilities.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferGrant {
    pub cid: String,
    pub get_url: String,
    pub put_url: Option<String>,
    #[serde(default)]
    pub get_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub put_headers: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPlan {
    pub manifest: Manifest,
    pub grants: Vec<TransferGrant>,
}
