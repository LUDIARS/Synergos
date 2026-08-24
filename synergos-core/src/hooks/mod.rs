//! publish / 受信時フック (docs/hooks.md)。
//!
//! 2 層定義: daemon 単位 (`synergos.toml` の `[hooks]`, 常に有効) と
//! プロジェクト単位 (`<project root>/.synergos/hooks.toml`, `hooks.allow_project_hooks = true`
//! のノードだけ opt-in で実行)。`pre-publish` は同期待ちで非 0 exit なら publish を中止する。
//! `post-publish` / `post-receive` は spawn するだけで待たない (転送・イベントループをブロックしない)。

pub mod project_file;
pub mod runner;
pub mod wiring;

pub use runner::{HookEvent, HookOutcome, HookRunner, HookSource, HookStatus};
