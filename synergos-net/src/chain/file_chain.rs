use std::collections::VecDeque;

use crate::error::{Result, SynergosNetError};
use crate::identity::{self, Identity};
use crate::types::{Blake3Hash, Cid, FileId, PeerId};

/// チェーンブロック
#[derive(Debug, Clone)]
pub struct ChainBlock {
    /// ブロックのハッシュ (Blake3)
    pub hash: Blake3Hash,
    /// 前ブロックのハッシュ（先頭ブロックは None）
    pub prev_hash: Option<Blake3Hash>,
    /// バージョン番号（単調増加）
    pub version: u64,
    /// 作成者
    pub author: PeerId,
    /// 作成者の公開鍵 (32 bytes)。PeerId と独立で保持することで
    /// 受信側が鍵→PeerId 導出と署名検証の両方を確認できる (S4 対策)。
    pub author_public_key: [u8; 32],
    /// 作成時刻 (Unix timestamp ms)
    pub timestamp: u64,
    /// 更新内容
    pub payload: ChainPayload,
    /// 作者による `signing_message()` への ed25519 署名 (64 bytes)。
    /// 未署名ブロックは 0 埋め。`sign(&mut self, ...)` 呼び出しで埋まる。
    pub signature: [u8; 64],
}

impl ChainBlock {
    /// ブロックのハッシュを計算して設定する
    pub fn compute_hash(&mut self) {
        let mut hasher = blake3::Hasher::new();
        if let Some(ref prev) = self.prev_hash {
            hasher.update(&prev.0);
        }
        hasher.update(&self.version.to_le_bytes());
        hasher.update(self.author.0.as_bytes());
        hasher.update(&self.author_public_key);
        hasher.update(&self.timestamp.to_le_bytes());
        match &self.payload {
            ChainPayload::TextDiff { patch, result_crc } => {
                hasher.update(b"text_diff");
                hasher.update(patch.as_bytes());
                hasher.update(&result_crc.to_le_bytes());
            }
            ChainPayload::BinaryCid {
                cid,
                file_size,
                crc,
            } => {
                hasher.update(b"binary_cid");
                hasher.update(cid.0.as_bytes());
                hasher.update(&file_size.to_le_bytes());
                hasher.update(&crc.to_le_bytes());
            }
            ChainPayload::FullSnapshot {
                cid,
                file_size,
                crc,
            } => {
                hasher.update(b"full_snapshot");
                hasher.update(cid.0.as_bytes());
                hasher.update(&file_size.to_le_bytes());
                hasher.update(&crc.to_le_bytes());
            }
        }
        self.hash = Blake3Hash(*hasher.finalize().as_bytes());
    }

    /// 署名対象のメッセージバイト列を組み立てる。
    /// - 自身のブロックハッシュを含むことで `compute_hash` が改竄検出に寄与。
    /// - prev_hash も含み、チェーン全体の改竄を遡って検出できるように。
    pub fn signing_message(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32 + 33 + 8 + 32 + 8);
        buf.extend_from_slice(&self.hash.0);
        if let Some(ref prev) = self.prev_hash {
            buf.push(1);
            buf.extend_from_slice(&prev.0);
        } else {
            buf.push(0);
        }
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.author_public_key);
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf
    }

    /// 作者の Identity で署名し、`author` / `author_public_key` を確定させる。
    /// `compute_hash` はこの関数内で呼ばれる。
    pub fn sign(&mut self, identity: &Identity) {
        self.author = identity.peer_id().clone();
        self.author_public_key = identity.public_key_bytes();
        self.compute_hash();
        self.signature = identity.sign(&self.signing_message());
    }

    /// 受信したブロックを検証する。
    /// 1. `author_public_key` → PeerId 導出が `author` と一致するか
    /// 2. `compute_hash` の結果が格納済み `hash` と一致するか
    /// 3. `signature` が `author_public_key` による有効な署名か
    pub fn verify(&self) -> Result<()> {
        let derived = identity::peer_id_from_public_bytes(&self.author_public_key);
        if derived != self.author {
            return Err(SynergosNetError::Gossip(
                "ChainBlock author does not match public key".into(),
            ));
        }
        let mut clone = self.clone();
        clone.compute_hash();
        if clone.hash != self.hash {
            return Err(SynergosNetError::Gossip(
                "ChainBlock hash mismatch".into(),
            ));
        }
        identity::verify(
            &self.author_public_key,
            &self.signing_message(),
            &self.signature,
        )
        .map_err(|_| SynergosNetError::Gossip("ChainBlock signature invalid".into()))?;
        Ok(())
    }

    /// 署名が埋まっているか (全 0 でないか) を簡易判定。
    pub fn is_signed(&self) -> bool {
        self.signature.iter().any(|b| *b != 0)
    }
}

/// ブロックのペイロード
#[derive(Debug, Clone)]
pub enum ChainPayload {
    /// テキストファイルの差分（unified diff）
    TextDiff { patch: String, result_crc: u32 },
    /// バイナリファイルの IPFS CID 参照
    BinaryCid { cid: Cid, file_size: u64, crc: u32 },
    /// フルスナップショット（初回 or 定期ベースライン）
    FullSnapshot { cid: Cid, file_size: u64, crc: u32 },
}

/// ファイルごとの更新チェーン
pub struct FileChain {
    pub file_id: FileId,
    /// 直近 N 件のブロック（古い順）
    blocks: VecDeque<ChainBlock>,
    /// 保持件数上限
    max_depth: usize,
    /// HEAD ブロックのハッシュ
    pub head: Option<Blake3Hash>,
}

