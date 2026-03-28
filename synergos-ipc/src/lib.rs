#![allow(dead_code, unused_imports, unused_variables)]
//! synergos-ipc: Shared IPC protocol types for Synergos
//!
//! synergos-core デーモンと各クライアント（GUI / CLI / Ars Plugin）間の
//! 通信プロトコルを定義する。プラットフォーム非依存の型定義と、
//! クロスプラットフォーム IPC トランスポート抽象化を提供する。

pub mod client;
pub mod command;
pub mod event;
pub mod response;
pub mod transport;

pub use client::IpcClient;
pub use command::IpcCommand;
pub use event::{EventFilter, IpcEvent};
pub use response::IpcResponse;
pub use transport::{IpcError, IpcTransport};
