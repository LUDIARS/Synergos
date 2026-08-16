//! 履歴ノード (history node) の E2E (docs/versioning-design.md §3):
//!
//!   1. A (通常) が v1 を publish → B (履歴ノード) が受信し保管庫に入れる
//!   2. v2 も同様。B の保管庫に v1/v2 が揃い、A には保管庫ができない
//!   3. A が `restore --version 1` → FileWant(v1) → B が保管庫から v1 を送る →
//!      A は手元 v2 より古い v1 を (pin 済みなので) 受け入れ、manifest も v1 に戻る
//!   4. 巻き戻し後の publish は観測済み最大版を飛び越えて v3 になる (版番号の衝突防止)
//!   5. 履歴ノード自身の restore は保管庫からローカルで差し替わる
//!
//! ネットワークは versioned_republish_e2e と同じ実 QUIC + gossip 配線。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_core::checkout::{restore_file, CheckoutContext, RestoreOutcome};
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FetchRequest, FileSharing, IncomingDirResolver, OutPathResolver, PublishNotification,
    ReceivedHook, TransferPriority,
};
use synergos_core::history::HistoryStore;
use synergos_core::manifest::{crc32_of_file, normalize_rel_path, BumpOutcome, ProjectManifest};
use synergos_core::project::{ProjectConfiguration, ProjectManager};
use synergos_net::config::HistoryConfig;
use synergos_net::config::{GossipsubConfig, QuicConfig};
use synergos_net::gossip::{
    handle_gossip_stream, send_gossip, GossipNode, GossipWireMessage, GOSSIP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};
use synergos_net::transfer::TRANSFER_STREAM_MAGIC;
use synergos_net::types::{FileId, TopicId};

const PROJECT: &str = "hist-e2e";

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 10_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

fn gcfg() -> GossipsubConfig {
    GossipsubConfig {
        mesh_n: 6,
        mesh_n_low: 4,
        mesh_n_high: 12,
        heartbeat_interval_ms: 1000,
        message_cache_size: 256,
    }
}

struct Node {
    identity: Arc<Identity>,
    quic: Arc<QuicManager>,
    gossip: Arc<GossipNode>,
    exchange: Arc<Exchange>,
    project_manager: Arc<ProjectManager>,
    history: Arc<HistoryStore>,
    root: std::path::PathBuf,
}

/// daemon.rs と同じ配線 (resolver / incoming dir / received hook) で Exchange を作る。
fn build_exchange(
    event_bus: SharedEventBus,
    identity: &Arc<Identity>,
    gossip: &Arc<GossipNode>,
    quic: &Arc<QuicManager>,
    pm: &Arc<ProjectManager>,
    history: &Arc<HistoryStore>,
) -> Arc<Exchange> {
    let mut ex =
        Exchange::with_network(event_bus, identity.peer_id().clone(), Some(gossip.clone()));
    let pm1 = pm.clone();
    let resolver: OutPathResolver =
        Arc::new(move |project_id, fid: &FileId| pm1.resolve_file_path(project_id, fid));
    ex.attach_quic(quic.clone(), resolver);
    let pm2 = pm.clone();
    let incoming: IncomingDirResolver = Arc::new(move |project_id| {
        pm2.project_root(project_id)
            .map(|r| r.join(".synergos").join("incoming"))
    });
    let pm3 = pm.clone();
    let received: ReceivedHook = Arc::new(
        move |project_id, file_id, version, size, crc, sender, pinned| {
            let pm = pm3.clone();
            Box::pin(async move {
                let _ = pm
                    .record_received_file(
                        &project_id,
                        &file_id.0,
                        version,
                        size,
                        crc,
                        &sender.0,
                        pinned,
                    )
                    .await;
            })
        },
    );
    ex.attach_receive_hooks(incoming, received);
    if history.enabled() {
        ex.attach_history_hooks(synergos_core::history::wiring::build_hooks(
            history.clone(),
            pm.clone(),
        ));
    }
    Arc::new(ex)
}

