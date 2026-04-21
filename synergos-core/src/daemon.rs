//! デーモンライフサイクル管理
//!
//! synergos-core デーモンの起動・常駐・シャットダウンを制御する。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use synergos_net::{
    config::NetConfig,
    conduit::Conduit,
    dht::DhtNode,
    gossip::GossipNode,
    mesh::Mesh,
    quic::QuicManager,
    tunnel::TunnelManager,
    types::PeerId,
};

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

/// ネットワーク基盤のハンドル群（Daemon が保持して寿命を合わせる）
pub struct NetworkHandles {
    pub local_peer_id: PeerId,
    pub net_config: Arc<NetConfig>,
    pub dht: Arc<DhtNode>,
    pub gossip: Arc<GossipNode>,
    pub quic: Arc<QuicManager>,
    pub tunnel: Arc<TunnelManager>,
    pub mesh: Arc<Mesh>,
    pub conduit: Arc<Conduit>,
}

/// Synergos Core デーモン
pub struct Daemon {
    ctx: Arc<ServiceContext>,
    net: Arc<NetworkHandles>,
}

impl Daemon {
    /// デーモンを初期化する
    pub async fn new(config: DaemonConfig) -> anyhow::Result<Self> {
        // ── 設定ファイルの読み込み（任意） ──
        let net_config = load_net_config(config.config_path.as_deref())?;
        let net_config = Arc::new(net_config);

        // ── ローカル識別子（永続化は別途：TODO） ──
        let local_peer_id = PeerId::generate();

        // ── ネットワーク基盤コンポーネント ──
        let dht = Arc::new(DhtNode::new(
            local_peer_id.clone(),
            net_config.dht.clone(),
        ));
        let gossip = Arc::new(GossipNode::new(
            local_peer_id.clone(),
            net_config.gossipsub.clone(),
        ));
        let quic = Arc::new(QuicManager::new(net_config.quic.clone()));
        let tunnel = Arc::new(TunnelManager::new(&net_config.tunnel));
        let mesh = Arc::new(Mesh::new(net_config.mesh.clone()));
        let conduit = Arc::new(Conduit::new(
            quic.clone(),
            tunnel.clone(),
            mesh.clone(),
            Duration::from_secs(15),
        ));

        let net = Arc::new(NetworkHandles {
            local_peer_id: local_peer_id.clone(),
            net_config,
            dht: dht.clone(),
            gossip: gossip.clone(),
            quic,
            tunnel,
            mesh,
            conduit,
        });

        // ── サービスレイヤ ──
        let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
        let project_manager =
            Arc::new(ProjectManager::with_gossip(event_bus.clone(), Some(gossip.clone())));
        let exchange = Arc::new(Exchange::with_network(
            event_bus.clone(),
            local_peer_id.clone(),
            Some(gossip.clone()),
        ));
        let presence = Arc::new(PresenceService::with_network(
            event_bus.clone(),
            Some(dht.clone()),
            Some(gossip.clone()),
        ));
        let conflict_manager = Arc::new(ConflictManager::new(event_bus.clone()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let ctx = Arc::new(ServiceContext {
            event_bus,
            project_manager,
            exchange,
            presence,
            conflict_manager,
            shutdown_tx,
            started_at,
        });

        Ok(Self { ctx, net })
    }

    /// デーモンを実行する（ブロッキング）
    ///
    /// IPC サーバーを起動し、シャットダウンシグナルまで常駐する。
    pub async fn run(self) -> anyhow::Result<()> {
        tracing::info!(
            "Synergos core daemon started (PID: {}, peer_id={})",
            std::process::id(),
            self.net.local_peer_id,
        );

        let ipc_server = IpcServer::new(self.ctx.clone());

        let shutdown_tx = self.ctx.shutdown_tx.clone();

        // OS シグナルハンドラを設定
        let signal_task = tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            tracing::info!("Shutdown signal received");
            let _ = shutdown_tx.send(());
        });

        // ── 定期メンテナンスタスク ──
        let cleanup_task = spawn_periodic_cleanup(self.ctx.clone(), self.net.clone());

        // Gossipsub ハートビート
        let heartbeat_task = spawn_gossip_heartbeat(
            self.net.gossip.clone(),
            self.net.net_config.gossipsub.heartbeat_interval_ms,
            self.ctx.shutdown_tx.subscribe(),
        );

        // IPC サーバーを実行（シャットダウンまでブロック）
        ipc_server.run().await?;

        // タスクをクリーンアップ
        signal_task.abort();
        cleanup_task.abort();
        heartbeat_task.abort();

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

        // 4. ネットワーク基盤のシャットダウン
        self.net.conduit.shutdown().await;
        self.net.quic.shutdown().await;
        let _ = self.net.tunnel.stop().await;
        self.net.mesh.cleanup_expired_sessions();

        Ok(())
    }
}

/// TOML ネットワーク設定ファイルを読み込む。指定なし／存在しない場合はデフォルト。
fn load_net_config(path: Option<&std::path::Path>) -> anyhow::Result<NetConfig> {
    match path {
        Some(p) if p.exists() => {
            tracing::info!("Loading net config from {}", p.display());
            let text = std::fs::read_to_string(p)?;
            let cfg: NetConfig = toml::from_str(&text)?;
            Ok(cfg)
        }
        Some(p) => {
            tracing::warn!(
                "Config file not found at {}; using defaults",
                p.display()
            );
            Ok(NetConfig::default())
        }
        None => Ok(NetConfig::default()),
    }
}

/// 定期クリーンアップタスク（招待トークン GC、転送 GC、DHT GC、DNS/TURN セッション GC）
fn spawn_periodic_cleanup(
    ctx: Arc<ServiceContext>,
    net: Arc<NetworkHandles>,
) -> tokio::task::JoinHandle<()> {
    let mut shutdown_rx = ctx.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // ConflictManager の既存クリーンアップを統合
                    ctx.conflict_manager.cleanup_resolved();
                    ctx.conflict_manager.cleanup_expired_notifications(24 * 60 * 60 * 1000);

                    // 招待トークン GC
                    let n = ctx.project_manager.gc_expired_invites();
                    if n > 0 {
                        tracing::debug!("Expired {} invite token(s)", n);
                    }

                    // 完了転送 GC
                    let n = ctx.exchange.gc_finished_transfers();
                    if n > 0 {
                        tracing::debug!("Reaped {} finished transfer(s)", n);
                    }

                    // DHT の期限切れレコード
                    net.dht.gc_expired();

                    // TURN セッション GC
                    net.mesh.cleanup_expired_sessions();

                    // QUIC → Presence の RTT / 帯域情報を転記 (#14 対策)。
                    // quinn の RTT 推定は connection レベルでしか取れないので、
                    // QuicManager が保持しているスナップショットを 60s ごとに
                    // PresenceService に流す。
                    for info in net.quic.list_connections() {
                        ctx.presence.update_rtt(&info.peer_id, info.rtt_ms);
                        let bw = net.quic.get_bandwidth_estimate(&info.peer_id);
                        if bw > 0 {
                            ctx.presence.update_bandwidth(&info.peer_id, bw);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    })
}

/// Gossipsub ハートビート
fn spawn_gossip_heartbeat(
    gossip: Arc<GossipNode>,
    interval_ms: u64,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    // 過剰に短い／長い設定を安全側に丸める
    let interval_ms = interval_ms.clamp(100, 60_000);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    gossip.heartbeat();
                }
                _ = shutdown_rx.recv() => {
                    break;
                }
            }
        }
    })
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
