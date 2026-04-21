//! ars-plugin-synergos: Ars リアルタイムコラボレーションプラグイン
//!
//! synergos-core デーモンへの**薄い IPC アダプタ**として機能する。
//! ネットワーク基盤を直接利用するのではなく、IPC 経由で synergos-core と通信し、
//! Ars EventBus とのブリッジのみを担う。
//!
//! ## アーキテクチャ上の位置づけ
//!
//! ```text
//! Ars EventBus ←→ [ars-plugin-synergos] ←IPC→ [synergos-core daemon]
//! ```
//!
//! IDE 内蔵の git 機能が git コマンドを呼び出すのと同様に、
//! このプラグインは synergos-core デーモンに IPC コマンドを送信する。

pub mod bridge;
pub mod events;

// Re-export IPC types
pub use synergos_ipc;
