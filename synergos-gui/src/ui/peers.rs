//! ピア一覧パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Peers");
    ui.separator();

    ui.label("No connected peers.");

    // TODO: Phase 5 で実装
    // - ピア一覧テーブル（名前、ルート、RTT、帯域、状態）
    // - 接続・切断ボタン
    // - ピア詳細ダイアログ
}
