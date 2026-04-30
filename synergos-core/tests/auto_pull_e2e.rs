//! Auto-pull E2E: A が share_file で publish したファイルを、B が
//! 明示的な `fetch_file` 呼び出し無し で受信できることを確認する。
//!
//! 経路:
//!   1. A: share_file → broadcast_offer (FileOffer gossip)
//!   2. B: gossip subscriber (auto-pull 有り) が FileOffer を受信
//!      → project が open + 未保有なら fetch_file を自動呼び出し
//!   3. B: fetch_file が FileWant を gossip publish
//!   4. A: gossip subscriber が FileWant を受信 → handle_file_want
//!      → share_and_send で QUIC TXFR 起動
//!   5. A → B: TXFR ストリームで実データ送信
//!   6. B: handle_incoming_transfer がファイルを保存 + shared_files に登録
//!
//! 違い: 既存の two_node_full_e2e は手で `fetch_file` を呼んでいるが、
//! ここでは subscriber 内で daemon と同じ auto-pull ロジックを inline で動かし、
//! 上位 IPC 無しでも転送が成立することを確認する。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FetchRequest, FileSharing, OutPathResolver, ShareRequest, TransferPriority,
};
use synergos_core::project::{ProjectConfiguration, ProjectManager};
use synergos_net::config::{GossipsubConfig, QuicConfig};
use synergos_net::gossip::{
    handle_gossip_stream, send_gossip, GossipNode, GossipWireMessage, GOSSIP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};
use synergos_net::transfer::TRANSFER_STREAM_MAGIC;
use synergos_net::types::{Blake3Hash, FileId, TopicId};

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
}

impl Node {
    async fn bind(save_dir: std::path::PathBuf) -> (Self, std::net::SocketAddr) {
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
        let mut ex_inner = Exchange::with_network(
            event_bus.clone(),
            identity.peer_id().clone(),
            Some(gossip.clone()),
        );
        let dir = save_dir.clone();
        let resolver: OutPathResolver = Arc::new(move |_p, fid: &FileId| Some(dir.join(&fid.0)));
        ex_inner.attach_quic(quic.clone(), resolver);
        let exchange = Arc::new(ex_inner);

        (
            Self {
                identity,
                quic,
                gossip,
                exchange,
                project_manager,
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
                    Ok(None) => break,
                    Err(_) => break,
                };
                let sender = acc.peer_id.clone();
                let connection = acc.connection;
                let gossip = me.gossip.clone();
                let exchange = me.exchange.clone();
                tokio::spawn(async move {
                    while let Ok((send, mut recv)) = connection.accept_bi().await {
                        let mut magic = [0u8; 4];
                        if recv.read_exact(&mut magic).await.is_err() {
                            continue;
                        }
                        let gossip = gossip.clone();
                        let exchange = exchange.clone();
                        let sender_clone = sender.clone();
                        tokio::spawn(async move {
                            if &magic == GOSSIP_STREAM_MAGIC {
                                drop(send);
                                let _ = handle_gossip_stream(gossip, recv, sender_clone).await;
                            } else if &magic == TRANSFER_STREAM_MAGIC {
                                let _ = exchange.handle_incoming_transfer(recv, sender_clone).await;
                            }
                        });
                    }
                });
            }
        })
    }

    /// auto-pull つき gossip subscriber + fanout。
    /// FileOffer 受信時に project が open かつ未保有なら fetch_file を発火する。
    fn spawn_gossip_with_auto_pull(
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
                        .handle_file_want(requester, file_id, version),
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
                        // auto-pull (daemon.rs と同じロジック)
                        let mine = sender == *me_sub.exchange.local_peer_id();
                        let already = me_sub.exchange.has_shared_file(&file_id, version);
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
}

#[tokio::test]
async fn b_auto_pulls_file_after_receiving_offer() {
    let _ = tracing_subscriber::fmt::try_init();

    let a_dir = std::env::temp_dir().join(format!("syn-autopull-a-{}", uuid::Uuid::new_v4()));
    let b_dir = std::env::temp_dir().join(format!("syn-autopull-b-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&a_dir).await.unwrap();
    tokio::fs::create_dir_all(&b_dir).await.unwrap();

    let (node_a, _addr_a) = Node::bind(a_dir.clone()).await;
    let (node_b, addr_b) = Node::bind(b_dir.clone()).await;
    let node_a = Arc::new(node_a);
    let node_b = Arc::new(node_b);

    // 両ノードで project を open する。auto-pull は project が open でないと発火しない。
    node_a
        .project_manager
        .open_project("auto-e2e".into(), a_dir.clone(), Some("a".into()))
        .await
        .unwrap();
    node_b
        .project_manager
        .open_project("auto-e2e".into(), b_dir.clone(), Some("b".into()))
        .await
        .unwrap();

    let _a_accept = node_a.spawn_accept();
    let (_a_sub, _a_fan) = node_a.spawn_gossip_with_auto_pull();
    let _b_accept = node_b.spawn_accept();
    let (_b_sub, _b_fan) = node_b.spawn_gossip_with_auto_pull();

    node_a
        .quic
        .connect(node_b.identity.peer_id().clone(), addr_b, "synergos")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A 側: client-side accept_bi ループ (B が server-initiated で開く gossip/TXFR を拾う)
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
            let gossip = a.gossip.clone();
            let exchange = a.exchange.clone();
            while let Ok((send, mut recv)) = connection.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                let gossip = gossip.clone();
                let exchange = exchange.clone();
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

    // gossip mesh セットアップ (両ノードで topic 購読 + 相手を mesh に grafted)
    let topic = TopicId::project("auto-e2e");
    node_a
        .gossip
        .graft(&topic, node_b.identity.peer_id().clone());
    node_b
        .gossip
        .graft(&topic, node_a.identity.peer_id().clone());

    // A がファイルを共有 (FileOffer を gossip に publish)
    let src = a_dir.join("greeting.txt");
    let payload = b"hello synergos auto-pull".to_vec();
    tokio::fs::write(&src, &payload).await.unwrap();
    let file_id = FileId::new("greeting.txt");
    node_a
        .exchange
        .share_file(ShareRequest {
            project_id: "auto-e2e".into(),
            file_id: file_id.clone(),
            file_path: src.clone(),
            file_size: payload.len() as u64,
            checksum: Blake3Hash::default(),
            priority: TransferPriority::Interactive,
            target_peer: None,
            version: 1,
        })
        .await
        .unwrap();

    // B 側で fetch_file を **明示的に呼び出さない**。auto-pull が機能していれば、
    // B が FileOffer を受信した時点で fetch_file が自動で発火し、A が TXFR で送ってくる。
    let expected_path = b_dir.join("greeting.txt");
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        if let Ok(data) = tokio::fs::read(&expected_path).await {
            if data.len() == payload.len() {
                got = Some(data);
                break;
            }
        }
    }
    let got = got.expect("B must auto-pull and receive the file within 6s");
    assert_eq!(got, payload);

    // 受信完了後は B 自身の shared_files にも登録されるはず
    // (handle_incoming_transfer の責務)。
    assert!(
        node_b.exchange.has_shared_file(&file_id, 1),
        "B should register received file in shared_files for re-distribution"
    );

    let _ = tokio::fs::remove_dir_all(&a_dir).await;
    let _ = tokio::fs::remove_dir_all(&b_dir).await;
}
