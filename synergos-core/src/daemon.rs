//! デーモンライフサイクル管理
//!
//! synergos-core デーモンの起動・常駐・シャットダウンを制御する。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use crate::conflict::ConflictManager;
use crate::event_bus::{CoreEventBus, SharedEventBus};
use crate::exchange::{Exchange, FileSharing};
use crate::ipc_server::{IpcServer, ServiceContext};
use crate::presence::{NodeRegistry, PresenceService};
use crate::project::ProjectManager;

/// デーモン設定
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// ネットワーク設定ファイルパス
    pub config_path: Option<PathBuf>,
    /// ログレベル
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            config_path: None,
            log_level: "info".to_string(),
        }
    }
}

/// Synergos Core デーモン
pub struct Daemon {
    ctx: Arc<ServiceContext>,
}

impl Daemon {
    /// デーモンを初期化する
    pub async fn new(config: DaemonConfig) -> anyhow::Result<Self> {
        let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
        let project_manager = Arc::new(ProjectManager::new(event_bus.clone()));
        let exchange = Arc::new(Exchange::new(event_bus.clone()));
        let presence = Arc::new(PresenceService::new(event_bus.clone()));
        let conflict_manager = Arc::new(ConflictManager::new(event_bus.clone()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 設定ファイルの読み込み
        if let Some(config_path) = &config.config_path {
            if config_path.exists() {
                tracing::info!("Loading config from {}", config_path.display());
                // 設定ファイルの内容は synergos-net の NetConfig として読み込む
                // 現時点ではデフォルト設定を使用
            }
        }

        let ctx = Arc::new(ServiceContext {
            event_bus,
            project_manager,
            exchange,
            presence,
            conflict_manager,
            shutdown_tx,
            started_at,
        });

        Ok(Self { ctx })
    }

    /// デーモンを実行する（ブロッキング）
    ///
    /// IPC サーバーを起動し、シャットダウンシグナルまで常駐する。
    pub async fn run(self) -> anyhow::Result<()> {
        tracing::info!(
            "Synergos core daemon started (PID: {})",
            std::process::id()
        );

        let ipc_server = IpcServer::new(self.ctx.clone());

        let shutdown_tx = self.ctx.shutdown_tx.clone();

        // OS シグナルハンドラを設定
        let signal_task = tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            tracing::info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
        });

        // コンフリクトマネージャの定期クリーンアップタスク
        let conflict_manager = self.ctx.conflict_manager.clone();
        let mut cleanup_shutdown_rx = self.ctx.shutdown_tx.subscribe();
        let cleanup_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        conflict_manager.cleanup_resolved();
                        // 24時間以上経過した通知を削除
                        conflict_manager.cleanup_expired_notifications(24 * 60 * 60 * 1000);
                    }
                    _ = cleanup_shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        // IPC サーバーを実行（シャットダウンまでブロック）
        ipc_server.run().await?;

        // タスクをクリーンアップ
        signal_task.abort();
        cleanup_task.abort();

        // グレースフルシャットダウン
        self.shutdown().await?;

        tracing::info!("Synergos core daemon stopped");
        Ok(())
    }

    /// グレースフルシャットダウン
    async fn shutdown(self) -> anyhow::Result<()> {
        tracing::info!("Performing graceful shutdown...");

        // 1. アクティブプロジェクトをすべて閉じる
        self.ctx.project_manager.close_all().await;

        // 2. Presence で自ノードを離脱
        let _ = self.ctx.presence.unregister_self().await;

        // 3. アクティブ転送を全キャンセル
        let active = self.ctx.exchange.list_transfers(None).await;
        for transfer in active {
            if transfer.state == crate::exchange::TransferState::Running
                || transfer.state == crate::exchange::TransferState::Queued
            {
                let _ = self.ctx.exchange.cancel_transfer(&transfer.transfer_id).await;
            }
        }

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
