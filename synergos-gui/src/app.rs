//! アプリケーション状態・メインループ

use crate::connection::CoreConnection;
use crate::ui;

/// Synergos GUI アプリケーション
pub struct SynergosApp {
    /// synergos-core への接続
    connection: CoreConnection,
    /// 現在選択中のタブ
    active_tab: Tab,
}

/// メインタブ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Peers,
    Transfers,
    Conflicts,
    Settings,
}

impl SynergosApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            connection: CoreConnection::new(),
            active_tab: Tab::Overview,
        }
    }
}

impl eframe::App for SynergosApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 定期的にデータをリフレッシュ
        self.connection.refresh_if_needed();

        // トップバー
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.heading("Synergos");
                ui.separator();

                ui.selectable_value(&mut self.active_tab, Tab::Overview, "Overview");
                ui.selectable_value(&mut self.active_tab, Tab::Peers, "Peers");
                ui.selectable_value(&mut self.active_tab, Tab::Transfers, "Transfers");
                ui.selectable_value(&mut self.active_tab, Tab::Conflicts, "Conflicts");
                ui.selectable_value(&mut self.active_tab, Tab::Settings, "Settings");
            });
        });

        // ステータスバー
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (status_text, color) = if self.connection.is_connected() {
                    let cache = self.connection.cache.lock().unwrap();
                    let info = match &cache.status {
                        Some(s) => format!(
                            "Core: Connected (PID {}) | Projects: {} | Connections: {} | Transfers: {}",
                            s.pid, s.project_count, s.active_connections, s.active_transfers
                        ),
                        None => "Core: Connected".to_string(),
                    };
                    (info, egui::Color32::from_rgb(0, 180, 0))
                } else {
                    ("Core: Disconnected".to_string(), egui::Color32::from_rgb(180, 0, 0))
                };
                ui.colored_label(color, status_text);
            });
        });

        // メインコンテンツ
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                Tab::Overview => ui::overview::show(ui, &self.connection),
                Tab::Peers => ui::peers::show(ui, &self.connection),
                Tab::Transfers => ui::transfers::show(ui, &self.connection),
                Tab::Conflicts => ui::conflicts::show(ui, &self.connection),
                Tab::Settings => ui::settings::show(ui, &self.connection),
            }
        });

        // 2秒ごとに再描画をリクエスト
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }
}
