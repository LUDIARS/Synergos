//! 設定パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Settings");
    ui.separator();

    ui.label("Settings panel — coming soon.");

    // TODO: Phase 5 で実装
    // - ネットワーク設定（帯域制限、転送並列度）
    // - Tunnel 設定
    // - ピア選択ポリシー
    // - デーモン設定（自動起動、ログレベル）
}
