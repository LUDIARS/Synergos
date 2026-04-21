//! デーモンライフサイクル管理
//!
//! synergos-core デーモンの起動・常駐・シャットダウンを制御する。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use synergos_net::{
    conduit::Conduit,
    config::NetConfig,
    dht::{handle_dht_stream, DhtNode, DHT_STREAM_MAGIC},
    gossip::GossipNode,
    identity::Identity,
    mesh::Mesh,
    quic::QuicManager,
    transfer::TRANSFER_STREAM_MAGIC,
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
#[allow(dead_code)]
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
    pub identity: Arc<Identity>,
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
        net_config
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid net config: {e}"))?;
        let net_config = Arc::new(net_config);

        // ── ローカル識別子（ed25519 キー + BLAKE3 派生 PeerId を永続化） ──
        let identity = Arc::new(
            Identity::load_or_generate(&Identity::default_path())
                .map_err(|e| anyhow::anyhow!("identity init failed: {e}"))?,
        );
        let local_peer_id = identity.peer_id().clone();

        // ── ネットワーク基盤コンポーネント ──
        let dht = Arc::new(DhtNode::new(local_peer_id.clone(), net_config.dht.clone()));
        let gossip = Arc::new(GossipNode::new(
            local_peer_id.clone(),
            net_config.gossipsub.clone(),
        ));
        let quic = Arc::new(QuicManager::new(net_config.quic.clone(), identity.clone()));
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
            identity,
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
        let project_manager = Arc::new(ProjectManager::with_gossip(
            event_bus.clone(),
            Some(gossip.clone()),
        ));
        let mut exchange_inner = Exchange::with_network(
            event_bus.clone(),
            local_peer_id.clone(),
            Some(gossip.clone()),
        );
        // QUIC と受信先レゾルバを注入する。
        // リゾルバは ProjectManager からプロジェクトルートを引いて FileId を
        // 相対パスとして連結する (実運用では file_id に path 情報を持たせる
        // 必要あり — 暫定で FileId をそのまま相対パスとして扱う)。
        {
            let pm = project_manager.clone();
            let resolver: crate::exchange::OutPathResolver =
                Arc::new(move |project_id, file_id| {
                    let root = pm.project_root(project_id)?;
                    Some(root.join(file_id.0.clone()))
                });
            exchange_inner.attach_quic(net.quic.clone(), resolver);
        }
        let exchange = Arc::new(exchange_inner);
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

        // QUIC 着信ストリームディスパッチャ (DHT / Transfer を magic で振り分け)
        let quic_accept_task = spawn_quic_accept_loop(
            self.net.clone(),
            self.ctx.clone(),
            self.ctx.shutdown_tx.subscribe(),
        );

        // Gossip サブスクライバ: FileWant / FileOffer を Exchange に配線する。
        // これで Synergos 既存の gossip チャネルだけで自動更新がドライブする。
        let gossip_sub_task = spawn_gossip_subscriber(
            self.net.gossip.clone(),
            self.ctx.clone(),
            self.ctx.shutdown_tx.subscribe(),
        );

        // IPC サーバーを実行（シャットダウンまでブロック）
        ipc_server.run().await?;

        // タスクをクリーンアップ
        signal_task.abort();
        cleanup_task.abort();
        heartbeat_task.abort();
        quic_accept_task.abort();
        gossip_sub_task.abort();

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
                let _ = self
                    .ctx
                    .exchange
                    .cancel_transfer(&transfer.transfer_id)
                    .await;
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
            tracing::warn!("Config file not found at {}; using defaults", p.display());
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

                    // Route Migration: 各接続済みピアに対して try_migrate_route を呼び、
                    // より高優先度の経路が開通していれば切り替える (S-route)。
                    // QUIC のアクティブリストを元にするのでオフラインピアは触らない。
                    for info in net.quic.list_connections() {
                        match net.conduit.try_migrate_route(&info.peer_id).await {
                            Ok(Some(new_route)) => {
                                tracing::debug!(
                                    "route migrated for {}: {:?}",
                                    info.peer_id.short(),
                                    new_route
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::debug!(
                                    "route migration attempt for {} failed: {e}",
                                    info.peer_id.short()
                                );
                            }
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

/// QUIC 着信ストリームを受け付け、magic バイトで DHT / Transfer にディスパッチする。
///
/// 今のところ `QuicManager::accept` で受け取った Connection から `accept_bi()` を
/// ループで拾い、1 bidi ストリームごとに:
///   1. 先頭 4 byte を読んで magic を判定
///   2. DHT なら `handle_dht_stream` に委譲
///   3. Transfer なら `Exchange::handle_incoming_transfer` に委譲
fn spawn_quic_accept_loop(
    net: Arc<NetworkHandles>,
    ctx: Arc<ServiceContext>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                accepted = net.quic.accept() => {
                    match accepted {
                        Ok(Some(acc)) => {
                            let dht = net.dht.clone();
                            let exchange = ctx.exchange.clone();
                            let sender = acc.peer_id.clone();
                            let connection = acc.connection;
                            tokio::spawn(async move {
                                dispatch_peer_streams(connection, sender, dht, exchange).await;
                            });
                        }
                        Ok(None) => {
                            tracing::debug!("quic accept returned None (endpoint closed); exiting loop");
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("quic accept error: {e}");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
    })
}

/// 1 つの QUIC コネクションから bidi ストリームをループで受け、先頭 magic で
/// DHT / Transfer に振り分ける。コネクションが閉じられたらループを抜ける。
async fn dispatch_peer_streams(
    connection: quinn::Connection,
    sender: PeerId,
    dht: Arc<DhtNode>,
    exchange: Arc<crate::exchange::Exchange>,
) {
    loop {
        let (send, mut recv) = match connection.accept_bi().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::debug!("connection {} closed: {e}", sender.short());
                return;
            }
        };

        let mut magic = [0u8; 4];
        if let Err(e) = recv.read_exact(&mut magic).await {
            tracing::debug!("stream magic read failed from {}: {e}", sender.short());
            continue;
        }

        let dht = dht.clone();
        let exchange = exchange.clone();
        let sender_cloned = sender.clone();
        tokio::spawn(async move {
            if &magic == DHT_STREAM_MAGIC {
                if let Err(e) = handle_dht_stream(dht, send, recv).await {
                    tracing::debug!("DHT stream handler error: {e}");
                }
            } else if &magic == TRANSFER_STREAM_MAGIC {
                if let Err(e) = exchange.handle_incoming_transfer(recv, sender_cloned).await {
                    tracing::debug!("Transfer stream handler error: {e}");
                }
            } else {
                tracing::debug!("unknown stream magic {:?}", magic);
            }
        });
    }
}

/// Gossip メッセージサブスクライバ。`GossipNode::receiver()` で購読し、
/// `FileWant` / `FileOffer` を `Exchange::handle_file_want` /
/// `handle_file_offer` に流す。
///
/// これにより「誰かがファイル A を Want した」→「A を持っている自分が
/// gossip 経由で気付く」→「QUIC の `TXFR` ストリームを開いて実データを
/// 送る」という完全な自動更新チェーンが Synergos 既存の gossip + ledger +
/// conduit 経路だけで成立する。IPC からの直接コマンドには依存しない。
fn spawn_gossip_subscriber(
    gossip: Arc<GossipNode>,
    ctx: Arc<ServiceContext>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    let mut rx = gossip.receiver();
    tokio::spawn(async move {
        use synergos_net::gossip::GossipMessage;
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                msg = rx.recv() => {
                    match msg {
                        Ok((_topic, GossipMessage::FileWant { requester, file_id, version })) => {
                            ctx.exchange.handle_file_want(requester, file_id, version);
                        }
                        Ok((_topic, GossipMessage::FileOffer { sender, file_id, version, size, crc, content_hash: _ })) => {
                            ctx.exchange.handle_file_offer(sender, file_id, version, size, crc);
                        }
                        Ok(_) => {
                            // CatalogUpdate / PeerStatus / ConflictAlert は別系統で処理
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("gossip subscriber lagged; dropped {n} messages");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
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
