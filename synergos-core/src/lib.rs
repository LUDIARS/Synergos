//! synergos-core ライブラリ公開面。
//! 統合テスト / 他クレートから Exchange, ProjectManager 等にアクセスするためのエントリポイント。

pub mod cli;
pub mod conflict;
pub mod event_bus;
pub mod exchange;
pub mod presence;
pub mod project;

pub mod daemon;
pub mod ipc_server;
