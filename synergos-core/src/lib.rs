//! synergos-core ライブラリ公開面。
//! 統合テスト / 他クレートから Exchange, ProjectManager 等にアクセスするためのエントリポイント。

pub mod catalog_sync;
pub mod cli;
pub mod conflict;
pub mod event_bus;
pub mod exchange;
pub mod peer_bootstrap;
pub mod peer_info_server;
pub mod presence;
pub mod project;

pub mod daemon;
pub mod ipc_server;
