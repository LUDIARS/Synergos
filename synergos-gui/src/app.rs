//! アプリケーション状態・メインループ

use std::path::PathBuf;

use crate::connection::CoreConnection;
use crate::ui;

/// Synergos GUI アプリケーション
pub struct SynergosApp {
    /// synergos-core への接続
    connection: CoreConnection,
    /// 現在選択中のタブ
    active_tab: Tab,
    /// Overview タブ: プロジェクト開閉 / 招待生成のための入力欄
    pub inputs: UiInputs,
}

/// UI に渡すユーザ入力状態。各タブで使う textedit のバッキング。
#[derive(Default, Clone)]
pub struct UiInputs {
    pub new_project_id: String,
    pub new_project_root: String,
    pub selected_project_id: String,
    pub invite_expires_secs: String, // 空 or 数値
    pub last_invite_token: Option<String>,
    pub invite_error: Option<String>,

    /// Peers タブ
    pub new_peer_id: String,
    pub peer_target_project: String,

    /// Transfers タブ
    pub transfer_project: String,
    pub transfer_file_id: String,
    pub transfer_peer_id: String,

    /// 通知バナー
    pub toast: Option<Toast>,
}

#[derive(Clone, Debug)]
pub enum Toast {
    Ok(String),
    Err(String),
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
            inputs: UiInputs::default(),
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

                // トースト通知
                if let Some(toast) = &self.inputs.toast.clone() {
                    ui.add_space(16.0);
                    match toast {
                        Toast::Ok(m) => ui.colored_label(
                            egui::Color32::from_rgb(0, 180, 0),
                            format!("✓ {m}"),
                        ),
                        Toast::Err(m) => ui.colored_label(
                            egui::Color32::from_rgb(200, 60, 60),
                            format!("✗ {m}"),
                        ),
                    };
                    if ui.small_button("✕").clicked() {
                        self.inputs.toast = None;
                    }
                }
            });
        });

        // メインコンテンツ
        egui::CentralPanel::default().show(ctx, |ui| match self.active_tab {
            Tab::Overview => ui::overview::show(ui, &self.connection, &mut self.inputs),
            Tab::Peers => ui::peers::show(ui, &self.connection, &mut self.inputs),
            Tab::Transfers => ui::transfers::show(ui, &self.connection, &mut self.inputs),
            Tab::Conflicts => ui::conflicts::show(ui, &self.connection),
            Tab::Settings => ui::settings::show(ui, &self.connection),
        });

        // 2 秒ごとに再描画をリクエスト
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }
}

impl UiInputs {
    pub fn set_ok(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::Ok(msg.into()));
    }
    pub fn set_err(&mut self, msg: impl Into<String>) {
        self.toast = Some(Toast::Err(msg.into()));
    }

    pub fn new_project_root_path(&self) -> Option<PathBuf> {
        let trimmed = self.new_project_root.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    }
}
