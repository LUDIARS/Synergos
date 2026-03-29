//! コンフリクト管理パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, _connection: &CoreConnection) {
    ui.heading("Conflicts");
    ui.separator();

    ui.label("No active conflicts.");
    ui.add_space(8.0);

    ui.label("When file conflicts are detected between peers, they will appear here.");
    ui.add_space(16.0);

    // コンフリクト解決方法の説明
    ui.collapsing("Resolution Methods", |ui| {
        egui::Grid::new("conflict_help")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.strong("Keep Local");
                ui.label("Adopt your local version and push it to the network");
                ui.end_row();

                ui.strong("Accept Remote");
                ui.label("Accept the remote version and overwrite local changes");
                ui.end_row();

                ui.strong("Manual Merge");
                ui.label("Manually merge both versions and create a new version");
                ui.end_row();
            });
    });
}
