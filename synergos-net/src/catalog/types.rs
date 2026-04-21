use serde::{Deserialize, Serialize};

use crate::types::{Blake3Hash, ChunkId, FileId};

/// ルートカタログ（プロジェクトに 1 つ）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCatalog {
    pub project_id: String,
    /// 累積更新件数（単調増加）
    pub update_count: u64,
    /// チャンクのインデックス
    pub chunks: Vec<ChunkIndex>,
    /// カタログ全体の CRC
    pub catalog_crc: u32,
    /// 最終更新時刻 (Unix timestamp ms)
    pub last_updated: u64,
}

/// チャンクインデックス（ルートカタログ内のエントリ）
///
/// `crc` は帯域効率の良いキャッシュキー比較用に残してある。整合性検証
/// (改竄検出) には必ず `content_hash` を使うこと (S5 対策)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkIndex {
    pub chunk_id: ChunkId,
    /// このチャンク内ファイルの CRC を合成した値 (キャッシュキー用)
    pub crc: u32,
    /// このチャンク内ファイルの Blake3 合成ハッシュ (整合性検証用)。
    /// デフォルト値はゼロ。古いピアは未設定で送ってくる可能性があるため
    /// 受信側はゼロ値の場合フォールバックとして CRC だけで比較してよい。
    #[serde(default)]
    pub content_hash: Blake3Hash,
    /// このチャンクの最終更新時刻
    pub last_updated: u64,
}

/// チャンク（ファイル群のまとまり）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: ChunkId,
    /// ファイルエントリ（追加順）
    pub files: Vec<FileEntry>,
    /// チャンクあたりの最大ファイル数
    pub max_files: usize,
}

impl Chunk {
    pub fn new(max_files: usize) -> Self {
        Self {
            chunk_id: ChunkId::generate(),
            files: Vec::new(),
            max_files,
        }
    }

    pub fn is_full(&self) -> bool {
        self.files.len() >= self.max_files
    }

    /// チャンク内全ファイルの CRC を合成
    pub fn compute_crc(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        for file in &self.files {
            hasher.update(&file.crc.to_le_bytes());
        }
        hasher.finalize()
    }
}

/// チャンク内のファイルエントリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub file_id: FileId,
    /// ファイルパス（プロジェクトルート相対）
    pub path: String,
    /// 現在のファイル CRC (キャッシュキー / 低コスト比較用)
    pub crc: u32,
    /// 現在のファイル内容の Blake3 ハッシュ (整合性検証用)。
    /// 古いピアからのメッセージではゼロ値になり得る。
    #[serde(default)]
    pub content_hash: Blake3Hash,
    /// ファイルの状態
    pub state: FileState,
    /// ファイルサイズ
    pub size: u64,
}

/// ファイルの同期状態
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileState {
    /// 最新（全ノードで一致）
    Synced,
    /// ローカルで変更あり（未公開）
    LocalModified,
    /// リモートで更新あり（未取得）
    RemoteUpdated,
    /// コンフリクト状態
    Conflict,
    /// 削除済み
    Deleted,
}

/// カタログの差分
#[derive(Debug, Clone)]
pub struct CatalogDiff {
    /// 更新が必要なファイル
    pub updates: Vec<FileUpdateAction>,
    /// コンフリクトが発生したファイル
    pub conflicts: Vec<FileId>,
    /// CRC が変化したチャンク
    pub changed_chunks: Vec<ChunkId>,
}

/// ファイル更新アクション
#[derive(Debug, Clone)]
pub enum FileUpdateAction {
    /// リモートの最新版で更新
    ApplyRemote { file_id: FileId },
    /// 新規ファイルの取得
    FetchNew { file_id: FileId },
    /// ファイル削除
    Remove { file_id: FileId },
}
