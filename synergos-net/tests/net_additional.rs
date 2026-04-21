//! PR-3: synergos-net 既存ロジックの L1 補強。
//!
//! - ConnectionCalibrator::calibrate の境界値
//! - RoutingTable の空バケット探索 / closest
//! - TransferLedger::mark_fulfilled / cancel_want / 同時 Want/Offer の順序独立性
//! - Mesh のキャッシュクリア

use std::time::{Duration, Instant};

use synergos_net::catalog::CatalogManager;
use synergos_net::chain::{LedgerAction, LedgerEntryState, OfferEntry, TransferLedger, WantEntry};
use synergos_net::config::MeshConfig;
use synergos_net::dht::rpc::RouteDto;
use synergos_net::mesh::Mesh;
use synergos_net::quic::{ConnectionCalibrator, SpeedTestResult};
use synergos_net::types::{FileId, PeerId};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── ConnectionCalibrator ─────────────────────────────────────────────

#[test]
fn calibrator_zero_bandwidth_minimum() {
    let result = SpeedTestResult {
        rtt_median_ms: 200,
        rtt_jitter_ms: 50,
        download_bps: 0,
        upload_bps: 0,
        effective_streams: 1,
    };
    let params = ConnectionCalibrator::calibrate(&result);
    assert_eq!(params.max_connections, 1);
    assert_eq!(params.large_streams, 1);
    assert_eq!(params.medium_streams, 1);
    assert_eq!(params.small_streams, 1);
    assert!((params.chunk_size_multiplier - 2.0).abs() < f64::EPSILON);
    assert_eq!(params.initial_bandwidth_limit, 0);
}

#[test]
fn calibrator_high_bandwidth_scales_up() {
    let result = SpeedTestResult {
        rtt_median_ms: 10,
        rtt_jitter_ms: 1,
        download_bps: 500_000_000,
        upload_bps: 500_000_000,
        effective_streams: 16,
    };
    let params = ConnectionCalibrator::calibrate(&result);
    assert_eq!(params.max_connections, 4);
    assert_eq!(params.large_streams, 8);
    assert_eq!(params.medium_streams, 4);
    assert_eq!(params.small_streams, 2);
    assert!((params.chunk_size_multiplier - 1.0).abs() < f64::EPSILON);
}

#[test]
fn calibrator_asymmetric_uses_min() {
    let result = SpeedTestResult {
        rtt_median_ms: 75,
        rtt_jitter_ms: 5,
        download_bps: 500_000_000,
        upload_bps: 5_000_000,
        effective_streams: 8,
    };
    let params = ConnectionCalibrator::calibrate(&result);
    assert_eq!(params.max_connections, 1);
    assert!((params.chunk_size_multiplier - 1.5).abs() < f64::EPSILON);
}

// ── TransferLedger ───────────────────────────────────────────────────

fn want_for(requester: &str, file: &str, version: u64) -> WantEntry {
    WantEntry {
        requester: PeerId::new(requester),
        file_id: FileId::new(file),
        version,
        requested_at: now_ms(),
        state: LedgerEntryState::Pending,
    }
}

fn offer_for(sender: &str, file: &str, version: u64, size: u64) -> OfferEntry {
    OfferEntry {
        sender: PeerId::new(sender),
        file_id: FileId::new(file),
        version,
        file_size: size,
        crc: 0,
        offered_at: now_ms(),
        state: LedgerEntryState::Pending,
    }
}

#[test]
fn ledger_mark_fulfilled_transitions_state() {
    let ledger = TransferLedger::new();
    ledger.register_offer(offer_for("sender", "f1", 1, 100));
    let _ = ledger.register_want(want_for("me", "f1", 1));
    ledger.mark_fulfilled(&FileId::new("f1"), 1, &PeerId::new("me"));
    // fulfilled 状態: pending_want_count は 0 になる
    assert_eq!(ledger.pending_want_count(&FileId::new("f1"), 1), 0);
}

#[test]
fn ledger_cancel_want_removes_from_pending() {
    let ledger = TransferLedger::new();
    let _ = ledger.register_want(want_for("me", "f2", 1));
    assert_eq!(ledger.pending_want_count(&FileId::new("f2"), 1), 1);
    ledger.cancel_want(&FileId::new("f2"), 1, &PeerId::new("me"));
    assert_eq!(ledger.pending_want_count(&FileId::new("f2"), 1), 0);
}

