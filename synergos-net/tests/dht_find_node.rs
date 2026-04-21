//! Kademlia iterative FIND_NODE のテスト。モックトランスポートで
//! ホップ数上限・タイムアウト・exact hit を確認する。

use std::collections::HashMap;
use std::net::SocketAddrV6;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use synergos_net::config::DhtConfig;
use synergos_net::dht::rpc::{DhtRequest, DhtResponse, DhtTransport, PeerRecordDto, RouteDto};
use synergos_net::dht::{DhtNode, FindNodeOptions, PeerRecord};
use synergos_net::error::{Result as NetResult, SynergosNetError};
use synergos_net::types::{PeerId, Route};

fn addr_for(n: u16) -> SocketAddrV6 {
    use std::net::Ipv6Addr;
    SocketAddrV6::new(Ipv6Addr::LOCALHOST, n, 0, 0)
}

fn dto(peer_id: &PeerId) -> PeerRecordDto {
    PeerRecordDto {
        peer_id: peer_id.clone(),
        display_name: peer_id.0.clone(),
        endpoints: vec![RouteDto::Direct {
            addr: addr_for(9000),
            fqdn: None,
        }],
        active_projects: vec!["p1".to_string()],
        ttl_secs: 60,
    }
}

fn cfg() -> DhtConfig {
    DhtConfig {
        k_bucket_size: 8,
        routing_refresh_secs: 300,
        peer_ttl_secs: 300,
    }
}

/// モックトランスポート。各ピアが「target をどれくらい知っているか」を
/// 事前設定して振る舞わせる。
struct MockTransport {
    /// responder peer_id → (exact target, closest neighbors)
    table: HashMap<PeerId, (Option<PeerRecordDto>, Vec<PeerRecordDto>)>,
    /// 応答遅延 (ms)
    latency_ms: Arc<Mutex<HashMap<PeerId, u64>>>,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            table: HashMap::new(),
            latency_ms: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn set_response(
        &mut self,
        responder: PeerId,
        exact: Option<PeerRecordDto>,
        closest: Vec<PeerRecordDto>,
    ) {
        self.table.insert(responder, (exact, closest));
    }

    fn set_latency(&self, responder: PeerId, ms: u64) {
        self.latency_ms.lock().unwrap().insert(responder, ms);
    }
}

#[async_trait]
impl DhtTransport for MockTransport {
    async fn request(&self, peer: &PeerId, req: &DhtRequest) -> NetResult<DhtResponse> {
        let latency = self
            .latency_ms
            .lock()
            .unwrap()
            .get(peer)
            .cloned()
            .unwrap_or(0);
        if latency > 0 {
            tokio::time::sleep(Duration::from_millis(latency)).await;
        }
        match req {
            DhtRequest::FindNode { .. } => {
                let (exact, closest) = self.table.get(peer).cloned().ok_or_else(|| {
                    SynergosNetError::PeerNotFound(format!("mock has no entry for {peer}"))
                })?;
                Ok(DhtResponse::Nodes { exact, closest })
            }
            _ => Ok(DhtResponse::Error {
                message: "unsupported".into(),
            }),
        }
    }
}

fn make_record(pid: &PeerId) -> PeerRecord {
    PeerRecord {
        peer_id: pid.clone(),
        display_name: pid.0.clone(),
        endpoints: vec![Route::Direct {
            addr: addr_for(9000),
            fqdn: None,
        }],
        active_projects: vec!["p1".into()],
        published_at: Instant::now(),
        ttl: Duration::from_secs(60),
    }
}

#[tokio::test]
async fn find_peer_local_hit_short_circuits() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let target = PeerId::new("target");
    node.announce(make_record(&target)).await;

    let transport = MockTransport::new();
    let opts = FindNodeOptions::default();
    let found = node
        .find_peer_iterative(&target, &transport, opts)
        .await
        .unwrap();
    assert_eq!(found.peer_id, target);
}

