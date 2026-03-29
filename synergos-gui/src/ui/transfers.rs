//! 転送状況パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection) {
    ui.heading("Transfers");
    ui.separator();

    // リフレッシュ
    connection.refresh_transfers();

    let cache = connection.cache.lock().unwrap();

    if cache.transfers.is_empty() {
        ui.label("No active transfers.");
        ui.add_space(8.0);
        ui.label("File transfers will appear here when sharing or receiving files.");
        return;
    }

    egui::Grid::new("transfers_grid")
        .num_columns(6)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Dir");
            ui.strong("File");
            ui.strong("Size");
            ui.strong("Progress");
            ui.strong("Speed");
            ui.strong("State");
            ui.end_row();

            for t in &cache.transfers {
                // 方向アイコン
                let dir_icon = if t.direction == "upload" { "↑" } else { "↓" };
                ui.label(dir_icon);

                // ファイル名
                ui.label(&t.file_name);

                // サイズ
                ui.label(format_size(t.file_size));

                // プログレスバー
                let progress = if t.file_size > 0 {
                    t.bytes_transferred as f32 / t.file_size as f32
                } else {
                    0.0
                };
                let bar = egui::ProgressBar::new(progress)
                    .text(format!("{:.0}%", progress * 100.0));
                ui.add_sized([120.0, 16.0], bar);

                // 速度
                ui.label(format_speed(t.speed_bps));

                // 状態
                let (color, text) = state_display(&t.state);
                ui.colored_label(color, text);
                ui.end_row();
            }
        });
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_speed(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.0} MB/s", bps as f64 / 1_000_000.0 / 8.0)
    } else if bps >= 1_000 {
        format!("{:.0} KB/s", bps as f64 / 1_000.0 / 8.0)
    } else if bps > 0 {
        format!("{} B/s", bps / 8)
    } else {
        "—".to_string()
    }
}

fn state_display(state: &str) -> (egui::Color32, &str) {
    match state {
        "Running" => (egui::Color32::from_rgb(0, 180, 0), "Active"),
        "Queued" => (egui::Color32::from_rgb(100, 100, 255), "Queued"),
        "Paused" => (egui::Color32::from_rgb(180, 180, 0), "Paused"),
        "Completed" => (egui::Color32::from_rgb(100, 200, 100), "Done"),
        "Cancelled" => (egui::Color32::from_rgb(150, 150, 150), "Cancelled"),
        _ if state.contains("Failed") => (egui::Color32::from_rgb(200, 0, 0), "Failed"),
        _ => (egui::Color32::from_rgb(150, 150, 150), state),
    }
}
