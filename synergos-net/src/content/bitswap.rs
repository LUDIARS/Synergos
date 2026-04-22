//! Bitswap-lite v2: CID 単位で「欲しい」/「持ってる?」を宣言して block を引き寄せる。
//!
//! v1 の「1 ストリーム = 1 Want = 1 Block」を保ったまま、以下を追加した:
//!
//! - `BitswapRequest::WantList { cids, want_have }`: 複数 CID を 1 リクエストで要求。
//!   `want_have=false` なら各 CID について `Block` または `NotFound` を順に返し、
//!   `want_have=true` なら `Have` / `DontHave` だけを返す (メタ問い合わせ)。
//! - `BitswapRequest::Cancel { cids }`: 送信者が不要になった WANT を通知する。
//!   サーバ側はレスポンスを返さずストリームを閉じる (単一往復向けの簡易実装)。
//! - `BitswapResponse::Have` / `DontHave`: want_have クエリへの応答。
//!
//! QUIC の bidi ストリーム上のワイヤフォーマット:
//!
//! ```text
//! client → server: magic(4) | len(4) | body (msgpack(BitswapRequest))
//! server → client: magic(4) | [len(4) | body (msgpack(BitswapResponse))] ... EOF
//! ```
//!
//! レスポンスは "フレーム列 + finish()" 形式で、クライアントは EOF まで
//! フレームを読み進める。これにより WantList では要求順に 1 フレームずつ
//! Block / NotFound (または Have / DontHave) が届く。
//!
//! `Want { cid }` 単発は WantList(1) と同等に扱われ、レスポンスも 1 フレーム。
//! v1 のエンコードと互換性があるので、旧 `request_block` 呼び出しはそのまま動く。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SynergosNetError};
use crate::types::Cid;

use super::block::Block;
use super::store::ContentStore;

pub const BITSWAP_STREAM_MAGIC: &[u8; 4] = b"BSW1";
pub const MAX_BITSWAP_FRAME: usize = 16 * 1024 * 1024; // 16 MiB / block
/// 1 WantList あたりの CID 上限。サーバ側の DoS 耐性を考慮した保守的な値。
pub const MAX_WANTLIST_LEN: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BitswapRequest {
    /// 単一 CID を要求。v1 互換。サーバは `Block` か `NotFound` を 1 フレーム返す。
    Want { cid: Cid },
    /// 複数 CID をまとめて要求。
    /// `want_have=false` なら `Block`/`NotFound`、`want_have=true` なら `Have`/`DontHave`
    /// をリクエスト順に 1 フレームずつ返す。
    WantList {
        cids: Vec<Cid>,
        /// true にすると Block 本体ではなくメタ情報 (Have / DontHave) だけ返す。
        #[serde(default)]
        want_have: bool,
    },
    /// 以前送った WANT をキャンセル。応答は返らない (ストリームは即クローズ)。
    Cancel { cids: Vec<Cid> },
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
    /// want_have=true クエリに対して「持っている」と宣言する応答。
    Have {
        cid: Cid,
    },
    /// want_have=true クエリに対して「持っていない」と宣言する応答。
    DontHave {
        cid: Cid,
    },
}

