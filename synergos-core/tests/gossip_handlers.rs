//! Gossip サブスクライバが PeerStatus / ConflictAlert をローカル状態に
//! 正しく反映するか (CatalogUpdate は現状 log のみのため対象外)。

use std::sync::Arc;

use synergos_core::conflict::ConflictManager;
use synergos_core::event_bus::{CoreEventBus, SharedEventBus};
use synergos_core::presence::{NodeRegistration, NodeRegistry, PeerState, PresenceService};
use synergos_net::gossip::{ActivityState, PeerActivityStatus};
use synergos_net::types::{FileId, PeerId};

fn bus() -> SharedEventBus {
    Arc::new(CoreEventBus::new())
}

#[tokio::test]
async fn handle_peer_status_updates_registered_node() {
    let p = PresenceService::new(bus());
    let peer = PeerId::new("peer-z");

    // 先に register_node しておく
    p.register_node(NodeRegistration {
        peer_id: peer.clone(),
        display_name: "z".into(),
        endpoints: vec![],
        project_ids: vec![],
        synergos_version: String::new(),
    })
    .await
    .unwrap();

    // Idle 状態を gossip で受信
    let status = PeerActivityStatus {
        peer_id: peer.clone(),
        display_name: "z".into(),
        state: ActivityState::Idle,
        last_active: 0,
        working_on: vec![],
    };
    p.handle_peer_status(&status, &PeerId::new("origin"));

    let node = p.get_node(&peer).await.unwrap();
    assert_eq!(node.state, PeerState::Idle);
}

#[tokio::test]
async fn handle_peer_status_offline_sets_disconnected() {
    let p = PresenceService::new(bus());
    let peer = PeerId::new("peer-w");
    p.register_node(NodeRegistration {
        peer_id: peer.clone(),
        display_name: "w".into(),
        endpoints: vec![],
        project_ids: vec![],
        synergos_version: String::new(),
    })
    .await
    .unwrap();

    let status = PeerActivityStatus {
        peer_id: peer.clone(),
        display_name: "w".into(),
        state: ActivityState::Offline,
        last_active: 0,
        working_on: vec![],
    };
    p.handle_peer_status(&status, &PeerId::new("origin"));

    let node = p.get_node(&peer).await.unwrap();
    assert_eq!(node.state, PeerState::Disconnected);
}

#[tokio::test]
async fn handle_peer_status_ignores_unknown_peer() {
    // register 未済の peer_id は no-op (panic しない)
    let p = PresenceService::new(bus());
    let status = PeerActivityStatus {
        peer_id: PeerId::new("unknown"),
        display_name: "".into(),
        state: ActivityState::Active,
        last_active: 0,
        working_on: vec![],
    };
    p.handle_peer_status(&status, &PeerId::new("o"));
    assert!(p.get_node(&PeerId::new("unknown")).await.is_none());
}

#[tokio::test]
async fn handle_conflict_alert_registers_active_conflict() {
    let cm = ConflictManager::new(bus());
    let file = FileId::new("doc.md");
    cm.handle_conflict_alert(
        file.clone(),
        vec![PeerId::new("a"), PeerId::new("b")],
        vec![3, 5],
    );
    let info = cm.get_conflict(&file).expect("registered");
    assert_eq!(info.remote_version, 5, "max of their_versions");
    assert_eq!(info.involved_peers.len(), 2);
}

#[tokio::test]
async fn handle_conflict_alert_merges_with_existing() {
    let cm = ConflictManager::new(bus());
    let file = FileId::new("shared.bin");
    cm.handle_conflict_alert(file.clone(), vec![PeerId::new("a")], vec![1]);
    cm.handle_conflict_alert(
        file.clone(),
        vec![PeerId::new("b"), PeerId::new("a")],
        vec![2],
    );
    let info = cm.get_conflict(&file).unwrap();
    assert_eq!(info.involved_peers.len(), 2, "deduplicated a, added b");
}