impl Node {
    async fn bind(root: std::path::PathBuf, history_node: bool) -> (Self, std::net::SocketAddr) {
        let identity = Arc::new(Identity::generate());
        let quic = Arc::new(QuicManager::new(qcfg(), identity.clone()));
        let addr = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
        let gossip = {
            let mut g = GossipNode::new(identity.peer_id().clone(), gcfg());
            g.set_identity(identity.clone());
            Arc::new(g)
        };
        let event_bus: SharedEventBus = Arc::new(CoreEventBus::new());
        let project_manager = Arc::new(ProjectManager::with_gossip(
            event_bus.clone(),
            Some(gossip.clone()),
        ));
        project_manager
            .open_project(PROJECT.into(), root.clone(), None)
            .await
            .unwrap();
        let history = Arc::new(HistoryStore::new(HistoryConfig {
            enabled: history_node,
            ..HistoryConfig::default()
        }));
        let exchange = build_exchange(
            event_bus,
            &identity,
            &gossip,
            &quic,
            &project_manager,
            &history,
        );
        (
            Self {
                identity,
                quic,
                gossip,
                exchange,
                project_manager,
                history,
                root,
            },
            addr,
        )
    }

    fn spawn_accept(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                let acc = match me.quic.accept().await {
                    Ok(Some(a)) => a,
                    _ => break,
                };
                let me2 = me.clone();
                tokio::spawn(async move {
                    while let Ok((send, mut recv)) = acc.connection.accept_bi().await {
                        let mut magic = [0u8; 4];
                        if recv.read_exact(&mut magic).await.is_err() {
                            continue;
                        }
                        let gossip = me2.gossip.clone();
                        let exchange = me2.exchange.clone();
                        let sender = acc.peer_id.clone();
                        tokio::spawn(async move {
                            if &magic == GOSSIP_STREAM_MAGIC {
                                drop(send);
                                let _ = handle_gossip_stream(gossip, recv, sender).await;
                            } else if &magic == TRANSFER_STREAM_MAGIC {
                                let _ = exchange.handle_incoming_transfer(recv, sender).await;
                            }
                        });
                    }
                });
            }
        })
    }

    /// daemon.rs の subscriber / fanout と同じロジック (auto-pull 込み)。
    fn spawn_gossip(
        self: &Arc<Self>,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let me_sub = self.clone();
        let sub = tokio::spawn(async move {
            let mut rx = me_sub.gossip.receiver();
            while let Ok((topic, msg)) = rx.recv().await {
                use synergos_net::gossip::GossipMessage;
                match msg {
                    GossipMessage::FileWant {
                        requester,
                        file_id,
                        version,
                    } => me_sub
                        .exchange
                        .handle_file_want(PROJECT, requester, file_id, version),
                    GossipMessage::FileOffer {
                        sender,
                        file_id,
                        version,
                        size,
                        crc,
                        ..
                    } => {
                        me_sub.exchange.handle_file_offer(
                            sender.clone(),
                            file_id.clone(),
                            version,
                            size,
                            crc,
                        );
                        let mine = sender == *me_sub.exchange.local_peer_id();
                        let already = me_sub.exchange.has_shared_file(PROJECT, &file_id, version);
                        if !mine && !already {
                            if let Some(project_id) =
                                topic.0.strip_prefix("project/").map(|s| s.to_string())
                            {
                                if me_sub.project_manager.project_root(&project_id).is_some() {
                                    let exchange = me_sub.exchange.clone();
                                    let req = FetchRequest {
                                        project_id,
                                        file_id: FileId(file_id.0.clone()),
                                        source_peer: Some(sender),
                                        priority: TransferPriority::Interactive,
                                        version,
                                    };
                                    tokio::spawn(async move {
                                        let _ = exchange.fetch_file(req).await;
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
        let me_fan = self.clone();
        let fan = tokio::spawn(async move {
            let mut rx = me_fan.gossip.outbound_receiver();
            while let Ok(out) = rx.recv().await {
                let peers: Vec<_> = if out.peers.is_empty() {
                    me_fan
                        .quic
                        .list_connections()
                        .into_iter()
                        .map(|c| c.peer_id)
                        .collect()
                } else {
                    out.peers
                };
                for peer in peers {
                    let wire = GossipWireMessage {
                        topic: out.topic.clone(),
                        signed: out.signed.clone(),
                    };
                    let quic = me_fan.quic.clone();
                    tokio::spawn(async move {
                        if let Ok((send, _recv)) =
                            quic.open_stream(&peer, StreamType::Control).await
                        {
                            let _ = send_gossip(send, &wire).await;
                        }
                    });
                }
            }
        });
        (sub, fan)
    }

    /// ipc_server の PublishUpdate と同じ手順: CRC → manifest bump → register → publish_updates
    async fn publish(&self, rel: &str) -> BumpOutcome {
        let abs = self.root.join(rel);
        let (crc, size) = crc32_of_file(&abs).await.unwrap();
        let rel_key = normalize_rel_path(std::path::Path::new(rel));
        let file_id = FileId::new(rel_key.clone());
        let outcome = self
            .project_manager
            .bump_file_version(
                PROJECT,
                &rel_key,
                size,
                crc,
                &self.identity.peer_id().0,
            )
            .await
            .unwrap();
        self.project_manager
            .register_file(PROJECT, file_id.clone(), std::path::PathBuf::from(rel));
        self.exchange
            .archive_to_history(synergos_core::exchange::ArchiveRequest {
                project_id: PROJECT.into(),
                file_id: file_id.clone(),
                version: outcome.version(),
                size,
                crc,
                publisher: self.identity.peer_id().0.clone(),
                source: "published",
                path: abs.clone(),
            })
            .await
            .unwrap();
        self.exchange
            .publish_updates(vec![PublishNotification {
                project_id: PROJECT.into(),
                file_id,
                file_path: abs,
                file_size: size,
                crc,
                version: outcome.version(),
            }])
            .await
            .unwrap();
        outcome
    }
}

async fn wait_for_content(path: &std::path::Path, expected: &[u8]) -> bool {
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Ok(data) = tokio::fs::read(path).await {
            if data == expected {
                return true;
            }
        }
    }
    false
}

async fn wait_for_history(node: &Node, rel: &str, version: u64) -> bool {
    for _ in 0..80 {
        if node
            .history
            .lookup(&node.root, PROJECT, rel, version)
            .await
            .ok()
            .flatten()
            .is_some()
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A (通常ノード) が publish、B (履歴ノード) が受信して全版を保管、
/// A が v1 に restore すると B の保管庫から旧版が届き、その後の publish は
/// 既存の版番号を飛び越える (v3)。
#[tokio::test]
async fn history_node_serves_old_version_on_restore() {
    let _ = tracing_subscriber::fmt::try_init();
    let a_dir = std::env::temp_dir().join(format!("syn-hist-a-{}", uuid::Uuid::new_v4()));
    let b_dir = std::env::temp_dir().join(format!("syn-hist-b-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(a_dir.join("assets"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(&b_dir).await.unwrap();

    let (node_a, _addr_a) = Node::bind(a_dir.clone(), false).await;
    let (node_b, addr_b) = Node::bind(b_dir.clone(), true).await;
    let node_a = Arc::new(node_a);
    let node_b = Arc::new(node_b);

    let _a_accept = node_a.spawn_accept();
    let (_a_sub, _a_fan) = node_a.spawn_gossip();
    let _b_accept = node_b.spawn_accept();
    let (_b_sub, _b_fan) = node_b.spawn_gossip();

    node_a
        .quic
        .connect(node_b.identity.peer_id().clone(), addr_b, "synergos")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A 側 client-side accept_bi (B が server-initiated で開く gossip/TXFR を拾う)
    {
        let a = node_a.clone();
        let b_peer = node_b.identity.peer_id().clone();
        tokio::spawn(async move {
            for _ in 0..20 {
                if a.quic.raw_connection(&b_peer).is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let Some(connection) = a.quic.raw_connection(&b_peer) else {
                return;
            };
            while let Ok((send, mut recv)) = connection.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                let gossip = a.gossip.clone();
                let exchange = a.exchange.clone();
                let sender = b_peer.clone();
                tokio::spawn(async move {
                    if &magic == GOSSIP_STREAM_MAGIC {
                        drop(send);
                        let _ = handle_gossip_stream(gossip, recv, sender).await;
                    } else if &magic == TRANSFER_STREAM_MAGIC {
                        let _ = exchange.handle_incoming_transfer(recv, sender).await;
                    }
                });
            }
        });
    }
    let topic = TopicId::project(PROJECT);
    node_a
        .gossip
        .graft(&topic, node_b.identity.peer_id().clone());
    node_b
        .gossip
        .graft(&topic, node_a.identity.peer_id().clone());

    let rel = if cfg!(windows) {
        "assets\\big.bin"
    } else {
        "assets/big.bin"
    };
    let rel_key = "assets/big.bin";
    let a_file = a_dir.join("assets").join("big.bin");
    let b_file = b_dir.join("assets").join("big.bin");

    // ── 1. v1 → B が受信し保管庫に入る ──
    let v1 = vec![0x11u8; 200 * 1024];
    tokio::fs::write(&a_file, &v1).await.unwrap();
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Bumped(1));
    assert!(wait_for_content(&b_file, &v1).await, "B must receive v1");
    assert!(
        wait_for_history(&node_b, rel_key, 1).await,
        "B must archive v1"
    );

    // ── 2. v2 → B が受信し保管庫に v1/v2 が揃う。作業ツリーは v2 ──
    let v2 = vec![0x22u8; 150 * 1024];
    tokio::fs::write(&a_file, &v2).await.unwrap();
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Bumped(2));
    assert!(wait_for_content(&b_file, &v2).await, "B must receive v2");
    assert!(
        wait_for_history(&node_b, rel_key, 2).await,
        "B must archive v2"
    );
    let listed = node_b.history.list(&b_dir, PROJECT, None).await.unwrap();
    assert_eq!(listed.len(), 2, "history node keeps every version");
    // 通常ノード A は何も保管しない
    assert!(node_a
        .history
        .list(&a_dir, PROJECT, None)
        .await
        .unwrap()
        .is_empty());
    assert!(!a_dir.join(".synergos").join("history").exists());

    // ── 3. A が v1 に restore → A は履歴を持たないので FileWant(v1) を出し、
    //       B が保管庫から v1 を送る。A は手元 v2 より古い v1 を pin 済みなので受け入れる ──
    let cctx = CheckoutContext {
        projects: &node_a.project_manager,
        exchange: &node_a.exchange,
        history: &node_a.history,
    };
    assert_eq!(
        restore_file(&cctx, PROJECT, rel_key, 1).await.unwrap(),
        RestoreOutcome::Requested
    );
    assert!(
        wait_for_content(&a_file, &v1).await,
        "A must get v1 back from the history node"
    );
    // A の manifest は v1 に書き戻る (pinned 受信 = force)
    let mut ma_v = 0;
    for _ in 0..30 {
        ma_v = ProjectManifest::load(&a_dir, PROJECT)
            .await
            .unwrap()
            .get(rel_key)
            .map(|e| e.version)
            .unwrap_or(0);
        if ma_v == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(ma_v, 1, "A manifest must record the restored version");
    let fid = FileId::new(rel_key);
    assert!(node_a.exchange.has_shared_file(PROJECT, &fid, 1));
    assert!(!node_a.exchange.has_shared_file(PROJECT, &fid, 2));

    // ── 4. 巻き戻し後の publish は観測済み最大版 (2) を飛び越えて v3 になる ──
    let v3 = vec![0x33u8; 100 * 1024];
    tokio::fs::write(&a_file, &v3).await.unwrap();
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Bumped(3));
    assert!(wait_for_content(&b_file, &v3).await, "B must receive v3");
    assert!(
        wait_for_history(&node_b, rel_key, 3).await,
        "B must archive v3"
    );

    // ── 5. B 自身の restore は保管庫からローカルで差し替わる (ネットワーク不要) ──
    let cctx_b = CheckoutContext {
        projects: &node_b.project_manager,
        exchange: &node_b.exchange,
        history: &node_b.history,
    };
    assert_eq!(
        restore_file(&cctx_b, PROJECT, rel_key, 2).await.unwrap(),
        RestoreOutcome::RestoredLocally
    );
    assert_eq!(tokio::fs::read(&b_file).await.unwrap(), v2);
    assert_eq!(
        ProjectManifest::load(&b_dir, PROJECT)
            .await
            .unwrap()
            .get(rel_key)
            .unwrap()
            .version,
        2
    );
    assert_eq!(
        restore_file(&cctx_b, PROJECT, rel_key, 2).await.unwrap(),
        RestoreOutcome::AlreadyAtVersion
    );

    let _ = tokio::fs::remove_dir_all(&a_dir).await;
    let _ = tokio::fs::remove_dir_all(&b_dir).await;
}
