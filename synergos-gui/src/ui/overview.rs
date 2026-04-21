//! ネットワーク概要パネル

use crate::app::UiInputs;
use crate::connection::CoreConnection;

pub fn show(ui: &mut egui::Ui, connection: &CoreConnection, inputs: &mut UiInputs) {
    ui.heading("Network Overview");
    ui.separator();

    // Network status
    {
        let cache = connection.cache.lock().unwrap();
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
    }

    ui.add_space(16.0);

    // ── Projects (操作系 UI) ──
    ui.heading("Projects");
    ui.separator();

    egui::CollapsingHeader::new("Open new project")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("open_project_form")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Project ID:");
                    ui.text_edit_singleline(&mut inputs.new_project_id);
                    ui.end_row();

                    ui.label("Root path:");
                    ui.text_edit_singleline(&mut inputs.new_project_root);
                    ui.end_row();
                });
            if ui.button("Open project").clicked() {
                let pid = inputs.new_project_id.trim().to_string();
                let root = inputs.new_project_root_path();
                match root {
                    Some(root) if !pid.is_empty() => {
                        if connection.open_project(&pid, &root) {
                            inputs.set_ok(format!("opened project {pid}"));
                            inputs.new_project_id.clear();
                            inputs.new_project_root.clear();
                        } else {
                            inputs.set_err(format!("open_project {pid} failed"));
                        }
                    }
                    _ => inputs.set_err("project id / root is empty"),
                }
            }
        });

    ui.add_space(8.0);

    let projects = {
        let cache = connection.cache.lock().unwrap();
        cache.projects.clone()
    };
    if projects.is_empty() {
        ui.label("No active projects.");
    } else {
        egui::Grid::new("projects_grid")
            .num_columns(5)
            .spacing([16.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Path");
                ui.strong("Peers");
                ui.strong("Transfers");
                ui.strong("Actions");
                ui.end_row();

                for p in &projects {
                    ui.label(&p.display_name);
                    ui.label(&p.root_path);
                    ui.label(format!("{}", p.peer_count));
                    ui.label(format!("{}", p.active_transfers));
                    ui.horizontal(|ui| {
                        if ui.button("Invite").clicked() {
                            inputs.selected_project_id = p.project_id.clone();
                            let expires =
                                inputs.invite_expires_secs.trim().parse::<u64>().ok();
                            match connection.create_invite(&p.project_id, expires) {
                                Some((token, _exp)) => {
                                    inputs.last_invite_token = Some(token);
                                    inputs.invite_error = None;
                                    inputs.set_ok(format!(
                                        "invite created for {}",
                                        p.project_id
                                    ));
                                }
                                None => {
                                    inputs.invite_error = Some(format!(
                                        "invite failed for {}",
                                        p.project_id
                                    ));
                                    inputs.set_err("invite failed");
                                }
                            }
                        }
                        if ui.button("Close").clicked() {
                            if connection.close_project(&p.project_id) {
                                inputs.set_ok(format!("closed {}", p.project_id));
                            } else {
                                inputs.set_err(format!("close {} failed", p.project_id));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
    }

    ui.add_space(16.0);

    egui::CollapsingHeader::new("Invite token")
        .default_open(inputs.last_invite_token.is_some())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Expires (secs, blank=never):");
                ui.text_edit_singleline(&mut inputs.invite_expires_secs);
            });
            if let Some(token) = inputs.last_invite_token.clone() {
                ui.label("Latest token:");
                let mut token_mut = token.clone();
                ui.add(
                    egui::TextEdit::singleline(&mut token_mut)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                if ui.button("Copy").clicked() {
                    ui.output_mut(|o| o.copied_text = token.clone());
                    inputs.set_ok("copied to clipboard");
                }
            } else if let Some(err) = inputs.invite_error.clone() {
                ui.colored_label(egui::Color32::from_rgb(200, 60, 60), err);
            } else {
                ui.label("No invite generated yet. Click [Invite] on a project row.");
            }
        });
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