// ─────────────────────── フレーム I/O helpers ───────────────────────
//
// QUIC stream 上で msgpack ボディを `u32 len + body` 形式で読み書きする。
// 応答側は複数フレームを書き、最後に `send.finish()` を呼ぶ。読み手は
// EOF で終端を検知する。
async fn write_frame(send: &mut quinn::SendStream, body: &[u8], what: &str) -> Result<()> {
    let len = (body.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write {what} len: {e}")))?;
    send.write_all(body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write {what} body: {e}")))?;
    Ok(())
}

async fn read_frame_body(recv: &mut quinn::RecvStream, what: &str) -> Result<Option<Vec<u8>>> {
    // len を 4 バイト読む。EOF なら None を返し、途中 EOF はエラー。
    let mut len_buf = [0u8; 4];
    let mut read = 0;
    while read < 4 {
        match recv.read(&mut len_buf[read..]).await {
            Ok(Some(n)) if n > 0 => read += n,
            Ok(Some(_)) | Ok(None) => {
                if read == 0 {
                    return Ok(None);
                }
                return Err(SynergosNetError::Quic(format!(
                    "bitswap truncated {what} len (read {read}/4 before EOF)"
                )));
            }
            Err(e) => {
                return Err(SynergosNetError::Quic(format!(
                    "bitswap read {what} len: {e}"
                )));
            }
        }
    }
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if frame_len > MAX_BITSWAP_FRAME {
        return Err(SynergosNetError::Serialization(format!(
            "bitswap {what} frame too large: {frame_len}"
        )));
    }
    let mut body = vec![0u8; frame_len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read {what} body: {e}")))?;
    Ok(Some(body))
}

// ─────────────────────── Client: single-block API (v1 互換) ───────────────────────

/// クライアント側: bidi stream を受け取り、WANT を送って BLOCK/NotFound を
/// 受け取る 1 往復。magic は呼び出し側が事前に書き込まない前提で本関数が付与する。
pub async fn request_block(
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    cid: &Cid,
) -> Result<BitswapResponse> {
    let mut results = request_many(send, recv, std::slice::from_ref(cid), false).await?;
    results
        .pop()
        .ok_or_else(|| SynergosNetError::Serialization("bitswap empty response".into()))
}

// ─────────────────────── Client: WantList / HAVE API ───────────────────────

/// 複数 CID を 1 ストリームで要求する。戻り値は要求順に並んだレスポンスフレーム。
/// `want_have=false` なら各要素は `Block` か `NotFound`、`want_have=true` なら
/// `Have` か `DontHave`。
pub async fn request_many(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    cids: &[Cid],
    want_have: bool,
) -> Result<Vec<BitswapResponse>> {
    if cids.is_empty() {
        return Ok(Vec::new());
    }
    if cids.len() > MAX_WANTLIST_LEN {
        return Err(SynergosNetError::Serialization(format!(
            "bitswap wantlist too large: {} > {MAX_WANTLIST_LEN}",
            cids.len()
        )));
    }

    // 要求送信
    send.write_all(BITSWAP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write magic: {e}")))?;
    let req = if cids.len() == 1 && !want_have {
        // v1 互換のため単発は Want で送る (旧サーバとも会話できる)。
        BitswapRequest::Want {
            cid: cids[0].clone(),
        }
    } else {
        BitswapRequest::WantList {
            cids: cids.to_vec(),
            want_have,
        }
    };
    let body = rmp_serde::to_vec(&req)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap req encode: {e}")))?;
    write_frame(&mut send, &body, "request").await?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("bitswap finish: {e}")))?;

    // 応答: magic を 1 度だけ読み、その後 frame 列を EOF まで読む。
    let mut magic = [0u8; 4];
    recv.read_exact(&mut magic)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap read magic: {e}")))?;
    if &magic != BITSWAP_STREAM_MAGIC {
        return Err(SynergosNetError::Serialization(
            "bitswap response magic mismatch".into(),
        ));
    }

    let mut out = Vec::with_capacity(cids.len());
    while let Some(body) = read_frame_body(&mut recv, "response").await? {
        let resp: BitswapResponse = rmp_serde::from_slice(&body)
            .map_err(|e| SynergosNetError::Serialization(format!("bitswap resp decode: {e}")))?;
        out.push(resp);
    }
    Ok(out)
}

/// Cancel リクエストを送るだけの片道呼び出し。サーバは応答を返さずに close する。
/// 呼び出し側は `recv` を drop すればよい。
pub async fn send_cancel(mut send: quinn::SendStream, cids: &[Cid]) -> Result<()> {
    send.write_all(BITSWAP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write magic: {e}")))?;
    let req = BitswapRequest::Cancel {
        cids: cids.to_vec(),
    };
    let body = rmp_serde::to_vec(&req)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap cancel encode: {e}")))?;
    write_frame(&mut send, &body, "cancel").await?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("bitswap finish cancel: {e}")))?;
    Ok(())
}

// ─────────────────────── Server: 1 リクエスト → N レスポンス ───────────────────────

/// サーバ側: magic を事前に読み終えた `recv` + store を受け取り、1 リクエストを処理。
/// WantList なら該当数ぶんレスポンスフレームを書き、最後に finish() を呼ぶ。
/// Cancel は何も書かずに close する。
pub async fn handle_bitswap_stream<S: ContentStore + ?Sized>(
    store: Arc<S>,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) -> Result<()> {
    let body = read_frame_body(&mut recv, "request")
        .await?
        .ok_or_else(|| SynergosNetError::Serialization("bitswap empty request".into()))?;

    let req: BitswapRequest = rmp_serde::from_slice(&body)
        .map_err(|e| SynergosNetError::Serialization(format!("bitswap req decode: {e}")))?;

    // Cancel は応答無し
    if matches!(req, BitswapRequest::Cancel { .. }) {
        tracing::debug!("bitswap: Cancel received; closing stream");
        send.finish()
            .map_err(|e| SynergosNetError::Quic(format!("bitswap finish cancel: {e}")))?;
        return Ok(());
    }

    // 応答 magic を先頭に 1 度だけ書く
    send.write_all(BITSWAP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("bitswap write resp magic: {e}")))?;

    // リクエスト → フレーム列へ展開
    let (cids, want_have): (Vec<Cid>, bool) = match req {
        BitswapRequest::Want { cid } => (vec![cid], false),
        BitswapRequest::WantList { cids, want_have } => (cids, want_have),
        BitswapRequest::Cancel { .. } => unreachable!(),
    };

    if cids.len() > MAX_WANTLIST_LEN {
        return Err(SynergosNetError::Serialization(format!(
            "bitswap wantlist too large: {} > {MAX_WANTLIST_LEN}",
            cids.len()
        )));
    }

    for cid in cids {
        let resp = if want_have {
            if store.has(&cid).await {
                BitswapResponse::Have { cid }
            } else {
                BitswapResponse::DontHave { cid }
            }
        } else {
            match store.get(&cid).await? {
                Some(Block { cid: ret, bytes }) => BitswapResponse::Block { cid: ret, bytes },
                None => BitswapResponse::NotFound { cid },
            }
        };
        let frame = rmp_serde::to_vec(&resp)
            .map_err(|e| SynergosNetError::Serialization(format!("bitswap resp encode: {e}")))?;
        write_frame(&mut send, &frame, "response").await?;
    }

    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("bitswap finish resp: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_want_roundtrip_serde() {
        let r = BitswapRequest::Want {
            cid: Cid("blake3-abc".into()),
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        let back: BitswapRequest = rmp_serde::from_slice(&b).unwrap();
        match back {
            BitswapRequest::Want { cid } => assert_eq!(cid.0, "blake3-abc"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn request_wantlist_roundtrip_serde() {
        let r = BitswapRequest::WantList {
            cids: vec![Cid("blake3-a".into()), Cid("blake3-b".into())],
            want_have: true,
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        let back: BitswapRequest = rmp_serde::from_slice(&b).unwrap();
        match back {
            BitswapRequest::WantList { cids, want_have } => {
                assert_eq!(cids.len(), 2);
                assert_eq!(cids[0].0, "blake3-a");
                assert!(want_have);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn request_cancel_roundtrip_serde() {
        let r = BitswapRequest::Cancel {
            cids: vec![Cid("blake3-x".into())],
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        let back: BitswapRequest = rmp_serde::from_slice(&b).unwrap();
        assert!(matches!(back, BitswapRequest::Cancel { .. }));
    }

    #[test]
    fn response_roundtrip_block() {
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

    #[test]
    fn response_roundtrip_have_donthave() {
        let r = BitswapResponse::Have {
            cid: Cid("blake3-x".into()),
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        assert!(matches!(
            rmp_serde::from_slice::<BitswapResponse>(&b).unwrap(),
            BitswapResponse::Have { .. }
        ));

        let r = BitswapResponse::DontHave {
            cid: Cid("blake3-y".into()),
        };
        let b = rmp_serde::to_vec(&r).unwrap();
        assert!(matches!(
            rmp_serde::from_slice::<BitswapResponse>(&b).unwrap(),
            BitswapResponse::DontHave { .. }
        ));
    }
}
