//! ネットワーク概要パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Network Overview");
    ui.separator();

    egui::Grid::new("overview_grid")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            ui.label("Status:");
            ui.label("—");
            ui.end_row();

            ui.label("Route:");
            ui.label("—");
            ui.end_row();

            ui.label("Active Peers:");
            ui.label("0");
            ui.end_row();

            ui.label("Bandwidth:");
            ui.label("— bps");
            ui.end_row();

            ui.label("Latency:");
            ui.label("— ms");
            ui.end_row();
        });

    // TODO: Phase 5 で実装
    // - リアルタイム帯域グラフ
    // - 接続状態インジケータ
}
