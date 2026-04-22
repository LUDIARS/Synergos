use std::fmt;
use std::net::SocketAddrV6;

use serde::{Deserialize, Serialize};

/// ピア識別子
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct PeerId(pub String);

impl PeerId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// ログ用の短縮表示: 先頭 12 文字 + "…"。
    /// 完全な PeerId を `info!` 等に載せないようにする (S24 対策)。
    /// 詳細な識別が必要な場合は `Display` を使用する。
    pub fn short(&self) -> String {
        if self.0.len() > 12 {
            format!("{}…", &self.0[..12])
        } else {
            self.0.clone()
        }
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
        for (i, d) in distance.iter_mut().enumerate() {
            *d = self.0[i] ^ other.0[i];
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
    Tunnel { tunnel_id: String, hostname: String },
    /// WebSocket リレー（フォールバック）
    Relay { server_url: String, room_id: String },
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

/// PeerId をログ用に短縮する。`PeerId::short` と同じだが、参照を `&PeerId`
/// として受け取れるので `tracing::info!("{}", truncate_peer_id(&pid))` と書ける。
pub fn truncate_peer_id(pid: &PeerId) -> String {
    pid.short()
}

/// プロジェクト相対にパスを縮める。パスがプロジェクトルート配下なら相対化、
/// 外にはみ出していれば file_name のみ、それも取れなければ "<redacted>"。
///
/// S20 対策として `IpcResponse::Error.message` や tracing ログに載せる path を
/// すべてこのヘルパを経由させることで、絶対パス (ユーザのホームディレクトリ等)
/// の漏洩を抑える。
pub fn redact_path(project_root: &std::path::Path, path: &std::path::Path) -> String {
    if let Ok(rel) = path.strip_prefix(project_root) {
        return rel.display().to_string();
    }
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => "<redacted>".to_string(),
    }
}

/// UNIX epoch からの経過時間を milliseconds で返す共通ユーティリティ。
/// `conflict` / `exchange` / `catalog` で個別定義されていた重複を解消する。
///
/// 時計が epoch より前に設定されている等の異常状態では 0 を返す。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests_redact {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn redact_keeps_relative_within_root() {
        let root = PathBuf::from("/home/user/project");
        let inside = root.join("src/main.rs");
        assert_eq!(redact_path(&root, &inside), "src/main.rs");
    }

    #[test]
    fn redact_outside_root_returns_file_name_only() {
        let root = PathBuf::from("/home/user/project");
        let outside = PathBuf::from("/etc/passwd");
        assert_eq!(redact_path(&root, &outside), "passwd");
    }

    #[test]
    fn truncate_peer_id_short() {
        let p = PeerId::new("abcdefghijklmno");
        assert!(truncate_peer_id(&p).ends_with('…'));
    }
}
