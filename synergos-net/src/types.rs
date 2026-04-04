use std::fmt;
use std::net::SocketAddrV6;

use serde::{Deserialize, Serialize};

/// ピア識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub String);

impl PeerId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// DHT ノード識別子（PeerId の Blake3 ハッシュから導出）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub fn from_peer_id(peer_id: &PeerId) -> Self {
        let hash = blake3::hash(peer_id.0.as_bytes());
        Self(*hash.as_bytes())
    }

    /// 2つの NodeId 間の XOR 距離を計算（Kademlia）
    pub fn xor_distance(&self, other: &NodeId) -> [u8; 32] {
        let mut distance = [0u8; 32];
        for i in 0..32 {
            distance[i] = self.0[i] ^ other.0[i];
        }
        distance
    }

    /// XOR 距離の先頭ゼロビット数（bucket index の決定に使用）
    pub fn leading_zeros(&self, other: &NodeId) -> u32 {
        let dist = self.xor_distance(other);
        let mut zeros = 0u32;
        for byte in &dist {
            if *byte == 0 {
                zeros += 8;
            } else {
                zeros += byte.leading_zeros();
                break;
            }
        }
        zeros
    }
}

/// ファイル識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub String);

impl FileId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 転送識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(pub String);

impl TransferId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// チャンク識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub String);

impl ChunkId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Gossipsub Topic 識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopicId(pub String);

impl TopicId {
    pub fn project(project_id: &str) -> Self {
        Self(format!("project/{}", project_id))
    }
}

/// メッセージ識別子（重複排除用）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(pub [u8; 32]);

impl MessageId {
    pub fn from_content(content: &[u8]) -> Self {
        Self(*blake3::hash(content).as_bytes())
    }
}

/// Blake3 ハッシュ
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Blake3Hash(pub [u8; 32]);

impl Blake3Hash {
    pub fn compute(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }
}

/// IPFS Content Identifier (簡易版)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cid(pub String);

/// 接続経路
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Route {
    /// IPv6 ダイレクト接続
    Direct {
        addr: SocketAddrV6,
        fqdn: Option<String>,
    },
    /// Cloudflare Tunnel 経由
    Tunnel {
        tunnel_id: String,
        hostname: String,
    },
    /// WebSocket リレー（フォールバック）
    Relay {
        server_url: String,
        room_id: String,
    },
}

/// 経路種別（簡易判別用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteKind {
    Direct,
    Tunnel,
    Relay,
}

impl Route {
    pub fn kind(&self) -> RouteKind {
        match self {
            Route::Direct { .. } => RouteKind::Direct,
            Route::Tunnel { .. } => RouteKind::Tunnel,
            Route::Relay { .. } => RouteKind::Relay,
        }
    }
}

/// 接続状態
#[derive(Debug, Clone)]
pub enum ConnectionState {
    Discovered,
    Connecting,
    Connected { rtt_ms: u32, route: RouteKind },
    Disconnected { reason: String },
}

/// ピアエンドポイント情報
#[derive(Debug, Clone)]
pub struct PeerEndpoint {
    pub peer_id: PeerId,
    pub display_name: String,
    pub routes: Vec<Route>,
    pub state: ConnectionState,
}

/// 現在時刻を Unix epoch ミリ秒で取得するユーティリティ
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// ファイルサイズクラス
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileSizeClass {
    /// 100MB 以上
    Large,
    /// 1MB 〜 100MB
    Medium,
    /// 1MB 未満
    Small,
}

impl FileSizeClass {
    const MB: u64 = 1024 * 1024;

    pub fn classify(size: u64) -> Self {
        match size {
            s if s >= 100 * Self::MB => Self::Large,
            s if s >= Self::MB => Self::Medium,
            _ => Self::Small,
        }
    }
}
