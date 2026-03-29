//! ピア一覧パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection) {
    ui.heading("Peers");
    ui.separator();

    let cache = connection.cache.lock().unwrap();

    if cache.peers.is_empty() {
        ui.label("No connected peers.");
        ui.add_space(8.0);
        ui.label("Peers will appear here when connected to a project.");
        return;
    }

    egui::Grid::new("peers_grid")
        .num_columns(6)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Peer ID");
            ui.strong("Route");
            ui.strong("RTT");
            ui.strong("Bandwidth");
            ui.strong("State");
            ui.end_row();

            for peer in &cache.peers {
                ui.label(&peer.display_name);
                ui.label(truncate_id(&peer.peer_id));
                ui.label(&peer.route);
                ui.label(format!("{} ms", peer.rtt_ms));
                ui.label(format_bandwidth(peer.bandwidth_bps));
                // 状態に応じた色付きラベル
                let (color, icon) = state_color_icon(&peer.state);
                ui.colored_label(color, format!("{} {}", icon, peer.state));
                ui.end_row();
            }
        });
}

fn truncate_id(id: &str) -> String {
    if id.len() > 12 {
        format!("{}...", &id[..12])
    } else {
        id.to_string()
    }
}

fn format_bandwidth(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.0} Mbps", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.0} Kbps", bps as f64 / 1_000.0)
    } else if bps > 0 {
        format!("{} bps", bps)
    } else {
        "—".to_string()
    }
}

fn state_color_icon(state: &str) -> (egui::Color32, &'static str) {
    match state {
        s if s.contains("Connected") => (egui::Color32::from_rgb(0, 180, 0), "●"),
        s if s.contains("Idle") => (egui::Color32::from_rgb(180, 180, 0), "●"),
        s if s.contains("Discovered") => (egui::Color32::from_rgb(100, 100, 255), "○"),
        s if s.contains("Connecting") => (egui::Color32::from_rgb(100, 100, 255), "◌"),
        s if s.contains("Away") => (egui::Color32::from_rgb(180, 120, 0), "●"),
        _ => (egui::Color32::from_rgb(150, 150, 150), "●"),
    }
}
