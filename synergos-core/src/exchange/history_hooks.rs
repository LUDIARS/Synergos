//! Exchange と履歴ノード保管庫 (`crate::history`) の結合点。
//!
//! Exchange は保管庫の実装を知らず、daemon が注入する 2 つのフックだけを呼ぶ:
//! - [`ArchiveHook`]: 受信完了 / publish 直後に「この版の実体を保管して」
//! - [`HistoryLookup`]: FileWant の版が手元の最新と違うとき「保管庫にあるか」
//!
//! 通常ノード (履歴無効) ではどちらも注入されず、Exchange の挙動は従来どおり。

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use synergos_net::types::FileId;

/// 保管依頼 1 件。
#[derive(Debug, Clone)]
pub struct ArchiveRequest {
    pub project_id: String,
    pub file_id: FileId,
    pub version: u64,
    pub size: u64,
    pub crc: u32,
    /// 版を publish したピア (受信なら送信元)。
    pub publisher: String,
    /// `"published"` / `"received"`。
    pub source: &'static str,
    /// 作業ツリー上の完成ファイル (受信なら rename 後)。
    pub path: PathBuf,
}

/// 保管庫が (project, file_id, version) に対して返す実体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryHit {
    pub path: PathBuf,
    pub size: u64,
    pub crc: u32,
}

pub type ArchiveFuture =
    Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'static>>;
pub type LookupFuture =
    Pin<Box<dyn std::future::Future<Output = Option<HistoryHit>> + Send + 'static>>;

pub type ArchiveHook = Arc<dyn Fn(ArchiveRequest) -> ArchiveFuture + Send + Sync + 'static>;
pub type HistoryLookup = Arc<dyn Fn(String, FileId, u64) -> LookupFuture + Send + Sync + 'static>;

/// daemon が Exchange に注入するフックの束。
#[derive(Clone)]
pub struct HistoryHooks {
    pub archive: ArchiveHook,
    pub lookup: HistoryLookup,
}
