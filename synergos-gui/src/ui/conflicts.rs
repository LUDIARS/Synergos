//! コンフリクト管理パネル
//!
//! `ConflictList` IPC でアクティブコンフリクトを取得し、各行に
//! [Keep Local] / [Accept Remote] / [Manual Merge] の解決ボタンを描画する。

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection) {
    ui.horizontal(|ui| {
        ui.heading("Conflicts");
        if ui.button("Refresh").clicked() {
            connection.refresh_conflicts(None);
        }
    });
    ui.separator();

    // キャッシュからコンフリクト一覧を取得
    let (conflicts, _stale) = {
        let cache = connection.cache.lock().unwrap();
        let stale = cache
            .last_refresh
            .map(|t| t.elapsed().as_secs() > 5)
            .unwrap_or(true);
        (cache.conflicts.clone(), stale)
    };

    if conflicts.is_empty() {
        ui.label("No active conflicts.");
        ui.add_space(8.0);
        ui.label("When file conflicts are detected between peers, they will appear here.");
        render_help(ui);
        return;
    }

    // 一覧テーブル
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("conflicts_table")
            .num_columns(5)
            .striped(true)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.strong("File");
                ui.strong("Local v.");
                ui.strong("Remote v.");
                ui.strong("State");
                ui.strong("Actions");
                ui.end_row();

                for c in &conflicts {
                    ui.label(&c.file_path);
                    ui.label(format!(
                        "{} ({})",
                        c.local_version,
                        truncate(&c.local_author)
                    ));
                    ui.label(format!(
                        "{} ({})",
                        c.remote_version,
                        truncate(&c.remote_author)
                    ));
                    ui.label(&c.state);
                    ui.horizontal(|ui| {
                        if ui.button("Keep Local").clicked()
                            && connection.resolve_conflict(&c.file_id, "keep_local")
                        {
                            connection.refresh_conflicts(None);
                        }
                        if ui.button("Accept Remote").clicked()
                            && connection.resolve_conflict(&c.file_id, "accept_remote")
                        {
                            connection.refresh_conflicts(None);
                        }
                        if ui.button("Manual").clicked()
                            && connection.resolve_conflict(&c.file_id, "manual_merge")
                        {
                            connection.refresh_conflicts(None);
                        }
                    });
                    ui.end_row();
                }
            });
    });

    render_help(ui);
}

fn truncate(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..12])
    } else {
        id.to_string()
    }
}

fn render_help(ui: &mut egui::Ui) {
    ui.add_space(16.0);
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
