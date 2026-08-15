mod json_store;

pub use json_store::JsonStore;

use serde::{Deserialize, Serialize};

/// 組織 (テナント)。ノードとメンバー (許可された人) を束ねる単位。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    /// slug (URL セーフな識別子)。
    pub id: String,
    pub name: String,
    /// この組織に属する人のメールアドレス (Cloudflare 登録デバイスの照合キー)。
    #[serde(default)]
    pub members: Vec<String>,
    pub created_at_ms: u64,
}

/// ノード種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Linux サーバ等の headless Mesh node (warp-cli connector で参加)。
    MeshNode,
    /// 人が使う端末 (Cloudflare One Client でエンロール)。
    ClientDevice,
}

/// 管制サーバーに登録されたノード。
#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub org_id: String,
    pub display_name: String,
    /// ノードの所有者 (メールアドレス)。dark node 判定の照合キー。
    pub owner_email: String,
    pub kind: NodeKind,
    /// MeshNode の場合: Cloudflare 側 connector (warp_connector tunnel) の ID。
    #[serde(default)]
    pub cf_connector_id: Option<String>,
    /// Synergos daemon の peer_id (判明している場合)。
    #[serde(default)]
    pub synergos_peer_id: Option<String>,
    /// Mesh 参加後に割り当てられた 100.96.0.0/12 のアドレス。
    /// 管理者が登録する期待値。heartbeat の自己申告値との照合に使う。
    #[serde(default)]
    pub mesh_ip: Option<String>,
    /// ノード自身が heartbeat で自己申告した Mesh IP。
    /// `mesh_ip` (管理者設定値) と食い違う場合は reconcile が mismatch として報告する。
    #[serde(default)]
    pub reported_mesh_ip: Option<String>,
    /// heartbeat 認証キーの blake3 ハッシュ (hex)。キー本体は保存しない。
    #[serde(default)]
    pub node_key_hash: Option<String>,
    /// 最後に heartbeat を受けた時刻 (unix ms)。
    #[serde(default)]
    pub last_heartbeat_ms: Option<u64>,
    /// heartbeat が報告した synergos-core のバージョン。
    #[serde(default)]
    pub synergos_version: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// レジストリ全体のスナップショット (永続化単位)。
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    #[serde(default)]
    pub orgs: Vec<Org>,
    #[serde(default)]
    pub nodes: Vec<Node>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