#[tokio::test]
async fn find_peer_iterative_returns_exact_hit_from_neighbor() {
    let node = DhtNode::new(PeerId::new("self"), cfg());

    let neighbor = PeerId::new("neighbor");
    let target = PeerId::new("target");

    // 自分の routing table には neighbor を入れておく
    node.add_peer(
        neighbor.clone(),
        vec![Route::Direct {
            addr: addr_for(9000),
            fqdn: None,
        }],
        None,
    )
    .await;

    // neighbor は target を exact hit で持っている
    let mut transport = MockTransport::new();
    transport.set_response(neighbor.clone(), Some(dto(&target)), vec![]);

    let opts = FindNodeOptions {
        max_hops: 2,
        alpha: 3,
        ..Default::default()
    };
    let found = node
        .find_peer_iterative(&target, &transport, opts)
        .await
        .expect("target should be resolved via neighbor");
    assert_eq!(found.peer_id, target);
}

#[tokio::test]
async fn find_peer_iterative_follows_closest_chain() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let n1 = PeerId::new("n1");
    let n2 = PeerId::new("n2");
    let target = PeerId::new("target");

    node.add_peer(
        n1.clone(),
        vec![Route::Direct {
            addr: addr_for(9001),
            fqdn: None,
        }],
        None,
    )
    .await;

    // n1 は target 自体は知らず、closer な n2 だけを返す
    let mut transport = MockTransport::new();
    transport.set_response(n1.clone(), None, vec![dto(&n2)]);
    transport.set_response(n2.clone(), Some(dto(&target)), vec![]);

    let found = node
        .find_peer_iterative(&target, &transport, FindNodeOptions::default())
        .await
        .expect("target should be resolved after 2 hops");
    assert_eq!(found.peer_id, target);
}

#[tokio::test]
async fn find_peer_iterative_respects_hop_limit() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let n1 = PeerId::new("n1");
    let n2 = PeerId::new("n2");
    let n3 = PeerId::new("n3");
    let target = PeerId::new("target");

    node.add_peer(
        n1.clone(),
        vec![Route::Direct {
            addr: addr_for(9001),
            fqdn: None,
        }],
        None,
    )
    .await;

    let mut transport = MockTransport::new();
    transport.set_response(n1.clone(), None, vec![dto(&n2)]);
    transport.set_response(n2.clone(), None, vec![dto(&n3)]);
    transport.set_response(n3.clone(), Some(dto(&target)), vec![]);

    // max_hops = 2 なので n3 への問合せは実行されず target に到達しない
    let opts = FindNodeOptions {
        max_hops: 2,
        alpha: 1,
        ..Default::default()
    };
    let found = node.find_peer_iterative(&target, &transport, opts).await;
    assert!(
        found.is_none(),
        "hop limit must prevent resolving target at depth 3"
    );
}

#[tokio::test]
async fn find_peer_iterative_timeout_on_slow_peer() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let slow = PeerId::new("slow");
    let target = PeerId::new("target");

    node.add_peer(
        slow.clone(),
        vec![Route::Direct {
            addr: addr_for(9001),
            fqdn: None,
        }],
        None,
    )
    .await;

    let mut transport = MockTransport::new();
    transport.set_response(slow.clone(), Some(dto(&target)), vec![]);
    transport.set_latency(slow.clone(), 500);

    let opts = FindNodeOptions {
        per_request_timeout: Duration::from_millis(50),
        max_hops: 1,
        alpha: 1,
        ..Default::default()
    };
    let start = Instant::now();
    let found = node.find_peer_iterative(&target, &transport, opts).await;
    assert!(found.is_none(), "slow peer should be timed-out");
    // 500ms 待たずに早期脱出していること
    assert!(start.elapsed() < Duration::from_millis(300));
}

#[tokio::test]
async fn handle_request_find_node_returns_local_exact() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let target = PeerId::new("target");
    node.announce(make_record(&target)).await;

    let resp = node
        .handle_request(DhtRequest::FindNode {
            target: target.clone(),
        })
        .await;
    match resp {
        DhtResponse::Nodes { exact, closest: _ } => {
            let exact = exact.expect("expected exact hit");
            assert_eq!(exact.peer_id, target);
        }
        other => panic!("unexpected response: {other:?}"),
    }
}

#[tokio::test]
async fn handle_request_ping_pong() {
    let node = DhtNode::new(PeerId::new("self"), cfg());
    let resp = node.handle_request(DhtRequest::Ping).await;
    matches!(resp, DhtResponse::Pong);
}
