//! synergos-core ライブラリ公開面。
//! 統合テスト / 他クレートから Exchange, ProjectManager 等にアクセスするためのエントリポイント。

pub mod catalog_sync;
pub mod checkout;
pub mod cli;
pub mod cli_history;
pub mod cli_hooks;
pub mod conflict;
pub mod control_heartbeat;
pub mod event_bus;
pub mod exchange;
pub mod invite_token;
pub mod peer_bootstrap;
pub mod peer_info_server;
pub mod peer_join;
pub mod presence;
pub mod project;

pub mod daemon;
pub mod history;
pub mod hooks;
pub mod ipc_server;
pub mod manifest;
pub mod restore;
pub mod version_state;
