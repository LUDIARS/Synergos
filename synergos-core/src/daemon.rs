//! デーモンライフサイクル管理
//!
//! synergos-core デーモンの起動・常駐・シャットダウンを制御する。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use synergos_net::{
    conduit::Conduit,
    config::NetConfig,
    content::{handle_bitswap_stream, MemoryContentStore, BITSWAP_STREAM_MAGIC},
    dht::{handle_dht_stream, DhtNode, DHT_STREAM_MAGIC},
    gossip::{
        handle_gossip_stream, send_gossip, GossipNode, GossipWireMessage, GOSSIP_STREAM_MAGIC,
    },
    identity::Identity,
    mesh::Mesh,
    promotion::{NetCapabilities, PromotionMode},
    quic::{QuicManager, StreamType},
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
#[derive(Debug, Clone, Default)]
pub struct DaemonConfig {
    /// ネットワーク設定ファイルパス
    pub config_path: Option<PathBuf>,
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
        let gossip = {
            let mut g = GossipNode::new(local_peer_id.clone(), net_config.gossipsub.clone());
            g.set_identity(identity.clone());
            Arc::new(g)
        };
        let quic = Arc::new(QuicManager::new(net_config.quic.clone(), identity.clone()));
        // QUIC server をバインド (これが無いと accept ループは未開通でピアを受けられない)。
        // 既定は `[::]:0` (IPv6 デュアルスタックでカーネル割当ポート)。
        // 公開ノードでは `quic.listen_addr = "[::]:7777"` 等を config で指定する。
        let bind_addr: SocketAddr = net_config
            .quic
            .listen_addr
            .unwrap_or_else(|| "[::]:0".parse().expect("static literal"));
        let actual_addr = quic
            .bind(bind_addr)
            .await
            .map_err(|e| anyhow::anyhow!("QUIC bind failed on {bind_addr}: {e}"))?;
        tracing::info!("QUIC listening on {}", actual_addr);
        let tunnel = Arc::new(TunnelManager::new(&net_config.tunnel));
        let mesh = Arc::new(Mesh::new(net_config.mesh.clone()));

        // 起動時 NAT 越え probe → effective relay-only を決定する。
        //   - force_relay_only=true なら問答無用で relay-only
        //   - auto_promote=true (既定) かつ probe で direct/tunnel いずれも不可
        //     なら relay-only として起動 (= 「ノード昇格できない」状態を自己診断)
        //   - それ以外は通常モード (Direct→Tunnel→Relay の優先試行)
        let effective_relay_only = if net_config.force_relay_only {
            tracing::info!("force_relay_only=true → relay-only mode (probe skipped)");
            true
        } else if net_config.auto_promote {
            let caps = NetCapabilities::detect(
                Duration::from_secs(3),
                !net_config.tunnel.api_token_ref.is_empty(),
                false, // relay endpoint 設定有無 (現状は判定不能、後続 PR で正確化)
            )
            .await;
            tracing::info!("net capabilities: {}", caps.summary());
            match caps.recommended_mode() {
                PromotionMode::FullNode => false,
                PromotionMode::RelayOnly => {
                    tracing::warn!(
                        "auto_promote: no direct/tunnel reachable → starting in relay-only mode"
                    );
                    true
                }
            }
        } else {
            false
        };

