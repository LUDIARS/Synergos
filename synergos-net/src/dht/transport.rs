//! QUIC を実体としたい `DhtTransport` 実装と、サーバ側の受信ループ。
//!
//! 前提条件:
//! - 相手ピアとは既に `QuicManager::connect` 済み (= ピア真性認証済み)
//! - DHT RPC は独立した双方向ストリームを 1 本使う。1 ストリーム = 1 ラウンド
//!
//! フレーミングは `rpc::encode` / `rpc::decode` のペア。

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{Result, SynergosNetError};
use crate::quic::{QuicManager, StreamType};
use crate::types::PeerId;

use super::node::DhtNode;
use super::rpc::{self, DhtRequest, DhtResponse, DhtTransport, MAX_FRAME_SIZE};

/// プロトコルマジック: DHT ストリームのヘッダ先頭に置いて識別する。
pub const DHT_STREAM_MAGIC: &[u8; 4] = b"DHT1";

/// QUIC バックエンド。`QuicManager` が相手ピアへの既存コネクションを
/// 保持している必要がある。
pub struct QuicDhtTransport {
    quic: Arc<QuicManager>,
}

impl QuicDhtTransport {
    pub fn new(quic: Arc<QuicManager>) -> Self {
        Self { quic }
    }
}

#[async_trait]
impl DhtTransport for QuicDhtTransport {
    async fn request(&self, peer: &PeerId, req: &DhtRequest) -> Result<DhtResponse> {
        let (mut send, mut recv) = self.quic.open_stream(peer, StreamType::Control).await?;

        // ヘッダ + フレーム
        send.write_all(DHT_STREAM_MAGIC).await.map_err(to_err)?;
        let payload = rpc::encode(req)?;
        send.write_all(&payload).await.map_err(to_err)?;
        send.finish()
            .map_err(|e| SynergosNetError::Quic(format!("finish: {e}")))?;

        // 応答を受け取る
        let mut magic = [0u8; 4];
        recv.read_exact(&mut magic).await.map_err(to_err)?;
        if &magic != DHT_STREAM_MAGIC {
            return Err(SynergosNetError::Serialization(
                "DHT response magic mismatch".into(),
            ));
        }

        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await.map_err(to_err)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_FRAME_SIZE {
            return Err(SynergosNetError::Serialization(format!(
                "DHT response too large: {len}"
            )));
        }
        let mut body = vec![0u8; len];
        recv.read_exact(&mut body).await.map_err(to_err)?;

        // rpc::decode はヘッダ付きを期待するので合成
        let mut full = Vec::with_capacity(4 + len);
        full.extend_from_slice(&len_buf);
        full.extend_from_slice(&body);
        rpc::decode(&full)
    }
}

/// 1 本の DHT ストリームを処理する: 長さ付きリクエスト読込 →
/// `DhtNode::handle_request` → 応答送信。
///
/// 呼び出し側 (ストリームディスパッチャ) が magic (`DHT1`) を既に消費済みの
/// 前提で入る。`send.write_all(DHT_STREAM_MAGIC)` で応答側の magic は
/// このハンドラが付与する。
pub async fn handle_dht_stream(
    dht: Arc<DhtNode>,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
) -> Result<()> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await.map_err(to_err)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(SynergosNetError::Serialization(format!(
            "DHT request too large: {len}"
        )));
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(to_err)?;

    let mut full = Vec::with_capacity(4 + len);
    full.extend_from_slice(&len_buf);
    full.extend_from_slice(&body);
    let req: DhtRequest = rpc::decode(&full)?;

    let resp = dht.handle_request(req).await;

    send.write_all(DHT_STREAM_MAGIC).await.map_err(to_err)?;
    let out = rpc::encode(&resp)?;
    send.write_all(&out).await.map_err(to_err)?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("finish: {e}")))?;
    Ok(())
}

fn to_err<E: std::fmt::Display>(e: E) -> SynergosNetError {
    SynergosNetError::Quic(format!("DHT IO: {e}"))
}
