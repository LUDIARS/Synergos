//! Bitswap-lite: CID 単位で「欲しい」と宣言して block を引き寄せる最小プロトコル。
//!
//! QUIC の bidi ストリーム上で、以下の 1 リクエスト = 1 往復を送る:
//!
//! ```text
//! client → server: BitswapRequest::Want { cid }
//! server → client: BitswapResponse::Block { cid, bytes }
//!                   | BitswapResponse::NotFound { cid }
//! ```
//!
//! 複数 CID を並列で引きたい場合は複数のストリームを張る。実際の IPFS
//! Bitswap (WantList / CancelList / HAVE / BLOCK) はより複雑だが、
//! Synergos ではまず「単純な block 往復」を動かしてから拡張する。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SynergosNetError};
use crate::types::Cid;

use super::block::Block;
use super::store::ContentStore;

pub const BITSWAP_STREAM_MAGIC: &[u8; 4] = b"BSW1";
pub const MAX_BITSWAP_FRAME: usize = 16 * 1024 * 1024; // 16 MiB / block

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BitswapRequest {
    Want { cid: Cid },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BitswapResponse {
    Block {
        cid: Cid,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    NotFound {
        cid: Cid,
    },
}

/// クライアント側: bidi stream を受け取り、WANT を送って BLOCK/NotFound を
/// 受け取る 1 往復。magic は呼び出し側が事前に書き込まない前提で本関数が付与する。
pub async fn request_block(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    cid: &Cid,
) -> Result<BitswapResponse> {
    send.write_all(BITSWAP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write magic: {e}")))?;

    let req = BitswapRequest::Want { cid: cid.clone() };
    let body = rmp_serde::to_vec(&req)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap req encode: {e}")))?;
    let len = (body.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write len: {e}")))?;
    send.write_all(&body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write body: {e}")))?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("bitswap finish: {e}")))?;

    // 応答
    let mut magic = [0u8; 4];
    recv.read_exact(&mut magic)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read magic: {e}")))?;
    if &magic != BITSWAP_STREAM_MAGIC {
        return Err(SynergosNetError::Serialization(
            "bitswap response magic mismatch".into(),
        ));
    }
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read len: {e}")))?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    if resp_len > MAX_BITSWAP_FRAME {
        return Err(SynergosNetError::Serialization(format!(
            "bitswap response too large: {resp_len}"
        )));
    }
    let mut body = vec![0u8; resp_len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read body: {e}")))?;
    let resp: BitswapResponse = rmp_serde::from_slice(&body)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap resp decode: {e}")))?;
    Ok(resp)
}

/// サーバ側: magic を事前に読み終えた recv + store を受け取り、1 リクエストを処理。
/// Daemon のストリームディスパッチャから呼ばれる想定で、本関数は magic を消費しない。
pub async fn handle_bitswap_stream<S: ContentStore + ?Sized>(
    store: Arc<S>,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) -> Result<()> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read req len: {e}")))?;
    let req_len = u32::from_be_bytes(len_buf) as usize;
    if req_len > MAX_BITSWAP_FRAME {
        return Err(SynergosNetError::Serialization(format!(
            "bitswap request too large: {req_len}"
        )));
    }
    let mut body = vec![0u8; req_len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read req body: {e}")))?;

    let req: BitswapRequest = rmp_serde::from_slice(&body)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap req decode: {e}")))?;

    let resp = match req {
        BitswapRequest::Want { cid } => match store.get(&cid).await? {
            Some(Block { cid: ret, bytes }) => BitswapResponse::Block { cid: ret, bytes },
            None => BitswapResponse::NotFound { cid },
        },
    };

    let resp_bytes = rmp_serde::to_vec(&resp)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap resp encode: {e}")))?;
    send.write_all(BITSWAP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write resp magic: {e}")))?;
    let len = (resp_bytes.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write resp len: {e}")))?;
    send.write_all(&resp_bytes)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write resp body: {e}")))?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("bitswap finish resp: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_serde() {
        let r = BitswapRequest::Want {
            cid: Cid("blake3-abc".into()),
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        let back: BitswapRequest = rmp_serde::from_slice(&b).unwrap();
        match back {
            BitswapRequest::Want { cid } => assert_eq!(cid.0, "blake3-abc"),
        }
    }

    #[test]
    fn response_roundtrip_serde() {
        let r = BitswapResponse::Block {
            cid: Cid("blake3-x".into()),
            bytes: vec![1, 2, 3],
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        let back: BitswapResponse = rmp_serde::from_slice(&b).unwrap();
        match back {
            BitswapResponse::Block { cid, bytes } => {
                assert_eq!(cid.0, "blake3-x");
                assert_eq!(bytes, vec![1, 2, 3]);
            }
            _ => panic!("wrong variant"),
        }
    }
}
