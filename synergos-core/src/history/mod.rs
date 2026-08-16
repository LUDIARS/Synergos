//! 履歴ノード (history node) — docs/versioning-design.md §3。
//!
//! `[history] enabled = true` のノードだけが、publish / 受信した各 version の
//! 実体を丸ごと保持し (`store`)、索引 (`index`) で (rel, version) → object を引き、
//! 保持ポリシー (`gc`) で古い版を落とす。通常ノードでは何もしない。

pub mod gc;
pub mod index;
pub mod store;
pub mod wiring;

pub use gc::GcReport;
pub use store::{ArchiveOutcome, HistoryStore, StoredVersion};
