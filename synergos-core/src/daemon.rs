//! デーモンライフサイクル管理
//!
//! synergos-core デーモンの起動・常駐・シャットダウンを制御する。

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::event_bus::{CoreEventBus, SharedEventBus};
use crate::ipc_server::IpcServer;
use crate::project::ProjectManager;

/// Synergos Core デーモン
pub struct Daemon {
    event_bus: SharedEventBus,
    project_manager: Arc<ProjectManager>,
    shutdown_tx: broadcast::Sender<()>,
}

impl Daemon {
    /// デーモンを初期化する
    pub async fn new(_config_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
        let project_manager = Arc::new(ProjectManager::new(event_bus.clone()));
        let (shutdown_tx, _) = broadcast::channel(1);

        // TODO: 設定ファイル読み込み
        // TODO: synergos-net の初期化

        Ok(Self {
            event_bus,
            project_manager,
            shutdown_tx,
        })
    }

    /// デーモンを実行する（ブロッキング）
    ///
    /// IPC サーバーを起動し、シャットダウンシグナルまで常駐する。
    pub async fn run(self) -> anyhow::Result<()> {
        tracing::info!(
            "Synergos core daemon started (PID: {})",
            std::process::id()
        );

        let ipc_server = IpcServer::new(
            self.event_bus.clone(),
            self.project_manager.clone(),
            self.shutdown_tx.clone(),
        );

        let shutdown_tx = self.shutdown_tx.clone();

        // OS シグナルハンドラを設定
        let signal_task = tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            tracing::info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
        });

        // IPC サーバーを実行（シャットダウンまでブロック）
        ipc_server.run().await?;

        // シグナルタスクをクリーンアップ
        signal_task.abort();

        // グレースフルシャットダウン
        self.shutdown().await?;

        tracing::info!("Synergos core daemon stopped");
        Ok(())
    }

    /// グレースフルシャットダウン
    async fn shutdown(self) -> anyhow::Result<()> {
        tracing::info!("Performing graceful shutdown...");

        // 1. アクティブプロジェクトをすべて閉じる
        self.project_manager.close_all().await;

        // 2. TODO: ネットワーク接続のクリーンアップ
        // 3. TODO: アクティブ転送の完了待ち

        Ok(())
    }
}

/// OS シャットダウンシグナルを待機する
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to register SIGTERM");
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to register SIGINT");
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
    }

    #[cfg(windows)]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to register Ctrl+C handler");
    }
}
