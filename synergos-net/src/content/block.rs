//! Block = `(Cid, bytes)`。CID は本バージョンでは `blake3-<hex64>` で固定。

use crate::types::Cid;

/// バイト列から CID を導出する。
/// `blake3` の 256-bit hash を hex にして `blake3-<64 hex>` の形にする。
pub fn cid_for(bytes: &[u8]) -> Cid {
    let hash = blake3::hash(bytes);
    Cid(format!("blake3-{}", hex::encode(hash.as_bytes())))
}

/// Content-addressed な block。`cid` は `bytes` から決定的に派生する。
#[derive(Debug, Clone)]
pub struct Block {
    pub cid: Cid,
    pub bytes: Vec<u8>,
}

impl Block {
    pub fn new(bytes: Vec<u8>) -> Self {
        let cid = cid_for(&bytes);
        Self { cid, bytes }
    }

    pub fn verify(&self) -> bool {
        cid_for(&self.bytes) == self.cid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cid_is_deterministic() {
        let a = cid_for(b"hello");
        let b = cid_for(b"hello");
        assert_eq!(a, b);
        assert!(a.0.starts_with("blake3-"));
        assert_eq!(a.0.len(), "blake3-".len() + 64);
    }

    #[test]
    fn different_bytes_produce_different_cid() {
        assert_ne!(cid_for(b"x"), cid_for(b"y"));
    }

    #[test]
    fn block_verify_succeeds_when_untampered() {
        let b = Block::new(b"data".to_vec());
        assert!(b.verify());
    }

    #[test]
    fn block_verify_fails_when_tampered() {
        let mut b = Block::new(b"data".to_vec());
        b.bytes[0] ^= 1;
        assert!(!b.verify());
    }
}