#[test]
fn ledger_concurrent_want_offer_match_regardless_of_order() {
    // Want → Offer
    let l1 = TransferLedger::new();
    let _ = l1.register_want(want_for("me", "f", 1));
    let a1 = l1.register_offer(offer_for("s", "f", 1, 42));
    assert!(a1.iter().any(|a| matches!(a, LedgerAction::Match { .. })));

    // Offer → Want
    let l2 = TransferLedger::new();
    let _ = l2.register_offer(offer_for("s", "f", 1, 42));
    let a2 = l2.register_want(want_for("me", "f", 1));
    match a2 {
        LedgerAction::Match { .. } => (),
        other => panic!("expected Match, got {other:?}"),
    }
}

// ── CatalogManager ───────────────────────────────────────────────────

#[tokio::test]
async fn catalog_record_update_on_missing_file_errors() {
    let cm = CatalogManager::new("proj".to_string(), 10, 32);
    // 直接 record_update するのは file_id 未登録 → エラー
    use synergos_net::chain::{ChainBlock, ChainPayload};
    use synergos_net::types::Cid;
    let bogus_file = FileId::new("never-added");
    let block = ChainBlock {
        hash: Default::default(),
        prev_hash: None,
        version: 1,
        author: PeerId::new("a"),
        author_public_key: [0u8; 32],
        timestamp: now_ms(),
        payload: ChainPayload::BinaryCid {
            cid: Cid("c".into()),
            file_size: 0,
            crc: 0,
        },
        signature: [0u8; 64],
    };
    let result = cm.record_update(&bogus_file, block).await;
    assert!(result.is_err(), "record_update must error on missing file");
}

#[tokio::test]
async fn catalog_add_file_is_isolated_per_project() {
    let a = CatalogManager::new("pA".into(), 10, 32);
    let b = CatalogManager::new("pB".into(), 10, 32);
    let _fa = a.add_file("foo.txt", 1, 10).await;
    let _fb = b.add_file("foo.txt", 2, 20).await;
    // root_catalog crc はプロジェクトごとに独立
    assert_ne!(
        a.root_catalog().await.catalog_crc,
        b.root_catalog().await.catalog_crc
    );
}

// ── Mesh (DNS cache) ─────────────────────────────────────────────────

#[tokio::test]
async fn mesh_clear_dns_cache_is_noop_when_empty() {
    let mesh = Mesh::new(MeshConfig {
        doh_endpoint: "https://cloudflare-dns.com/dns-query".into(),
        dns_servers: vec![],
        turn_servers: vec![],
        stun_servers: vec![],
        probe_timeout_ms: 1_000,
    });
    mesh.clear_dns_cache();
    assert_eq!(mesh.list_turn_sessions().len(), 0);
}

#[tokio::test]
async fn mesh_cleanup_expired_sessions_tolerates_no_sessions() {
    let mesh = Mesh::new(MeshConfig {
        doh_endpoint: "https://cloudflare-dns.com/dns-query".into(),
        dns_servers: vec![],
        turn_servers: vec![],
        stun_servers: vec![],
        probe_timeout_ms: 1_000,
    });
    mesh.cleanup_expired_sessions();
    assert!(mesh.list_turn_sessions().is_empty());
}

// ── RouteDto (dht::rpc) Direct roundtrip ─────────────────────────────

#[test]
fn route_dto_roundtrip_preserves_direct() {
    use std::net::{Ipv6Addr, SocketAddrV6};
    use synergos_net::types::Route;
    let original = Route::Direct {
        addr: SocketAddrV6::new(Ipv6Addr::LOCALHOST, 9000, 0, 0),
        fqdn: Some("example.com".into()),
    };
    let dto: RouteDto = (&original).into();
    let back: Route = (&dto).into();
    match back {
        Route::Direct { addr, fqdn } => {
            assert_eq!(addr.port(), 9000);
            assert_eq!(fqdn, Some("example.com".into()));
        }
        _ => panic!("wrong variant"),
    }
}

// ── RoutingTable (empty bucket exploration) ──────────────────────────

// RoutingTable の内部モジュールは pub ではないが、DhtNode 経由で
// closest_peers を呼ぶことで空状態の振る舞いを確認する。
#[tokio::test]
async fn dht_closest_on_empty_table_returns_empty() {
    use synergos_net::config::DhtConfig;
    use synergos_net::dht::DhtNode;
    use synergos_net::types::NodeId;

    let node = DhtNode::new(
        PeerId::new("self"),
        DhtConfig {
            k_bucket_size: 8,
            routing_refresh_secs: 300,
            peer_ttl_secs: 300,
        },
    );
    let target = NodeId::from_peer_id(&PeerId::new("anybody"));
    let peers = node.closest_peers(&target, 5).await;
    assert!(peers.is_empty());
}

// last_seen を用いた upsert の時間的振る舞い (Instant の昇順確認)
#[test]
fn instant_ordering_sanity() {
    let a = Instant::now();
    std::thread::sleep(Duration::from_millis(2));
    let b = Instant::now();
    assert!(b > a);
}
