//! ピア一覧パネル

use crate::app::UiInputs;
use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection, inputs: &mut UiInputs) {
    ui.heading("Peers");
    ui.separator();

    // ── 手動接続フォーム ──
    egui::CollapsingHeader::new("Connect to peer")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("peer_connect_form")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Project ID:");
                    ui.text_edit_singleline(&mut inputs.peer_target_project);
                    ui.end_row();

                    ui.label("Peer ID:");
                    ui.text_edit_singleline(&mut inputs.new_peer_id);
                    ui.end_row();
                });
            if ui.button("Connect").clicked() {
                let pid = inputs.peer_target_project.trim().to_string();
                let peer = inputs.new_peer_id.trim().to_string();
                if pid.is_empty() || peer.is_empty() {
                    inputs.set_err("project id / peer id required");
                } else if connection.peer_connect(&pid, &peer) {
                    inputs.set_ok(format!("connect requested: {peer}"));
                } else {
                    inputs.set_err("peer_connect failed");
                }
            }
            // 現在表示中のピアリストから refresh 用ボタン
            if ui.button("Refresh for this project").clicked() {
                let pid = inputs.peer_target_project.trim().to_string();
                if !pid.is_empty() {
                    connection.refresh_peers(&pid);
                    inputs.set_ok("refreshed");
                }
            }
        });

    ui.add_space(8.0);

    let peers = {
        let cache = connection.cache.lock().unwrap();
        cache.peers.clone()
    };

    if peers.is_empty() {
        ui.label("No connected peers.");
        ui.add_space(8.0);
        ui.label("Peers will appear here when connected to a project.");
        return;
    }

    egui::Grid::new("peers_grid")
        .num_columns(7)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Peer ID");
            ui.strong("Route");
            ui.strong("RTT");
            ui.strong("Bandwidth");
            ui.strong("State");
            ui.strong("Actions");
            ui.end_row();

            for peer in &peers {
                ui.label(&peer.display_name);
                ui.label(truncate_id(&peer.peer_id));
                ui.label(&peer.route);
                ui.label(format!("{} ms", peer.rtt_ms));
                ui.label(format_bandwidth(peer.bandwidth_bps));
                let (color, icon) = state_color_icon(&peer.state);
                ui.colored_label(color, format!("{} {}", icon, peer.state));
                if ui.button("Disconnect").clicked() {
                    if connection.peer_disconnect(&peer.peer_id) {
                        inputs.set_ok(format!("disconnected {}", truncate_id(&peer.peer_id)));
                    } else {
                        inputs.set_err("peer_disconnect failed");
                    }
                }
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
