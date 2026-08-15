//! バージョン付き再 publish の E2E:
//!
//!   1. A が `publish_updates` (v1) → B が auto-pull で受信
//!   2. A が同じファイルを書き換えて再 publish → manifest が v2 を発番 → B が **再受信** する
//!      (旧実装は version が 1 固定で 2 回目以降が届かなかった)
//!   3. 内容が同じ再 publish は version 据え置き → B は取りに行かない
//!   4. 両ノードの `.synergos/manifest.json` に version が残る
//!   5. 受信の一時ファイルはプロジェクト内 `.synergos/incoming/` を使い、残骸を残さない
//!   6. A の Exchange を作り直しても (= daemon 再起動相当) manifest から Offer 台帳が復元される
//!
//! ネットワークは auto_pull_e2e と同じ実 QUIC + gossip 配線。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FetchRequest, FileSharing, IncomingDirResolver, OutPathResolver, PublishNotification,
    ReceivedHook, TransferPriority,
};
use synergos_core::manifest::{crc32_of_file, normalize_rel_path, BumpOutcome, ProjectManifest};
use synergos_core::project::{ProjectConfiguration, ProjectManager};
use synergos_core::restore::restore_shared_files_from_manifests;
use synergos_net::config::{GossipsubConfig, QuicConfig};
use synergos_net::gossip::{
    handle_gossip_stream, send_gossip, GossipNode, GossipWireMessage, GOSSIP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};
use synergos_net::transfer::TRANSFER_STREAM_MAGIC;
use synergos_net::types::{FileId, TopicId};

const PROJECT: &str = "ver-e2e";

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
    root: std::path::PathBuf,
}

/// daemon.rs と同じ配線 (resolver / incoming dir / received hook) で Exchange を作る。
fn build_exchange(
    event_bus: SharedEventBus,
    identity: &Arc<Identity>,
    gossip: &Arc<GossipNode>,
    quic: &Arc<QuicManager>,
    pm: &Arc<ProjectManager>,
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
    let received: ReceivedHook =
        Arc::new(move |project_id, file_id, version, size, crc, sender| {
            let pm = pm3.clone();
            Box::pin(async move {
                let _ = pm
                    .record_received_file(&project_id, &file_id.0, version, size, crc, &sender.0)
                    .await;
            })
        });
    ex.attach_receive_hooks(incoming, received);
    Arc::new(ex)
}

impl Node {
    async fn bind(root: std::path::PathBuf) -> (Self, std::net::SocketAddr) {
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
        let exchange = build_exchange(event_bus, &identity, &gossip, &quic, &project_manager);
        (
            Self {
                identity,
                quic,
                gossip,
                exchange,
                project_manager,
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
        let outcome = self
            .project_manager
            .bump_file_version(PROJECT, &rel_key, size, crc, &self.identity.peer_id().0)
            .await
            .unwrap();
        let file_id = FileId::new(rel_key.clone());
        self.project_manager
            .register_file(PROJECT, file_id.clone(), std::path::PathBuf::from(rel));
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
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Ok(data) = tokio::fs::read(path).await {
            if data == expected {
                return true;
            }
        }
    }
    false
}

#[tokio::test]
async fn republish_bumps_version_and_reaches_peer() {
    let _ = tracing_subscriber::fmt::try_init();
    let a_dir = std::env::temp_dir().join(format!("syn-ver-a-{}", uuid::Uuid::new_v4()));
    let b_dir = std::env::temp_dir().join(format!("syn-ver-b-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(a_dir.join("assets"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(&b_dir).await.unwrap();

    let (node_a, _addr_a) = Node::bind(a_dir.clone()).await;
    let (node_b, addr_b) = Node::bind(b_dir.clone()).await;
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

    // ── 1. v1 publish (サブディレクトリ、Windows 区切りでも FileId は `/`) ──
    let rel = if cfg!(windows) {
        "assets\\big.bin"
    } else {
        "assets/big.bin"
    };
    let v1 = vec![0xA5u8; 300 * 1024]; // 複数フレーム
    tokio::fs::write(a_dir.join("assets").join("big.bin"), &v1)
        .await
        .unwrap();
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Bumped(1));
    let b_file = b_dir.join("assets").join("big.bin");
    assert!(wait_for_content(&b_file, &v1).await, "B must receive v1");
    let fid = FileId::new("assets/big.bin");
    assert!(node_b.exchange.has_shared_file(PROJECT, &fid, 1));

    // ── 2. 内容を変えて再 publish → v2 が B に届く ──
    let v2 = vec![0x5Au8; 200 * 1024];
    tokio::fs::write(a_dir.join("assets").join("big.bin"), &v2)
        .await
        .unwrap();
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Bumped(2));
    assert!(
        wait_for_content(&b_file, &v2).await,
        "B must receive v2 (re-publish)"
    );
    assert!(node_b.exchange.has_shared_file(PROJECT, &fid, 2));

    // ── 3. 同内容の再 publish は据え置き ──
    assert_eq!(node_a.publish(rel).await, BumpOutcome::Unchanged(2));

    // ── 4. manifest が両側に残る ──
    let ma = ProjectManifest::load(&a_dir, PROJECT).await.unwrap();
    let mb = ProjectManifest::load(&b_dir, PROJECT).await.unwrap();
    assert_eq!(ma.get("assets/big.bin").unwrap().version, 2);
    // B 側の記録は非同期フックなので少し待つ
    let mut mb_v = mb.get("assets/big.bin").map(|e| e.version).unwrap_or(0);
    for _ in 0..20 {
        if mb_v == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        mb_v = ProjectManifest::load(&b_dir, PROJECT)
            .await
            .unwrap()
            .get("assets/big.bin")
            .map(|e| e.version)
            .unwrap_or(0);
    }
    assert_eq!(mb_v, 2, "B manifest must record received version 2");
    assert_eq!(
        ProjectManifest::load(&b_dir, PROJECT)
            .await
            .unwrap()
            .get("assets/big.bin")
            .unwrap()
            .crc,
        crc32fast::hash(&v2)
    );

    // ── 5. 一時ファイルはプロジェクト内 incoming に置かれ、残骸が無い ──
    let incoming = b_dir.join(".synergos").join("incoming");
    assert!(
        incoming.is_dir(),
        "incoming dir must be inside the project root"
    );
    let mut rd = tokio::fs::read_dir(&incoming).await.unwrap();
    assert!(
        rd.next_entry().await.unwrap().is_none(),
        "no .part leftovers"
    );

    // ── 6. A の再起動相当: 新しい Exchange に manifest から Offer 台帳を復元 ──
    let pm_fresh = Arc::new(ProjectManager::new(Arc::new(CoreEventBus::new())));
    pm_fresh
        .open_project(PROJECT.into(), a_dir.clone(), None)
        .await
        .unwrap();
    let ex_fresh = Arc::new(Exchange::new(Arc::new(CoreEventBus::new())));
    let restored = restore_shared_files_from_manifests(&pm_fresh, &ex_fresh).await;
    assert_eq!(restored, 1);
    assert!(ex_fresh.has_shared_file(PROJECT, &fid, 2));
    assert!(!ex_fresh.has_shared_file(PROJECT, &fid, 3));

    let _ = tokio::fs::remove_dir_all(&a_dir).await;
    let _ = tokio::fs::remove_dir_all(&b_dir).await;
}
