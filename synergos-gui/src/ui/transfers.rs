//! 転送状況パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Transfers");
    ui.separator();

    ui.label("No active transfers.");

    // TODO: Phase 5 で実装
    // - アクティブ転送テーブル（ファイル名、サイズ、進捗、速度、方向）
    // - プログレスバー
    // - キャンセルボタン
    // - 転送履歴
}
