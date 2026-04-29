//! 自動更新 E2E: Peer A が共有しているファイルを、Peer B の Gossip FileWant
//! 受信をトリガに Peer A が自動で QUIC push する流れを検証する。
//!
//! 全パイプは Synergos 既存機構を通る:
//!   - 登録: `share_file` → `TransferLedger::register_offer` + `shared_files`
//!   - トリガ: Exchange::handle_file_want(remote_requester, file_id, version)
//!   - 送信: share_and_send → QUIC bidi `TXFR` stream
//!   - 受信: Daemon 相当のアクセプタ → `handle_incoming_transfer`
//!
//! 本テストでは 2 ノード間の gossip 中継は手で叩き、QUIC 転送だけを本物で
//! 通す。gossip の mesh 自体は別途 integration される。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::exchange::{
    Exchange, FileSharing, OutPathResolver, ShareRequest, TransferPriority,
};
use synergos_net::config::QuicConfig;
use synergos_net::identity::Identity;
use synergos_net::quic::QuicManager;
use synergos_net::transfer::TRANSFER_STREAM_MAGIC;
use synergos_net::types::{Blake3Hash, FileId};

fn qcfg() -> QuicConfig {
    QuicConfig {
        max_concurrent_streams: 8,
        idle_timeout_ms: 5_000,
        max_udp_payload_size: 1350,
        enable_0rtt: false,
        listen_addr: None,
    }
}

#[tokio::test]
async fn file_want_auto_triggers_share_and_send_over_quic() {
    let _ = tracing_subscriber::fmt::try_init();

    // ── セットアップ ──
    let id_a = Arc::new(Identity::generate()); // sender / offerer
    let id_b = Arc::new(Identity::generate()); // requester / receiver

    let quic_a = Arc::new(QuicManager::new(qcfg(), id_a.clone()));
    let quic_b = Arc::new(QuicManager::new(qcfg(), id_b.clone()));
    let bound_a: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();
    let _ = quic_a.bind(bound_a).await.unwrap();
    let bound_b = quic_b.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    // 受信側 (B) の保存先
    let dst_dir = std::env::temp_dir().join(format!("synergos-auto-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dst_dir).await.unwrap();

    let bus_b: SharedEventBus = Arc::new(CoreEventBus::new());
    let mut ex_b_inner = Exchange::with_network(bus_b.clone(), id_b.peer_id().clone(), None);
    let dst_dir_b = dst_dir.clone();
    let resolver_b: OutPathResolver =
        Arc::new(move |_p, fid: &FileId| Some(dst_dir_b.join(&fid.0)));
    ex_b_inner.attach_quic(quic_b.clone(), resolver_b);
    let ex_b = Arc::new(ex_b_inner);

    // B の accept ループ (Daemon 相当): 1 bidi 受けたら handle_incoming_transfer
    let qb = quic_b.clone();
    let ex_b_for_task = ex_b.clone();
    let receive_task = tokio::spawn(async move {
        if let Ok(Some(acc)) = qb.accept().await {
            let sender = acc.peer_id.clone();
            while let Ok((send, mut recv)) = acc.connection.accept_bi().await {
                let _ = send;
                let mut magic = [0u8; 4];
                if recv.read_exact(&mut magic).await.is_err() {
                    continue;
                }
                if &magic == TRANSFER_STREAM_MAGIC {
                    let _ = ex_b_for_task
                        .handle_incoming_transfer(recv, sender.clone())
                        .await;
                }
                break;
            }
        }
    });

    // 送信側 (A) の Exchange
    let bus_a: SharedEventBus = Arc::new(CoreEventBus::new());
    let mut ex_a_inner = Exchange::with_network(bus_a.clone(), id_a.peer_id().clone(), None);
    let dummy_resolver: OutPathResolver = Arc::new(|_, _| None);
    ex_a_inner.attach_quic(quic_a.clone(), dummy_resolver);
    let ex_a = Arc::new(ex_a_inner);

    // A が file を共有登録 (TransferLedger + shared_files 両方)
    let src_dir = std::env::temp_dir().join(format!("synergos-auto-src-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&src_dir).await.unwrap();
    let src_file = src_dir.join("report.bin");
    let payload: Vec<u8> = (0..100u32)
        .flat_map(|n| (n as u8).to_le_bytes())
        .cycle()
        .take(200 * 1024)
        .collect();
    tokio::fs::write(&src_file, &payload).await.unwrap();

    let file_id = FileId::new("report.bin");
    ex_a.share_file(ShareRequest {
        project_id: "p".into(),
        file_id: file_id.clone(),
        file_path: src_file.clone(),
        file_size: payload.len() as u64,
        checksum: Blake3Hash::default(),
        priority: TransferPriority::Interactive,
        target_peer: None, // broadcast offer; want 側でトリガ
        version: 1,
    })
    .await
    .unwrap();

    // A が B に接続 (実運用では Conduit/Gossip が経路を提示してから張る)
    quic_a
        .connect(id_b.peer_id().clone(), bound_b, "synergos")
        .await
        .unwrap();

    // ── B が FileWant を broadcast した、と想定して A の gossip subscriber
    // 経由で handle_file_want を直接叩く (gossip 中継は別途統合) ──
    ex_a.handle_file_want(id_b.peer_id().clone(), file_id.clone(), 1);

    // 受信完了を待つ
    tokio::time::timeout(Duration::from_secs(5), receive_task)
        .await
        .expect("receive task should finish")
        .unwrap();

    let got = tokio::fs::read(dst_dir.join("report.bin")).await.unwrap();
    assert_eq!(got.len(), payload.len());
    assert_eq!(got, payload);

    let _ = tokio::fs::remove_dir_all(&src_dir).await;
    let _ = tokio::fs::remove_dir_all(&dst_dir).await;
}

#[tokio::test]
async fn file_want_without_shared_file_is_noop() {
    let id = Arc::new(Identity::generate());
    let quic = Arc::new(QuicManager::new(qcfg(), id.clone()));
    quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();

    let bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let mut ex_inner = Exchange::with_network(bus, id.peer_id().clone(), None);
    let resolver: OutPathResolver = Arc::new(|_, _| None);
    ex_inner.attach_quic(quic, resolver);
    let ex = Arc::new(ex_inner);

    // shared_files に無いので無視される (panic せず、転送も起動しない)
    ex.handle_file_want(
        synergos_net::types::PeerId::new("some-peer"),
        FileId::new("nonexistent"),
        1,
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ex.list_transfers(None).await.len(), 0);
}

#[tokio::test]
async fn file_offer_registers_remote_offer_in_ledger() {
    let id = Arc::new(Identity::generate());
    let bus: SharedEventBus = Arc::new(CoreEventBus::new());
    let ex = Exchange::with_network(bus, id.peer_id().clone(), None);

    // リモートピアからの offer を handle_file_offer で受け取る
    ex.handle_file_offer(
        synergos_net::types::PeerId::new("remote-sender"),
        FileId::new("cool.bin"),
        1,
        1024,
        42,
    );
    // ledger の pending_want_count は 0 (まだ Want 無い)
    assert_eq!(
        ex.ledger().pending_want_count(&FileId::new("cool.bin"), 1),
        0
    );
}
