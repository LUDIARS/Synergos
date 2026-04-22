use serde::{Deserialize, Serialize};

use crate::error::{Result, SynergosNetError};
use crate::identity::{self, Identity};
use crate::types::{Blake3Hash, ChunkId, Cid, FileId, MessageId, PeerId};

/// 署名付き Gossip メッセージ (S3 対策: FileOffer.sender / PeerStatus.origin が
/// 無証明だった問題を解消する)。全ての GossipMessage は本封筒に包んで送出され、
/// 受信側で `verify` を通ったものだけが `apply` される。
///
/// pubkey / signature は固定長だが serde の `[u8; 64]` 自動 derive 非対応を
/// 避けるため、wire 上は `Vec<u8>` で保持する。`verify` 時に長さを検証する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedGossipMessage {
    pub message: GossipMessage,
    /// 送信者の ed25519 公開鍵 (32 bytes expected)
    pub sender_public_key: Vec<u8>,
    /// `signing_bytes(&message)` への ed25519 署名 (64 bytes expected)
    pub signature: Vec<u8>,
}

impl SignedGossipMessage {
    /// 自分の Identity でメッセージを署名する。
    pub fn sign(message: GossipMessage, identity: &Identity) -> Self {
        let bytes = signing_bytes(&message);
        let signature = identity.sign(&bytes);
        Self {
            message,
            sender_public_key: identity.public_key_bytes().to_vec(),
            signature: signature.to_vec(),
        }
    }

    /// 署名が有効か検証する。メッセージ中の sender / origin / requester
    /// 等の peer_id が `sender_public_key` の導出結果と一致することも確認する。
    pub fn verify(&self) -> Result<()> {
        if self.sender_public_key.len() != 32 {
            return Err(SynergosNetError::Identity(
                "gossip sender_public_key length != 32".into(),
            ));
        }
        if self.signature.len() != 64 {
            return Err(SynergosNetError::Identity(
                "gossip signature length != 64".into(),
            ));
        }
        let mut pub_bytes = [0u8; 32];
        pub_bytes.copy_from_slice(&self.sender_public_key);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&self.signature);
        let derived = identity::peer_id_from_public_bytes(&pub_bytes);
        // 各メッセージの「送信者」にあたるフィールドを確認する。
        // 他のピアの情報を中継する場合は sender != origin のケースもあり得るので
        // ここでは message 内の「自己申告の送信者」と一致するかを見る。
        match &self.message {
            GossipMessage::PeerStatus { origin, .. } => {
                if origin != &derived {
                    return Err(SynergosNetError::Identity(
                        "gossip origin peer_id mismatch with signer".into(),
                    ));
                }
            }
            GossipMessage::FileOffer { sender, .. } => {
                if sender != &derived {
                    return Err(SynergosNetError::Identity(
                        "gossip FileOffer sender mismatch with signer".into(),
                    ));
                }
            }
            GossipMessage::FileWant { requester, .. } => {
                if requester != &derived {
                    return Err(SynergosNetError::Identity(
                        "gossip FileWant requester mismatch with signer".into(),
                    ));
                }
            }
            GossipMessage::CatalogUpdate { publisher, .. } => {
                // `publisher` が旧ピア互換のためゼロ / 空のときはスキップ (序文の署名検証のみ)。
                if !publisher.0.is_empty() && publisher != &derived {
                    return Err(SynergosNetError::Identity(
                        "gossip CatalogUpdate publisher mismatch with signer".into(),
                    ));
                }
            }
            // ConflictAlert は project 全体向けなので送信者の明示フィールドは無い。
            GossipMessage::ConflictAlert { .. } => {}
        }
        identity::verify(&pub_bytes, &signing_bytes(&self.message), &sig_bytes)
            .map_err(|_| SynergosNetError::Identity("gossip signature invalid".into()))
    }
}

/// Gossip メッセージの正規バイト列化。
/// 署名対象 + MessageCache キー (S21 対策: `format!("{:?}", msg)` の不安定性
/// を排除) の両方で使う。実運用の長期互換性のためには msgpack / proto /
/// CBOR 等の正規形式が望ましいが、暫定として自前の固定タグ方式で決定的にする。
pub fn canonical_bytes(msg: &GossipMessage) -> Vec<u8> {
    signing_bytes(msg)
}

