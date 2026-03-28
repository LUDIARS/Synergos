//! コンフリクト管理パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Conflicts");
    ui.separator();

    ui.label("No active conflicts.");

    // TODO: Phase 5 で実装
    // - アクティブコンフリクト一覧
    // - 解決操作（KeepLocal / AcceptRemote / ManualMerge）
    // - 差分ビューワ
}
