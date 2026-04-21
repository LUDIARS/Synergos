//! 設定パネル

use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection) {
    ui.heading("Settings");
    ui.separator();

    let cache = connection.cache.lock().unwrap();

    // デーモン状態
    ui.collapsing("Daemon", |ui| {
        egui::Grid::new("daemon_settings")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Status:");
                if connection.is_connected() {
                    ui.colored_label(egui::Color32::from_rgb(0, 180, 0), "Running");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(180, 0, 0), "Not running");
                }
                ui.end_row();

                if let Some(status) = &cache.status {
                    ui.label("PID:");
                    ui.label(format!("{}", status.pid));
                    ui.end_row();

                    ui.label("Projects:");
                    ui.label(format!("{}", status.project_count));
                    ui.end_row();

                    ui.label("Connections:");
                    ui.label(format!("{}", status.active_connections));
                    ui.end_row();

                    ui.label("Transfers:");
                    ui.label(format!("{}", status.active_transfers));
                    ui.end_row();
                }
            });
    });

    ui.add_space(8.0);

    // ネットワーク設定（読み取り専用の表示）
    ui.collapsing("Network", |ui| {
        if let Some(net) = &cache.network {
            egui::Grid::new("network_settings")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Primary Route:");
                    ui.label(&net.primary_route);
                    ui.end_row();

                    ui.label("Max Connections:");
                    ui.label(format!("{}", net.max_connections));
                    ui.end_row();

                    ui.label("Avg Latency:");
                    ui.label(format!("{} ms", net.avg_latency_ms));
                    ui.end_row();
                });
        } else {
            ui.label("Network settings not available.");
        }
    });

    ui.add_space(8.0);

    // ── ネットワーク設定 編集 (#13) ──
    ui.collapsing("Network Tuning", |ui| {
        let id = ui.make_persistent_id("synergos_net_tuning");
        let (mut mesh_n, mut max_streams, mut hostname, mut feedback) = ui.memory_mut(|m| {
            m.data
                .get_persisted::<(String, String, String, String)>(id)
                .unwrap_or_default()
        });

        egui::Grid::new("network_tuning_form")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("gossipsub.mesh_n:");
                ui.text_edit_singleline(&mut mesh_n);
                ui.end_row();

                ui.label("quic.max_concurrent_streams:");
                ui.text_edit_singleline(&mut max_streams);
                ui.end_row();

                ui.label("tunnel.hostname:");
                ui.text_edit_singleline(&mut hostname);
                ui.end_row();
            });

        if ui.button("Apply").clicked() {
            let parsed_mesh = mesh_n.trim().parse::<u16>().ok();
            let parsed_max = max_streams.trim().parse::<u32>().ok();
            let parsed_host = if hostname.trim().is_empty() {
                None
            } else {
                Some(hostname.trim().to_string())
            };
            let ok = connection.update_config(parsed_mesh, parsed_max, parsed_host);
            feedback = if ok {
                "Applied (restart daemon for full effect)".into()
            } else {
                "Update rejected".into()
            };
        }

        ui.memory_mut(|m| {
            m.data
                .insert_persisted(id, (mesh_n, max_streams, hostname, feedback.clone()))
        });

        if !feedback.is_empty() {
            ui.add_space(4.0);
            ui.colored_label(egui::Color32::from_rgb(120, 120, 180), &feedback);
        }
    });

    ui.add_space(8.0);

    // 転送設定（説明のみ）
    ui.collapsing("Transfer Policy", |ui| {
        egui::Grid::new("transfer_settings")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Chunk Size (Large):");
                ui.label("1 MiB");
                ui.end_row();

                ui.label("Chunk Size (Medium):");
                ui.label("256 KiB");
                ui.end_row();

                ui.label("Chunk Size (Small):");
                ui.label("64 KiB");
                ui.end_row();

                ui.label("Bandwidth Ratio:");
                ui.label("Large 60% / Medium 30% / Small 10%");
                ui.end_row();
            });
    });

    ui.add_space(8.0);

    // バージョン情報
    ui.collapsing("About", |ui| {
        ui.label("Synergos GUI v0.1.0");
        ui.label("A standalone GUI for the Synergos collaboration platform.");
        ui.add_space(4.0);
        ui.label("Synergos — Greek for \"working together\"");
    });
}
