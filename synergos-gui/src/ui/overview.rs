//! ネットワーク概要パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection) {
    ui.heading("Network Overview");
    ui.separator();

    let cache = connection.cache.lock().unwrap();

    // ネットワーク状態
    if let Some(net) = &cache.network {
        egui::Grid::new("overview_grid")
            .num_columns(2)
            .spacing([20.0, 8.0])
            .show(ui, |ui| {
                ui.label("Status:");
                if connection.is_connected() {
                    ui.colored_label(egui::Color32::from_rgb(0, 180, 0), "Connected");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(180, 0, 0), "Disconnected");
                }
                ui.end_row();

                ui.label("Route:");
                ui.label(&net.primary_route);
                ui.end_row();

                ui.label("Active Peers:");
                ui.label(format!("{}/{}", net.active_connections, net.max_connections));
                ui.end_row();

                ui.label("Bandwidth:");
                ui.label(format_bandwidth(net.total_bandwidth_bps));
                ui.end_row();

                ui.label("Used Bandwidth:");
                ui.label(format_bandwidth(net.used_bandwidth_bps));
                ui.end_row();

                ui.label("Latency:");
                ui.label(format!("{} ms", net.avg_latency_ms));
                ui.end_row();
            });
    } else {
        ui.label("No network data available.");
        if !connection.is_connected() {
            ui.colored_label(
                egui::Color32::from_rgb(180, 120, 0),
                "Daemon not running. Start synergos-core to connect.",
            );
        }
    }

    ui.add_space(16.0);

    // プロジェクト一覧
    ui.heading("Projects");
    ui.separator();

    if cache.projects.is_empty() {
        ui.label("No active projects.");
    } else {
        egui::Grid::new("projects_grid")
            .num_columns(4)
            .spacing([16.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Path");
                ui.strong("Peers");
                ui.strong("Transfers");
                ui.end_row();

                for p in &cache.projects {
                    ui.label(&p.display_name);
                    ui.label(&p.root_path);
                    ui.label(format!("{}", p.peer_count));
                    ui.label(format!("{}", p.active_transfers));
                    ui.end_row();
                }
            });
    }
}

fn format_bandwidth(bps: u64) -> String {
    if bps >= 1_000_000_000 {
        format!("{:.1} Gbps", bps as f64 / 1_000_000_000.0)
    } else if bps >= 1_000_000 {
        format!("{:.1} Mbps", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.1} Kbps", bps as f64 / 1_000.0)
    } else {
        format!("{} bps", bps)
    }
}