impl FileChain {
    pub fn new(file_id: FileId, max_depth: usize) -> Self {
        Self {
            file_id,
            blocks: VecDeque::new(),
            max_depth,
            head: None,
        }
    }

    /// 新しいブロックを追加
    pub fn append(&mut self, block: ChainBlock) -> Result<()> {
        // prev_hash が現在の HEAD と一致するか検証
        if block.prev_hash != self.head {
            return Err(SynergosNetError::ChainFork {
                expected: self.head.clone(),
                got: block.prev_hash.clone(),
            });
        }
        self.head = Some(block.hash.clone());
        self.blocks.push_back(block);
        // 上限超過分を削除
        while self.blocks.len() > self.max_depth {
            self.blocks.pop_front();
        }
        Ok(())
    }

    /// 指定バージョンからの差分ブロック列を取得
    pub fn blocks_since(&self, version: u64) -> Vec<&ChainBlock> {
        self.blocks.iter().filter(|b| b.version > version).collect()
    }

    /// HEAD ブロックを取得
    pub fn head_block(&self) -> Option<&ChainBlock> {
        self.blocks.back()
    }

    /// HEAD の CRC を取得
    pub fn head_crc(&self) -> Option<u32> {
        self.blocks.back().map(|b| match &b.payload {
            ChainPayload::TextDiff { result_crc, .. } => *result_crc,
            ChainPayload::BinaryCid { crc, .. } => *crc,
            ChainPayload::FullSnapshot { crc, .. } => *crc,
        })
    }

    /// HEAD のバージョンを取得
    pub fn head_version(&self) -> u64 {
        self.blocks.back().map(|b| b.version).unwrap_or(0)
    }

    /// 全ブロックを取得
    pub fn blocks(&self) -> &VecDeque<ChainBlock> {
        &self.blocks
    }

    /// ブロックのイテレータを取得
    pub fn blocks_iter(&self) -> impl Iterator<Item = &ChainBlock> {
        self.blocks.iter()
    }

    /// 指定ハッシュのブロックを検索
    pub fn find_block(&self, hash: &Blake3Hash) -> Option<&ChainBlock> {
        self.blocks.iter().find(|b| &b.hash == hash)
    }

    /// 2 つのチェーンの共通祖先を見つける
    pub fn find_common_ancestor(&self, remote_chain: &FileChain) -> Option<&ChainBlock> {
        // ローカルのブロックを逆順に走査し、リモートにも存在するものを探す
        self.blocks
            .iter()
            .rev()
            .find(|local_block| remote_chain.find_block(&local_block.hash).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_block(version: u64, prev: Option<Blake3Hash>) -> ChainBlock {
        let id = Identity::generate();
        let mut block = ChainBlock {
            hash: Blake3Hash([0; 32]),
            prev_hash: prev,
            version,
            author: id.peer_id().clone(),
            author_public_key: id.public_key_bytes(),
            timestamp: version * 1000,
            payload: ChainPayload::TextDiff {
                patch: format!("patch-{}", version),
                result_crc: version as u32,
            },
            signature: [0u8; 64],
        };
        block.sign(&id);
        block
    }

    #[test]
    fn signed_block_verifies() {
        let id = Identity::generate();
        let mut block = ChainBlock {
            hash: Blake3Hash([0; 32]),
            prev_hash: None,
            version: 1,
            author: PeerId::new(""), // sign が上書き
            author_public_key: [0u8; 32],
            timestamp: 42,
            payload: ChainPayload::TextDiff {
                patch: "p".into(),
                result_crc: 7,
            },
            signature: [0u8; 64],
        };
        block.sign(&id);
        assert!(block.is_signed());
        block.verify().expect("valid signature");
    }

    #[test]
    fn tampered_payload_fails_verify() {
        let id = Identity::generate();
        let mut block = ChainBlock {
            hash: Blake3Hash([0; 32]),
            prev_hash: None,
            version: 1,
            author: PeerId::new(""),
            author_public_key: [0u8; 32],
            timestamp: 42,
            payload: ChainPayload::TextDiff {
                patch: "original".into(),
                result_crc: 1,
            },
            signature: [0u8; 64],
        };
        block.sign(&id);
        // ペイロードを差し替え (hash は元のまま)
        block.payload = ChainPayload::TextDiff {
            patch: "tampered".into(),
            result_crc: 999,
        };
        assert!(block.verify().is_err());
    }

    #[test]
    fn test_append_and_head() {
        let mut chain = FileChain::new(FileId::new("f1"), 10);
        let b0 = make_block(1, None);
        chain.append(b0.clone()).unwrap();

        assert_eq!(chain.head_version(), 1);
        assert_eq!(chain.head, Some(b0.hash.clone()));

        let b1 = make_block(2, Some(b0.hash.clone()));
        chain.append(b1.clone()).unwrap();
        assert_eq!(chain.head_version(), 2);
    }

    #[test]
    fn test_fork_detection() {
        let mut chain = FileChain::new(FileId::new("f1"), 10);
        let b0 = make_block(1, None);
        chain.append(b0).unwrap();

        // prev_hash が HEAD と一致しないブロック → エラー
        let bad = make_block(2, Some(Blake3Hash([99; 32])));
        assert!(chain.append(bad).is_err());
    }

    #[test]
    fn test_max_depth() {
        let mut chain = FileChain::new(FileId::new("f1"), 3);
        let mut prev: Option<Blake3Hash> = None;
        for v in 1..=5 {
            let b = make_block(v, prev.clone());
            prev = Some(b.hash.clone());
            chain.append(b).unwrap();
        }
        assert_eq!(chain.blocks().len(), 3);
        assert_eq!(chain.blocks().front().unwrap().version, 3);
    }
}
