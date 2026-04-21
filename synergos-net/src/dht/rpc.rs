//! DHT FIND_NODE RPC (S-BASE の iterative Kademlia lookup に必要な
//! 最小限のメッセージ型 + 汎用トランスポートトレイト).
//!
//! トランスポート自体は QUIC に固定せず、トレイト `DhtTransport` を介して
//! 注入できるようにしてある。これにより:
//!
//! - 単体テストで in-memory のモックトランスポートを使える
//! - 将来 TLS over WebSocket 経由の relay lookup を差し込むのも容易
//!
//! フレーミングは `[u32 length][msgpack payload]` の単純形。リクエストと
//! レスポンスは 1 ラウンド = 1 ストリームで完結する (長時間のフォローアップは無い)。

use std::net::SocketAddrV6;

use async_trait::async_trait;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SynergosNetError};
use crate::types::{PeerId, Route};

/// DHT レコード (PeerRecord の over-the-wire 表現)。
/// `Instant` は序列化できないので published_at は省略し、TTL だけ伝える。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerRecordDto {
    pub peer_id: PeerId,
    pub display_name: String,
    pub endpoints: Vec<RouteDto>,
    pub active_projects: Vec<String>,
    pub ttl_secs: u64,
}

/// Route のシリアライズ用。`Route` は現状 serde 実装済みだが、将来の
/// network 表現のブレから隔離するため DTO を噛ませておく。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RouteDto {
    Direct {
        addr: SocketAddrV6,
        fqdn: Option<String>,
    },
    Tunnel {
        tunnel_id: String,
        hostname: String,
    },
    Relay {
        server_url: String,
        room_id: String,
    },
}

impl From<&Route> for RouteDto {
    fn from(r: &Route) -> Self {
        match r {
            Route::Direct { addr, fqdn } => RouteDto::Direct {
                addr: *addr,
                fqdn: fqdn.clone(),
            },
            Route::Tunnel {
                tunnel_id,
                hostname,
            } => RouteDto::Tunnel {
                tunnel_id: tunnel_id.clone(),
                hostname: hostname.clone(),
            },
            Route::Relay {
                server_url,
                room_id,
            } => RouteDto::Relay {
                server_url: server_url.clone(),
                room_id: room_id.clone(),
            },
        }
    }
}

impl From<&RouteDto> for Route {
    fn from(r: &RouteDto) -> Self {
        match r {
            RouteDto::Direct { addr, fqdn } => Route::Direct {
                addr: *addr,
                fqdn: fqdn.clone(),
            },
            RouteDto::Tunnel {
                tunnel_id,
                hostname,
            } => Route::Tunnel {
                tunnel_id: tunnel_id.clone(),
                hostname: hostname.clone(),
            },
            RouteDto::Relay {
                server_url,
                room_id,
            } => Route::Relay {
                server_url: server_url.clone(),
                room_id: room_id.clone(),
            },
        }
    }
}

/// DHT RPC リクエスト
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DhtRequest {
    /// 近接ノードの返却を要求
    FindNode { target: PeerId },
    /// プロジェクトに参加しているピアを要求
    FindProjectPeers { project_id: String },
    /// 自分のレコードを announce
    Announce { record: PeerRecordDto },
    /// ヘルスチェック
    Ping,
}

/// DHT RPC レスポンス
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DhtResponse {
    /// FIND_NODE への応答 (closest k 件 + もしあれば exact match を含む)
    Nodes {
        /// 応答ノードの exact hit (responder 自身が target レコードを持っていた場合)
        exact: Option<PeerRecordDto>,
        /// target に近い closest k 件
        closest: Vec<PeerRecordDto>,
    },
    /// FindProjectPeers への応答
    ProjectPeers { peers: Vec<PeerRecordDto> },
    /// Announce の ack
    AnnounceAck,
    /// Ping への pong
    Pong,
    /// エラー
    Error { message: String },
}

/// RPC ラウンドをトランスポートを介して往復させる trait。
/// QUIC / 相関テストで差し替え可能にする。
#[async_trait]
pub trait DhtTransport: Send + Sync {
    /// 指定ピアに `req` を送り、1 回の応答を受け取る。
    async fn request(&self, peer: &PeerId, req: &DhtRequest) -> Result<DhtResponse>;
}

/// バイト列 → `DhtRequest` のフレーム復号。ヘッダ `[u32 big-endian length]` の後ろに msgpack ペイロード。
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>> {
    let body =
        rmp_serde::to_vec(msg).map_err(|e| SynergosNetError::Serialization(format!("{e}")))?;
    if body.len() > MAX_FRAME_SIZE {
        return Err(SynergosNetError::Serialization(format!(
            "DHT frame too large: {} > {}",
            body.len(),
            MAX_FRAME_SIZE
        )));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    if bytes.len() < 4 {
        return Err(SynergosNetError::Serialization(
            "DHT frame too short".into(),
        ));
    }
    let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if len > MAX_FRAME_SIZE || bytes.len() != 4 + len {
        return Err(SynergosNetError::Serialization(format!(
            "DHT frame length mismatch ({} declared, {} actual)",
            len,
            bytes.len() - 4
        )));
    }
    rmp_serde::from_slice(&bytes[4..]).map_err(|e| SynergosNetError::Serialization(format!("{e}")))
}

/// リクエスト 1 本あたりの最大サイズ (1 MiB)。`FindNode` でも 1 shard の
/// 返すノード数は k 個 (20 程度) なので十分な余裕。
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_request_roundtrip() {
        let req = DhtRequest::FindNode {
            target: PeerId::new("abc"),
        };
        let bytes = encode(&req).unwrap();
        let back: DhtRequest = decode(&bytes).unwrap();
        match back {
            DhtRequest::FindNode { target } => assert_eq!(target.0, "abc"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn encode_decode_response_roundtrip() {
        let resp = DhtResponse::Nodes {
            exact: None,
            closest: vec![PeerRecordDto {
                peer_id: PeerId::new("a"),
                display_name: "alice".into(),
                endpoints: vec![],
                active_projects: vec!["p1".into()],
                ttl_secs: 60,
            }],
        };
        let bytes = encode(&resp).unwrap();
        let back: DhtResponse = decode(&bytes).unwrap();
        match back {
            DhtResponse::Nodes { closest, .. } => assert_eq!(closest.len(), 1),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn decode_rejects_malformed_length() {
        let mut bytes = vec![0u8; 10];
        bytes[0..4].copy_from_slice(&999u32.to_be_bytes());
        let r: Result<DhtRequest> = decode(&bytes);
        assert!(r.is_err());
    }
}
