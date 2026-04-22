//! synergos-gui: Synergos 専用 GUI アプリケーション
//!
//! synergos-core デーモンに IPC 接続し、ネットワーク状態・ピア管理・
//! ファイル転送・コンフリクト解決を GUI で操作する。
//!
//! git に対する SourceTree のような位置づけ。Ars には一切依存しない。

use synergos_gui::app;

use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    // ロギング初期化
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("synergos_gui=info")),
        )
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Synergos")
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Synergos",
        options,
        Box::new(|cc| Ok(Box::new(app::SynergosApp::new(cc)))),
    )
}
