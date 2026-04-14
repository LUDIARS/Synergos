use std::collections::VecDeque;

use crate::error::{Result, SynergosNetError};
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
    /// 作成時刻 (Unix timestamp ms)
    pub timestamp: u64,
    /// 更新内容
    pub payload: ChainPayload,
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
        let mut block = ChainBlock {
            hash: Blake3Hash([0; 32]),
            prev_hash: prev,
            version,
            author: PeerId::new("test"),
            timestamp: version * 1000,
            payload: ChainPayload::TextDiff {
                patch: format!("patch-{}", version),
                result_crc: version as u32,
            },
        };
        block.compute_hash();
        block
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
