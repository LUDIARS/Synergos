//! Full 2-node E2E: 2 つの疑似ノードを立て、gossip mesh + QUIC 転送まで
//! すべて Synergos 自身の配線で動かす。
//!
//! シナリオ:
//!   1. A (sender) は share_file でファイルを offer し、gossip で FileOffer を publish
//!   2. B (receiver) は gossip 経由で A の FileOffer を受信 → handle_file_offer で ledger に登録
//!   3. B は fetch_file を呼ぶ → gossip で FileWant を publish
//!   4. A は gossip 経由で FileWant を受信 → handle_file_want → share_and_send
//!   5. A → B に QUIC TXFR ストリームを開き、実データをプッシュ
//!   6. B の accept ループが handle_incoming_transfer で保存
//!   7. B の保存先を読んで元ペイロードと一致することを確認

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FetchRequest, FileSharing, OutPathResolver, ShareRequest, TransferPriority,
};
use synergos_core::project::ProjectManager;
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

/// 1 ノードの最小 Daemon 相当。GossipNode / QuicManager / Exchange +
/// 必要な背景タスクをまとめる。
struct Node {
    identity: Arc<Identity>,
    quic: Arc<QuicManager>,
    gossip: Arc<GossipNode>,
    exchange: Arc<Exchange>,
    event_bus: SharedEventBus,
    #[allow(dead_code)]
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
        let project_manager = Arc::new(ProjectManager::new(event_bus.clone()));
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
                event_bus,
                project_manager,
            },
            addr,
        )
    }

    /// QUIC accept ループ: GSP1 と TXFR を正しくディスパッチする。
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

    /// gossip subscriber + fanout を spawn する。
    fn spawn_gossip(
        self: &Arc<Self>,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let me_sub = self.clone();
        let sub = tokio::spawn(async move {
            let mut rx = me_sub.gossip.receiver();
            while let Ok((_topic, msg)) = rx.recv().await {
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
                    } => me_sub
                        .exchange
                        .handle_file_offer(sender, file_id, version, size, crc),
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
async fn two_nodes_auto_transfer_via_gossip_and_quic() {
    let _ = tracing_subscriber::fmt::try_init();

    let a_dir = std::env::temp_dir().join(format!("syn-2node-a-{}", uuid::Uuid::new_v4()));
    let b_dir = std::env::temp_dir().join(format!("syn-2node-b-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&a_dir).await.unwrap();
    tokio::fs::create_dir_all(&b_dir).await.unwrap();

    let (node_a, addr_a) = Node::bind(a_dir.clone()).await;
    let (node_b, addr_b) = Node::bind(b_dir.clone()).await;
    let node_a = Arc::new(node_a);
    let node_b = Arc::new(node_b);

    // QUIC accept + gossip subscriber/fanout を両ノードで立ち上げる
    let _a_accept = node_a.spawn_accept();
    let (_a_sub, _a_fan) = node_a.spawn_gossip();
    let _b_accept = node_b.spawn_accept();
    let (_b_sub, _b_fan) = node_b.spawn_gossip();

    // QUIC は bidirectional なので A→B 1 本だけ張る。accept 側でも
    // QuicManager.accept が connections に A を入れるので、B も
    // `open_stream(A)` が可能。これで gossip fanout / TXFR 両方とも
    // 同一コネクション上で server-initiated bidi を開ける。
    node_a
        .quic
        .connect(node_b.identity.peer_id().clone(), addr_b, "synergos")
        .await
        .unwrap();
    // B が accept() で受け取るのを待つ
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = addr_a; // 未使用回避

    // A は client として A→B を開いた側。`spawn_accept` は
    // `quic.accept()` から来る接続しか聞かないので、A→B 上の
    // server-initiated bidi (B が open_stream で張ってくる gossip/TXFR)
    // を拾うために client-side でも accept_bi ループを走らせる。
    {
        let a = node_a.clone();
        let b_peer = node_b.identity.peer_id().clone();
        tokio::spawn(async move {
            // 接続が connections に入るのを少し待つ
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

    let topic = TopicId::project("auto-e2e");
    node_a.gossip.subscribe(topic.clone());
    node_b.gossip.subscribe(topic.clone());
    // mesh に相手を手動 graft (実運用では GRAFT ハンドシェイクで入る)
    node_a
        .gossip
        .graft(&topic, node_b.identity.peer_id().clone());
    node_b
        .gossip
        .graft(&topic, node_a.identity.peer_id().clone());

    // A がファイルを登録 (FileOffer を publish)
    let src = a_dir.join("big.bin");
    let payload: Vec<u8> = (0..256u32)
        .flat_map(|n| (n as u8).to_le_bytes())
        .cycle()
        .take(300 * 1024)
        .collect();
    tokio::fs::write(&src, &payload).await.unwrap();
    let file_id = FileId::new("big.bin");
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

    // 少し待ってから B 側が fetch_file で FileWant を publish する
    tokio::time::sleep(Duration::from_millis(200)).await;
    node_b
        .exchange
        .fetch_file(FetchRequest {
            project_id: "auto-e2e".into(),
            file_id: file_id.clone(),
            source_peer: None,
            priority: TransferPriority::Interactive,
            version: 1,
        })
        .await
        .unwrap();

    // ファイルが B 側に届くのを待つ
    let expected_path = b_dir.join("big.bin");
    let mut got: Option<Vec<u8>> = None;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Ok(data) = tokio::fs::read(&expected_path).await {
            if data.len() == payload.len() {
                got = Some(data);
                break;
            }
        }
    }
    let got = got.expect("B must receive the file within 6s");
    assert_eq!(got, payload);

    let _ = tokio::fs::remove_dir_all(&a_dir).await;
    let _ = tokio::fs::remove_dir_all(&b_dir).await;
    // event_bus を使わない警告回避
    let _ = node_a.event_bus.clone();
    let _ = node_b.event_bus.clone();
}