        let conduit = Arc::new(Conduit::with_relay_only(
            quic.clone(),
            tunnel.clone(),
            mesh.clone(),
            Duration::from_secs(15),
            effective_relay_only,
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
        // プロジェクト永続化: identity と同じ dir の下に projects.json
        let state_path = Identity::default_path()
            .parent()
            .map(|p| p.join("projects.json"))
            .unwrap_or_else(|| std::path::PathBuf::from("projects.json"));
        let project_manager = Arc::new(ProjectManager::with_state_path(
            event_bus.clone(),
            Some(gossip.clone()),
            state_path.clone(),
        ));
        // 既存 state を restore: persisted なプロジェクトを open し直す。
        if let Ok(persisted) = project_manager.load_state().await {
            use crate::project::ProjectConfiguration;
            for p in persisted {
                if let Err(e) = project_manager
                    .open_project(
                        p.project_id.clone(),
                        p.root_path.clone(),
                        Some(p.display_name.clone()),
                    )
                    .await
                {
                    tracing::warn!(
                        "failed to restore project {} from {}: {e}",
                        p.project_id,
                        state_path.display()
                    );
                }
            }
        }
        let mut exchange_inner = Exchange::with_network(
            event_bus.clone(),
            local_peer_id.clone(),
            Some(gossip.clone()),
        );
        // QUIC と受信先レゾルバを注入する。
        // リゾルバは `ProjectManager::resolve_file_path` に委譲し、
        // 事前 register_file された file_id→path マッピングを優先する。
        // 未登録の場合は FileId をそのまま相対パスとしてフォールバック。
        {
            let pm = project_manager.clone();
            let resolver: crate::exchange::OutPathResolver =
                Arc::new(move |project_id, file_id| pm.resolve_file_path(project_id, file_id));
            exchange_inner.attach_quic(net.quic.clone(), resolver);
        }
        // Bitswap 用 ContentStore: ServiceContext と同じインスタンスを
        // Exchange にも流して、publish_updates 時に RootCatalog snapshot を
        // put → BSW1 経路で相手が引けるようにする (#25/#26)。
        let shared_content_store = Arc::new(synergos_net::content::MemoryContentStore::new());
        exchange_inner.attach_content_store(shared_content_store.clone());
        let exchange = Arc::new(exchange_inner);
        let presence = Arc::new(PresenceService::with_network_and_mode(
            event_bus.clone(),
            Some(dht.clone()),
            Some(gossip.clone()),
            effective_relay_only,
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
            net_config: Some(net.net_config.clone()),
            catalogs: Arc::new(dashmap::DashMap::new()),
            content_store: shared_content_store,
            quic: net.quic.clone(),
        });

        // 永続化されていた project を restore した後、それぞれの
        // CatalogManager も同時に立ち上げる。
        use crate::project::ProjectConfiguration;
        for info in ProjectConfiguration::list_projects(&*ctx.project_manager) {
            ctx.catalogs.insert(
                info.project_id.clone(),
                Arc::new(synergos_net::catalog::CatalogManager::new(
                    info.project_id.clone(),
                    net.net_config.catalog.chunk_max_files,
                    net.net_config.catalog.chain_max_depth,
                )),
            );
        }

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

        // Gossip fan-out: publish() が outbound_rx に流す OutboundGossip を
        // QUIC 上の GSP1 ストリームで各メッシュピアに配る。
        let gossip_fanout_task = spawn_gossip_fanout(
            self.net.gossip.clone(),
            self.net.quic.clone(),
            self.ctx.shutdown_tx.subscribe(),
        );

        // CatalogSyncService: CatalogSyncNeededEvent を購読して BitswapSession で
        // publisher から RootCatalog スナップショットを取りに行き、
        // CatalogSyncCompletedEvent を emit する (#25/#26)。
        let catalog_sync_task = crate::catalog_sync::CatalogSyncService::spawn(
            self.ctx.event_bus.clone(),
            self.net.quic.clone(),
            self.ctx.content_store.clone(),
            self.ctx.shutdown_tx.subscribe(),
        );

        // Peer-info HTTP servlet (任意): bootstrap 公開ノード用。Cloudflare Tunnel
        // 経由で /peer-info を外部に publish し、クライアントは GET → quic_endpoint
        // を学習して直接 QUIC 接続する。`peer_info_listen_addr = None` ならスキップ。
        let peer_info_task = self
            .net
            .net_config
            .peer_info_listen_addr
            .map(|listen_addr| {
                let peer_id = self.net.local_peer_id.clone();
                let quic = self.net.quic.clone();
                let shutdown_rx = self.ctx.shutdown_tx.subscribe();
                tokio::spawn(async move {
                    if let Err(e) = crate::peer_info_server::run(
                        listen_addr,
                        peer_id,
                        quic,
                        "synergos-core".into(),
                        shutdown_rx,
                    )
                    .await
                    {
                        tracing::warn!("peer-info servlet exited with error: {e}");
                    }
                })
            });

        // bootstrap_urls: 起動時に config 指定の peer-info URL に自動接続する。
        // 失敗は warn で記録するだけで daemon 起動は継続する (best-effort)。
        let bootstrap_task = if !self.net.net_config.bootstrap_urls.is_empty() {
            let urls = self.net.net_config.bootstrap_urls.clone();
            let quic = self.net.quic.clone();
            let presence = self.ctx.presence.clone();
            Some(tokio::spawn(async move {
                for url in urls {
                    match crate::peer_bootstrap::bootstrap_from_url(
                        &url,
                        &quic,
                        std::time::Duration::from_secs(10),
                    )
                    .await
                    {
                        Ok(peer_id) => {
                            tracing::info!(
                                "bootstrap connected to {url}: peer_id={}",
                                peer_id.short()
                            );
                            let registration = crate::presence::NodeRegistration {
                                peer_id: peer_id.clone(),
                                display_name: peer_id.to_string(),
                                endpoints: vec![],
                                project_ids: vec![],
                            };
                            if let Err(e) = presence.register_node(registration).await {
                                tracing::warn!("bootstrap {url}: register_node failed: {e}");
                                continue;
                            }
                            let _ = presence
                                .update_node_state(&peer_id, crate::presence::PeerState::Connected)
                                .await;
                        }
                        Err(e) => {
                            tracing::warn!("bootstrap {url} failed: {e}");
                        }
                    }
                }
            }))
        } else {
            None
        };

        // IPC サーバーを実行（シャットダウンまでブロック）
        ipc_server.run().await?;

        // タスクをクリーンアップ
        signal_task.abort();
        cleanup_task.abort();
        heartbeat_task.abort();
        quic_accept_task.abort();
        gossip_sub_task.abort();
        gossip_fanout_task.abort();
        catalog_sync_task.abort();
        if let Some(t) = peer_info_task {
            t.abort();
        }
        if let Some(t) = bootstrap_task {
            t.abort();
        }

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
                            let gossip = net.gossip.clone();
                            let exchange = ctx.exchange.clone();
                            let content_store = ctx.content_store.clone();
                            let sender = acc.peer_id.clone();
                            let connection = acc.connection;
                            tokio::spawn(async move {
                                dispatch_peer_streams(
                                    connection,
                                    sender,
                                    dht,
                                    gossip,
                                    exchange,
                                    content_store,
                                )
                                .await;
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
/// DHT / Gossip / Transfer / Bitswap に振り分ける。コネクションが閉じられたらループを抜ける。
async fn dispatch_peer_streams(
    connection: quinn::Connection,
    sender: PeerId,
    dht: Arc<DhtNode>,
    gossip: Arc<GossipNode>,
    exchange: Arc<crate::exchange::Exchange>,
    content_store: Arc<MemoryContentStore>,
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
        let gossip = gossip.clone();
        let exchange = exchange.clone();
        let content_store = content_store.clone();
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
            } else if &magic == GOSSIP_STREAM_MAGIC {
                drop(send); // gossip は片方向相当 (応答なし)
                if let Err(e) = handle_gossip_stream(gossip, recv, sender_cloned).await {
                    tracing::debug!("Gossip stream handler error: {e}");
                }
            } else if &magic == BITSWAP_STREAM_MAGIC {
                if let Err(e) = handle_bitswap_stream(content_store, send, recv).await {
                    tracing::debug!("Bitswap stream handler error: {e}");
                }
            } else {
                tracing::debug!("unknown stream magic {:?}", magic);
            }
        });
    }
}

/// 新ピア接続時に dispatch_peer_streams を spawn する箇所の呼び出し更新。
/// spawn_quic_accept_loop は `dispatch_peer_streams` に `gossip` 引数を
/// 渡す必要があるので、そちらも NetworkHandles 全体を引き回すよう修正する。
fn spawn_gossip_fanout(
    gossip: Arc<GossipNode>,
    quic: Arc<QuicManager>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    let mut rx = gossip.outbound_receiver();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                out = rx.recv() => {
                    match out {
                        Ok(outbound) => {
                            // 各メッシュピアに対して独立 stream を開いて送信する。
                            // publish() が返す peers は空のこともある (mesh 未形成) —
                            // その場合は fallback として「現在 QUIC 接続中の全ピア」に
                            // 流す。mesh が立ち上がる前の早期 publish を救う緩和策。
                            let peers: Vec<PeerId> = if outbound.peers.is_empty() {
                                quic.list_connections()
                                    .into_iter()
                                    .map(|c| c.peer_id)
                                    .collect()
                            } else {
                                outbound.peers.clone()
                            };
                            for peer in peers {
                                let wire = GossipWireMessage {
                                    topic: outbound.topic.clone(),
                                    signed: outbound.signed.clone(),
                                };
                                let quic = quic.clone();
                                tokio::spawn(async move {
                                    match quic.open_stream(&peer, StreamType::Control).await {
                                        Ok((send, _recv)) => {
                                            if let Err(e) = send_gossip(send, &wire).await {
                                                tracing::debug!(
                                                    "gossip send to {} failed: {e}",
                                                    peer.short()
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                "gossip open_stream to {} failed: {e}",
                                                peer.short()
                                            );
                                        }
                                    }
                                });
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("gossip fanout lagged; dropped {n} messages");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    })
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
                        Ok((_topic, GossipMessage::PeerStatus { status, origin, .. })) => {
                            ctx.presence.handle_peer_status(&status, &origin);
                        }
                        Ok((_topic, GossipMessage::ConflictAlert { file_id, conflicting_nodes, their_versions })) => {
                            ctx.conflict_manager.handle_conflict_alert(file_id, conflicting_nodes, their_versions);
                        }
                        Ok((_topic, GossipMessage::CatalogUpdate { project_id, root_crc, update_count, updated_chunks, catalog_cid, publisher })) => {
                            // #26: リモートの root_crc とローカルの root_catalog を比較する。
                            // 異なる場合は CatalogSyncNeededEvent を emit し、更新対象の
                            // chunk IDs と catalog_cid を EventBus 購読者 (CatalogSyncService)
                            // に通知する。購読者側が BitswapSession で実チャンクを取りに行く。
                            let local_match = match ctx.catalogs.get(&project_id) {
                                Some(cm) => {
                                    let local = cm.root_catalog().await;
                                    local.catalog_crc == root_crc
                                        && local.update_count == update_count
                                }
                                None => {
                                    // 未 open のプロジェクトなので対象外
                                    true
                                }
                            };
                            if !local_match {
                                tracing::info!(
                                    "catalog drift detected: project={project_id} remote_crc={root_crc:x} remote_updates={update_count} changed_chunks={} catalog_cid={:?}",
                                    updated_chunks.len(),
                                    catalog_cid.as_ref().map(|c| c.0.as_str())
                                );
                                ctx.event_bus.emit(
                                    crate::event_bus::CatalogSyncNeededEvent {
                                        project_id: project_id.clone(),
                                        remote_root_crc: root_crc,
                                        remote_update_count: update_count,
                                        changed_chunks: updated_chunks
                                            .iter()
                                            .map(|c| c.0.clone())
                                            .collect(),
                                        catalog_cid: catalog_cid.clone(),
                                        publisher: if publisher.0.is_empty() {
                                            None
                                        } else {
                                            Some(publisher.0.clone())
                                        },
                                    },
                                );
                            } else {
                                tracing::debug!(
                                    "gossip CatalogUpdate (in sync): project={project_id}"
                                );
                            }
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
