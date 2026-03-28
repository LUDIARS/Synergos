//! synergos-core への IPC 接続管理

/// Core デーモンへの接続状態
pub struct CoreConnection {
    connected: bool,
    // TODO: IpcClient インスタンス
    // TODO: イベント受信タスク
}

impl CoreConnection {
    pub fn new() -> Self {
        Self { connected: false }
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    // TODO: Phase 5 で実装
    // - 自動接続・再接続
    // - コマンド送信
    // - イベント受信・UI更新通知
}
