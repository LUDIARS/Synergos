//! Gossip mesh 実配信 E2E: GossipNode::publish → outbound → QUIC `GSP1` →
//! 相手の handle_gossip_stream → on_signed_message_received → ローカル subscriber
//! という完全なチェーンをノード間で動かす。
//!
//! これが通れば「A が FileOffer publish → B が gossip で知る → B が FileWant
//! publish → A が gossip で知って QUIC TXFR で自動プッシュ」の auto-update
//! パイプラインが end-to-end で回る。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use synergos_net::config::{GossipsubConfig, QuicConfig};
use synergos_net::gossip::{
    handle_gossip_stream, send_gossip, GossipMessage, GossipNode, GossipWireMessage,
    GOSSIP_STREAM_MAGIC,
};
use synergos_net::identity::Identity;
use synergos_net::quic::{QuicManager, StreamType};
use synergos_net::types::{FileId, TopicId};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
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

fn make_node(identity: Arc<Identity>) -> Arc<GossipNode> {
    let mut g = GossipNode::new(identity.peer_id().clone(), gcfg());
    g.set_identity(identity);
    Arc::new(g)
}

#[tokio::test]
async fn publish_flows_through_quic_gossip_stream_to_remote_subscriber() {
    let id_a = Arc::new(Identity::generate());
    let id_b = Arc::new(Identity::generate());

    // QUIC エンドポイント
    let quic_a = Arc::new(QuicManager::new(qcfg(), id_a.clone()));
    let quic_b = Arc::new(QuicManager::new(qcfg(), id_b.clone()));
    let _bound_a = quic_a.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
    let bound_b = quic_b.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    // GossipNode
    let g_a = make_node(id_a.clone());
    let g_b = make_node(id_b.clone());

    // B 側の受信タスク: accept して GSP1 magic を読み、handle_gossip_stream に流す
    let qb = quic_b.clone();
    let gb_clone = g_b.clone();
    let id_a_peer = id_a.peer_id().clone();
    let b_accept = tokio::spawn(async move {
        if let Ok(Some(acc)) = qb.accept().await {
            while let Ok((_send, mut recv)) = acc.connection.accept_bi().await {
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == GOSSIP_STREAM_MAGIC {
                    let _ = handle_gossip_stream(gb_clone.clone(), recv, id_a_peer.clone()).await;
                    break; // 1 メッセージ受けたらテスト用に抜ける
                }
            }
        }
    });

    // B 側のローカル受信 subscriber を事前に確保
    let mut b_rx = g_b.receiver();

    // A→B に QUIC 接続
    quic_a
        .connect(id_b.peer_id().clone(), bound_b, "synergos")
        .await
        .unwrap();

    // A で publish する (outbound_rx に流れる)
    let mut a_outbound = g_a.outbound_receiver();
    let topic = TopicId::project("demo");
    g_a.subscribe(topic.clone());
    g_a.publish(
        &topic,
        GossipMessage::FileOffer {
            sender: id_a.peer_id().clone(),
            file_id: FileId::new("hello.txt"),
            version: 1,
            size: 42,
            crc: 0xdeadbeef,
            content_hash: Default::default(),
        },
    );

    // outbound を拾って A 側から B へ手動で配送 (Daemon の fanout タスク相当)
    let outbound = tokio::time::timeout(Duration::from_secs(2), a_outbound.recv())
        .await
        .expect("outbound must arrive quickly")
        .expect("outbound channel open");
    let wire = GossipWireMessage {
        topic: outbound.topic.clone(),
        signed: outbound.signed.clone(),
    };
    let (send, _recv) = quic_a
        .open_stream(id_b.peer_id(), StreamType::Control)
        .await
        .unwrap();
    send_gossip(send, &wire).await.unwrap();

    // B 側の subscriber が受信できるはず
    let (recv_topic, recv_msg) = tokio::time::timeout(Duration::from_secs(3), b_rx.recv())
        .await
        .expect("B must receive broadcast")
        .expect("channel open");
    assert_eq!(recv_topic, topic);
    match recv_msg {
        GossipMessage::FileOffer {
            sender, file_id, ..
        } => {
            assert_eq!(sender, *id_a.peer_id());
            assert_eq!(file_id.0, "hello.txt");
        }
        other => panic!("unexpected gossip variant: {other:?}"),
    }

    tokio::time::timeout(Duration::from_millis(200), b_accept)
        .await
        .ok();
}
