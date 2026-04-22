//! Synergos WebSocket Relay Server (Issue #24)。
//!
//! Protocol は `synergos-net/src/relay/mod.rs` のクライアントと互換。
//!
//! クライアント → サーバ:
//! 1. 最初の text frame: `{"type":"join","room_id":"<uuid>","peer_id":"<peer>"}`
//! 2. 以降の binary frame: msgpack `RelayData { from_peer_id, payload }`
//!
//! サーバ → クライアント:
//! - 同じ room_id の他クライアントから届いた binary frame をそのまま転送
//!   (送信者自身には返さない)
//!
//! 本実装は TCP/WS のみ。TLS を被せるのは逆プロキシ (nginx / Caddy) 想定。
//! 直接 TLS を待ち受けたい場合は別途 `rustls` を足すが、運用の柔軟性を優先
//! して本体は平文 WS にする。

pub mod config;
pub mod server;

pub use config::RelayConfig;
pub use server::{run, ServerHandle};
