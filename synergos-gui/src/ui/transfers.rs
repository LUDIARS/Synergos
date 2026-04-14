//! 転送状況パネル

use crate::app::UiInputs;
use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection, inputs: &mut UiInputs) {
    ui.heading("Transfers");
    ui.separator();

    // 転送要求フォーム
    egui::CollapsingHeader::new("Request new transfer")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("transfer_request_form")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Project ID:");
                    ui.text_edit_singleline(&mut inputs.transfer_project);
                    ui.end_row();

                    ui.label("File ID:");
                    ui.text_edit_singleline(&mut inputs.transfer_file_id);
                    ui.end_row();

                    ui.label("Peer ID:");
                    ui.text_edit_singleline(&mut inputs.transfer_peer_id);
                    ui.end_row();
                });
            if ui.button("Request").clicked() {
                let pid = inputs.transfer_project.trim().to_string();
                let fid = inputs.transfer_file_id.trim().to_string();
                let peer = inputs.transfer_peer_id.trim().to_string();
                if pid.is_empty() || fid.is_empty() || peer.is_empty() {
                    inputs.set_err("project / file / peer are all required");
                } else if connection.request_transfer(&pid, &fid, &peer) {
                    inputs.set_ok("transfer queued");
                } else {
                    inputs.set_err("TransferRequest failed");
                }
            }
        });

    ui.add_space(8.0);

    // リフレッシュ
    connection.refresh_transfers();

    let transfers = {
        let cache = connection.cache.lock().unwrap();
        cache.transfers.clone()
    };

    if transfers.is_empty() {
        ui.label("No active transfers.");
        ui.add_space(8.0);
        ui.label("File transfers will appear here when sharing or receiving files.");
        return;
    }

    egui::Grid::new("transfers_grid")
        .num_columns(7)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Dir");
            ui.strong("File");
            ui.strong("Size");
            ui.strong("Progress");
            ui.strong("Speed");
            ui.strong("State");
            ui.strong("Actions");
            ui.end_row();

            for t in &transfers {
                // 方向アイコン
                let dir_icon = if t.direction == "upload" {
                    "↑"
                } else {
                    "↓"
                };
                ui.label(dir_icon);

                ui.label(&t.file_name);
                ui.label(format_size(t.file_size));

                let progress = if t.file_size > 0 {
                    t.bytes_transferred as f32 / t.file_size as f32
                } else {
                    0.0
                };
                let bar =
                    egui::ProgressBar::new(progress).text(format!("{:.0}%", progress * 100.0));
                ui.add_sized([120.0, 16.0], bar);

                ui.label(format_speed(t.speed_bps));

                let (color, text) = state_display(&t.state);
                ui.colored_label(color, text);

                if ui.button("Cancel").clicked() {
                    if connection.cancel_transfer(&t.transfer_id) {
                        inputs.set_ok("cancel requested");
                    } else {
                        inputs.set_err("cancel failed");
                    }
                }
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
