//! Gossip mesh の QUIC 上 fan-out 実装。
//!
//! `GossipNode::publish` は従来ローカル broadcast channel にしか流れず、
//! 2 ノード間の伝播は上位 (Daemon) に委ねられていた。本モジュールは以下を
//! 提供する:
//!
//! - `OutboundGossip` — publish 時に出力される (topic, signed, peers) の三つ組
//! - `GossipWireMessage` — QUIC 上で送る topic + signed の wire 形式
//! - `send_gossip` / `handle_gossip_stream` — QUIC bidi ストリームでの入出力
//! - ストリームマジック `GSP1` — Daemon のディスパッチャが振り分けに使う
//!
//! 受信側は `GossipNode::on_signed_message_received` を呼び、ローカル購読者に
//! deliver する (その先で FileWant → Exchange::handle_file_want などに流れる)。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SynergosNetError};
use crate::gossip::{GossipNode, SignedGossipMessage};
use crate::types::{PeerId, TopicId};

/// Gossip ストリーム識別マジック。Daemon のディスパッチャが先頭 4 byte を
/// 読んで DHT1 / TXFR / GSP1 を振り分ける。
pub const GOSSIP_STREAM_MAGIC: &[u8; 4] = b"GSP1";

/// 1 リクエスト (= 1 メッセージ) あたりの最大サイズ (256 KiB)。
/// 実際の GossipMessage は PeerStatus / FileOffer 等で 1 KiB もいかないが、
/// 将来の拡張用バッファ込み。
pub const MAX_GOSSIP_FRAME: usize = 256 * 1024;

/// QUIC 上で送る gossip メッセージの wire 形式。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipWireMessage {
    pub topic: TopicId,
    pub signed: SignedGossipMessage,
}

/// publish 時に Daemon の送信タスクへ流す三つ組。
#[derive(Debug, Clone)]
pub struct OutboundGossip {
    pub topic: TopicId,
    pub signed: SignedGossipMessage,
    /// fan-out 先のメッシュピア (publish() が返したリスト)
    pub peers: Vec<PeerId>,
}

/// 指定の QUIC bidi send half に 1 つの `GossipWireMessage` を書き込む。
/// 受信側は `handle_gossip_stream` で対になる処理を行う。
pub async fn send_gossip(mut send: quinn::SendStream, msg: &GossipWireMessage) -> Result<()> {
    let body = rmp_serde::to_vec(msg)
        .map_err(|e| SynergosNetError::Serialization(format!("gossip encode: {e}")))?;
    if body.len() > MAX_GOSSIP_FRAME {
        return Err(SynergosNetError::Serialization(format!(
            "gossip frame too large: {} > {}",
            body.len(),
            MAX_GOSSIP_FRAME
        )));
    }

    send.write_all(GOSSIP_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("gossip magic: {e}")))?;
    let len = (body.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("gossip len: {e}")))?;
    send.write_all(&body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("gossip body: {e}")))?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("gossip finish: {e}")))?;
    Ok(())
}

/// 受信側: magic は呼び出し側で消費済みの前提 (Daemon のディスパッチャが
/// `GSP1` を見て本関数に回す)。長さ + 本体を読み、`GossipNode` に流す。
pub async fn handle_gossip_stream(
    gossip: Arc<GossipNode>,
    mut recv: quinn::RecvStream,
    from: PeerId,
) -> Result<()> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("gossip read len: {e}")))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_GOSSIP_FRAME {
        return Err(SynergosNetError::Serialization(format!(
            "gossip frame too large: {len}"
        )));
    }

    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("gossip read body: {e}")))?;

    let wire: GossipWireMessage = rmp_serde::from_slice(&body)
        .map_err(|e| SynergosNetError::Serialization(format!("gossip decode: {e}")))?;

    // 署名検証 + ローカル deliver。deliver の後で broadcast channel 経由で
    // 購読者 (Exchange, Presence 等) に届く。
    gossip.on_signed_message_received(&wire.topic, wire.signed, &from);
    Ok(())
}
