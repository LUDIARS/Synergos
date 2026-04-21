use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::types::*;
use crate::chain::{ChainBlock, FileChain};
use crate::error::{Result, SynergosNetError};
use crate::types::{ChunkId, FileId};

/// カタログマネージャ
pub struct CatalogManager {
    root: RwLock<RootCatalog>,
    chunks: DashMap<ChunkId, Chunk>,
    /// 各ファイルの更新チェーン
    chains: DashMap<FileId, FileChain>,
    /// ファイルID → 所属チャンクID のマッピング
    file_to_chunk: DashMap<FileId, ChunkId>,
    /// チャンクあたりの最大ファイル数
    chunk_max_files: usize,
    /// チェーンの最大深度
    chain_max_depth: usize,
}

impl CatalogManager {
    pub fn new(project_id: String, chunk_max_files: usize, chain_max_depth: usize) -> Self {
        Self {
            root: RwLock::new(RootCatalog {
                project_id,
                update_count: 0,
                chunks: Vec::new(),
                catalog_crc: 0,
                last_updated: now_ms(),
            }),
            chunks: DashMap::new(),
            chains: DashMap::new(),
            file_to_chunk: DashMap::new(),
            chunk_max_files,
            chain_max_depth,
        }
    }

    /// ファイルを追加（次の空きチャンクに配置）
    pub async fn add_file(&self, path: &str, crc: u32, size: u64) -> FileId {
        let file_id = FileId::generate();
        let entry = FileEntry {
            file_id: file_id.clone(),
            path: path.to_string(),
            crc,
            // 低コスト API のため content_hash の実値は呼び出し側 (PublishUpdate
            // ハンドラ等で Blake3 を算出) から `record_update` で差し込む。
            content_hash: Default::default(),
            state: FileState::Synced,
            size,
        };

        // 空きのあるチャンクを探す、なければ新規作成
        let chunk_id = self.find_or_create_chunk().await;

        if let Some(mut chunk) = self.chunks.get_mut(&chunk_id) {
            chunk.files.push(entry);
        }

        // チェーンを初期化
        self.chains.insert(
            file_id.clone(),
            FileChain::new(file_id.clone(), self.chain_max_depth),
        );
        self.file_to_chunk.insert(file_id.clone(), chunk_id.clone());

        // カタログ CRC を更新
        self.update_chunk_index(&chunk_id).await;
        self.recompute_catalog_crc().await;

        file_id
    }

    /// ファイル更新をチェーンに記録 + カタログ CRC 更新
    pub async fn record_update(&self, file_id: &FileId, block: ChainBlock) -> Result<()> {
        // 1. FileChain にブロック追加
        let mut chain = self.chains.get_mut(file_id).ok_or_else(|| {
            SynergosNetError::PeerNotFound(format!("File not found: {}", file_id))
        })?;
        chain.append(block)?;

        // 2. FileEntry の CRC を更新
        let new_crc = chain.head_crc().unwrap_or(0);
        drop(chain);

        if let Some(chunk_id) = self.file_to_chunk.get(file_id) {
            if let Some(mut chunk) = self.chunks.get_mut(&*chunk_id) {
                if let Some(entry) = chunk.files.iter_mut().find(|f| f.file_id == *file_id) {
                    entry.crc = new_crc;
                    entry.state = FileState::Synced;
                }
            }
            // 3. チャンク CRC を再計算
            self.update_chunk_index(&chunk_id).await;
        }

        // 4-5. ルートカタログ更新
        {
            let mut root = self.root.write().await;
            root.update_count += 1;
            root.last_updated = now_ms();
        }
        self.recompute_catalog_crc().await;

        Ok(())
    }

    /// ファイルの状態を変更
    pub async fn set_file_state(&self, file_id: &FileId, state: FileState) {
        if let Some(chunk_id) = self.file_to_chunk.get(file_id) {
            if let Some(mut chunk) = self.chunks.get_mut(&*chunk_id) {
                if let Some(entry) = chunk.files.iter_mut().find(|f| f.file_id == *file_id) {
                    entry.state = state;
                }
            }
        }
    }

    /// リモートカタログとの差分を検出
    pub async fn diff_catalog(&self, remote: &RootCatalog) -> CatalogDiff {
        let root = self.root.read().await;

        let mut diff = CatalogDiff {
            updates: Vec::new(),
            conflicts: Vec::new(),
            changed_chunks: Vec::new(),
        };

        // update_count が同じなら変更なし
        if root.update_count == remote.update_count && root.catalog_crc == remote.catalog_crc {
            return diff;
        }

        // チャンク CRC を比較し、異なるチャンクのみ詳細比較
        for remote_chunk_idx in &remote.chunks {
            let local_match = root
                .chunks
                .iter()
                .find(|c| c.chunk_id == remote_chunk_idx.chunk_id);

            match local_match {
                Some(local_idx) if local_idx.crc == remote_chunk_idx.crc => {
                    // CRC 一致 → スキップ
                }
                Some(_) => {
                    // CRC 不一致 → チャンク内の各ファイルを比較
                    diff.changed_chunks.push(remote_chunk_idx.chunk_id.clone());
                    self.diff_chunk_files(&remote_chunk_idx.chunk_id, &mut diff)
                        .await;
                }
                None => {
                    // 新しいチャンク → 全ファイルを取得対象に
                    diff.changed_chunks.push(remote_chunk_idx.chunk_id.clone());
                }
            }
        }

        diff
    }

