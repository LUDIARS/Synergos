//! Bounded, deterministic per-block compression; identity always hashes decoded bytes.
use super::MAX_BLOCK_BYTES;
use anyhow::{ensure, Context, Result};
use flate2::{read::GzDecoder, Compression, GzBuilder};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Identity,
    Gzip,
}

pub(crate) fn encode(bytes: &[u8], encoding: Encoding) -> Result<Vec<u8>> {
    ensure!(bytes.len() <= MAX_BLOCK_BYTES, "block too large");
    if encoding == Encoding::Identity {
        return Ok(bytes.to_vec());
    }
    let mut writer = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::fast());
    writer.write_all(bytes)?;
    Ok(writer.finish()?)
}

pub(crate) fn choose(bytes: &[u8]) -> Result<(Encoding, Vec<u8>)> {
    let compressed = encode(bytes, Encoding::Gzip)?;
    // Require at least 10% savings; already-compressed data stays byte-for-byte intact.
    if compressed.len() < bytes.len().saturating_mul(9) / 10 {
        Ok((Encoding::Gzip, compressed))
    } else {
        Ok((Encoding::Identity, bytes.to_vec()))
    }
}

pub(crate) fn decode(bytes: &[u8], encoding: Encoding, expected: usize) -> Result<Vec<u8>> {
    ensure!(expected <= MAX_BLOCK_BYTES, "decoded block too large");
    let decoded = match encoding {
        Encoding::Identity => bytes.to_vec(),
        Encoding::Gzip => {
            let mut out = Vec::new();
            GzDecoder::new(bytes)
                .take(expected as u64 + 1)
                .read_to_end(&mut out)
                .context("invalid compressed block")?;
            out
        }
    };
    ensure!(decoded.len() == expected, "decoded block size mismatch");
    Ok(decoded)
}
