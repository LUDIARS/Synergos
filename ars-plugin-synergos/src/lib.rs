//! ars-plugin-synergos: Ars リアルタイムコラボレーションプラグイン
//!
//! synergos-net の上に構築される Ars 固有のプラグインレイヤ。
//! ProjectModule trait を実装し、EventBus を通じて他モジュールと通信する。

pub mod conflict;
pub mod events;
pub mod exchange;
pub mod presence;

// Re-export synergos-net types
pub use synergos_net;