    /// ルートカタログのスナップショットを取得
    pub async fn root_catalog(&self) -> RootCatalog {
        self.root.read().await.clone()
    }

    /// ファイルのチェーンを取得
    pub fn get_chain(
        &self,
        file_id: &FileId,
    ) -> Option<dashmap::mapref::one::Ref<'_, FileId, FileChain>> {
        self.chains.get(file_id)
    }

    // --- internal ---

    async fn find_or_create_chunk(&self) -> ChunkId {
        // 最後のチャンクに空きがあればそれを返す
        let root = self.root.read().await;
        if let Some(last) = root.chunks.last() {
            if let Some(chunk) = self.chunks.get(&last.chunk_id) {
                if !chunk.is_full() {
                    return last.chunk_id.clone();
                }
            }
        }
        drop(root);

        // 新規チャンク作成
        let chunk = Chunk::new(self.chunk_max_files);
        let id = chunk.chunk_id.clone();
        self.chunks.insert(id.clone(), chunk);

        let mut root = self.root.write().await;
        root.chunks.push(ChunkIndex {
            chunk_id: id.clone(),
            crc: 0,
            content_hash: Default::default(),
            last_updated: now_ms(),
        });

        id
    }

    async fn update_chunk_index(&self, chunk_id: &ChunkId) {
        if let Some(chunk) = self.chunks.get(chunk_id) {
            let crc = chunk.compute_crc();
            let mut root = self.root.write().await;
            if let Some(idx) = root.chunks.iter_mut().find(|c| c.chunk_id == *chunk_id) {
                idx.crc = crc;
                idx.last_updated = now_ms();
            }
        }
    }

    async fn recompute_catalog_crc(&self) {
        let mut root = self.root.write().await;
        let mut hasher = crc32fast::Hasher::new();
        for chunk_idx in &root.chunks {
            hasher.update(&chunk_idx.crc.to_le_bytes());
        }
        root.catalog_crc = hasher.finalize();
    }

    async fn diff_chunk_files(&self, chunk_id: &ChunkId, diff: &mut CatalogDiff) {
        if let Some(chunk) = self.chunks.get(chunk_id) {
            for file_entry in &chunk.files {
                if let Some(_chain) = self.chains.get(&file_entry.file_id) {
                    // ローカルに変更がありツリーが分岐 → コンフリクト
                    if file_entry.state == FileState::LocalModified {
                        diff.conflicts.push(file_entry.file_id.clone());
                    } else {
                        diff.updates.push(FileUpdateAction::ApplyRemote {
                            file_id: file_entry.file_id.clone(),
                        });
                    }
                } else {
                    diff.updates.push(FileUpdateAction::FetchNew {
                        file_id: file_entry.file_id.clone(),
                    });
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{ChainBlock, ChainPayload};
    use crate::types::{Blake3Hash, Cid, PeerId};

    #[tokio::test]
    async fn test_add_file_and_record_update() {
        let mgr = CatalogManager::new("test-project".into(), 256, 10);

        let file_id = mgr.add_file("src/main.rs", 0x1234, 1024).await;

        let root = mgr.root_catalog().await;
        assert_eq!(root.chunks.len(), 1);
        assert_eq!(root.update_count, 0);

        // Record an update
        let id = crate::identity::Identity::generate();
        let mut block = ChainBlock {
            hash: Blake3Hash([0; 32]),
            prev_hash: None,
            version: 1,
            author: PeerId::new("placeholder"),
            author_public_key: [0u8; 32],
            timestamp: 1000,
            payload: ChainPayload::FullSnapshot {
                cid: Cid("Qm...".into()),
                file_size: 2048,
                crc: 0x5678,
            },
            signature: [0u8; 64],
        };
        block.sign(&id);
        mgr.record_update(&file_id, block).await.unwrap();

        let root = mgr.root_catalog().await;
        assert_eq!(root.update_count, 1);
    }

    #[tokio::test]
    async fn test_chunk_overflow() {
        let mgr = CatalogManager::new("test".into(), 2, 10);

        mgr.add_file("a.rs", 1, 100).await;
        mgr.add_file("b.rs", 2, 200).await;
        mgr.add_file("c.rs", 3, 300).await; // should create new chunk

        let root = mgr.root_catalog().await;
        assert_eq!(root.chunks.len(), 2);
    }
}