fn signing_bytes(msg: &GossipMessage) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    match msg {
        GossipMessage::PeerStatus {
            peer_id,
            status,
            origin,
            hops,
        } => {
            out.extend_from_slice(b"peer_status");
            out.extend_from_slice(peer_id.0.as_bytes());
            out.extend_from_slice(origin.0.as_bytes());
            out.push(*hops);
            out.extend_from_slice(status.peer_id.0.as_bytes());
            out.extend_from_slice(status.display_name.as_bytes());
            out.push(status.state as u8);
            out.extend_from_slice(&status.last_active.to_le_bytes());
            for f in &status.working_on {
                out.extend_from_slice(f.0.as_bytes());
                out.push(0);
            }
        }
        GossipMessage::CatalogUpdate {
            project_id,
            root_crc,
            update_count,
            updated_chunks,
            catalog_cid,
            publisher,
        } => {
            out.extend_from_slice(b"catalog_update");
            out.extend_from_slice(project_id.as_bytes());
            out.extend_from_slice(&root_crc.to_le_bytes());
            out.extend_from_slice(&update_count.to_le_bytes());
            for c in updated_chunks {
                out.extend_from_slice(c.0.as_bytes());
                out.push(0);
            }
            if let Some(cid) = catalog_cid {
                out.extend_from_slice(cid.0.as_bytes());
                out.push(0);
            }
            out.extend_from_slice(publisher.0.as_bytes());
        }
        GossipMessage::FileWant {
            requester,
            file_id,
            version,
        } => {
            out.extend_from_slice(b"file_want");
            out.extend_from_slice(requester.0.as_bytes());
            out.extend_from_slice(file_id.0.as_bytes());
            out.extend_from_slice(&version.to_le_bytes());
        }
        GossipMessage::FileOffer {
            sender,
            file_id,
            version,
            size,
            crc,
            content_hash,
        } => {
            out.extend_from_slice(b"file_offer");
            out.extend_from_slice(sender.0.as_bytes());
            out.extend_from_slice(file_id.0.as_bytes());
            out.extend_from_slice(&version.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&content_hash.0);
        }
        GossipMessage::ConflictAlert {
            file_id,
            conflicting_nodes,
            their_versions,
        } => {
            out.extend_from_slice(b"conflict_alert");
            out.extend_from_slice(file_id.0.as_bytes());
            for p in conflicting_nodes {
                out.extend_from_slice(p.0.as_bytes());
                out.push(0);
            }
            for v in their_versions {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}

/// Gossip メッセージの種類
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GossipMessage {
    /// ピアのアクティブ状態
    PeerStatus {
        peer_id: PeerId,
        status: PeerActivityStatus,
        /// この情報を最初に発信したピア
        origin: PeerId,
        /// ホップ数（伝播距離）
        hops: u8,
    },
    /// カタログ更新通知
    CatalogUpdate {
        project_id: String,
        root_crc: u32,
        update_count: u64,
        updated_chunks: Vec<ChunkId>,
        /// 発行者側 `ContentStore` に置かれた `RootCatalog` スナップショットの CID。
        /// 受信側は BitswapSession でこの CID を引いて full diff を計算する。
        /// 旧ピア互換のため optional で、無いときはドリフト検知のみに留まる。
        #[serde(default)]
        catalog_cid: Option<Cid>,
        /// この CatalogUpdate を発行したピア。BitswapSession の接続先に使う。
        /// 旧ピア互換のため serde default で空 PeerId を許容する。
        #[serde(default)]
        publisher: PeerId,
    },
    /// ファイル更新要求（受信側が「欲しい」と宣言）
    FileWant {
        requester: PeerId,
        file_id: FileId,
        version: u64,
    },
    /// ファイル送信通知（送信側が「送りたい」と宣言）
    ///
    /// `crc` は低コスト比較キャッシュ用。整合性検証は `content_hash`
    /// (Blake3 で衝突耐性を持つ) を使うこと。古いピアが content_hash を
    /// ゼロのまま送ってきた場合は受信側でフォールバック判断する。
    FileOffer {
        sender: PeerId,
        file_id: FileId,
        version: u64,
        size: u64,
        crc: u32,
        #[serde(default)]
        content_hash: Blake3Hash,
    },
    /// コンフリクト通知
    ConflictAlert {
        file_id: FileId,
        conflicting_nodes: Vec<PeerId>,
        their_versions: Vec<u64>,
    },
}

/// ピアのアクティビティ状態
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerActivityStatus {
    pub peer_id: PeerId,
    pub display_name: String,
    pub state: ActivityState,
    /// 最終アクティブ時刻 (Unix timestamp ms)
    pub last_active: u64,
    /// 作業中のファイル（任意）
    pub working_on: Vec<FileId>,
}

/// アクティビティ状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityState {
    /// アクティブに作業中
    Active,
    /// アイドル（接続はしている）
    Idle,
    /// 離席
    Away,
    /// オフライン（他ピアのキャッシュ情報）
    Offline,
}

/// Gossipsub 制御メッセージ
#[derive(Debug, Clone)]
pub enum GossipControl {
    /// メッシュへの参加要求
    Graft,
    /// メッシュからの離脱
    Prune,
    /// 保有メッセージの告知（重複排除用）
    IHave { message_ids: Vec<MessageId> },
    /// メッセージの要求
    IWant { message_ids: Vec<MessageId> },
}
