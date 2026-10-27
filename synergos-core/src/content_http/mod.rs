//! Capability-only HTTP adapter for the legacy BLAKE3 ChunkDag format.
//! @implements SPEC-SY-content-http
//! No membership issuer, listener, QUIC connection or custody administrator credential.
mod assembly;
mod cache;
pub mod cli;
mod codec;
mod http;
mod manifest;
#[cfg(test)]
mod network_tests;
mod prepare;
#[cfg(test)]
mod tests;
mod transfer;

pub use cache::BlockCache;
pub use codec::Encoding;
pub use manifest::{BlockDescription, Manifest, TransferGrant, TransferPlan};
pub use prepare::prepare;
pub use transfer::{pull, push, TransferOptions, TransferReport};

pub const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHUNKS: usize = 131_072;
pub const DEFAULT_CHUNK_BYTES: usize = 4 * 1024 * 1024;

pub(crate) fn validate_cid(cid: &str) -> anyhow::Result<()> {
    let digest = cid.strip_prefix("blake3-").unwrap_or("");
    anyhow::ensure!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid block CID"
    );
    Ok(())
}
