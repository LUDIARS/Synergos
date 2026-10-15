//! QUIC 接続の確立・切断を Presence の相手一覧へ反映する。
//!
//! 従来は `peer add-url` / 招待 join をした側だけが相手を登録していたため、
//! 接続を受けた側の `peer list` に相手が出なかった (B4)。daemon は着信・発信の
//! どちらの接続でもここを呼び、両側で相手を Connected として扱う。
//! 切断時は Disconnected に落とし、再接続先 URL が分かっているピアは
//! daemon の再接続ループが張り直す (B5)。

use std::time::Instant;

use synergos_net::types::PeerId;

use super::{
    NodeRegistration, NodeRegistry, NodeRegistryError, PeerState, PresenceService, RegisteredNode,
};
use crate::event_bus::{PeerConnectedEvent, PeerDisconnectedEvent};

impl PresenceService {
    /// QUIC 接続が確立したピアを Connected として反映する。
    /// 未登録なら `project_ids` で新規登録して `PeerConnectedEvent` を出し true を返す。
    /// 登録済みなら状態を Connected に戻し、`project_ids` を追加マージする
    /// (表示名・版などの登録済み情報は保つ)。
    pub fn observe_connected(&self, peer_id: &PeerId, project_ids: Vec<String>) -> bool {
        if let Some(mut entry) = self.nodes.get_mut(peer_id) {
            let node = entry.value_mut();
            let reconnected = node.state != PeerState::Connected;
            node.state = PeerState::Connected;
            node.last_seen = Instant::now();
            for project_id in project_ids {
                if !node.project_ids.contains(&project_id) {
                    node.project_ids.push(project_id);
                }
            }
            let project_id = node.project_ids.first().cloned().unwrap_or_default();
            let display_name = node.display_name.clone();
            drop(entry);
            if reconnected {
                self.emit_connected(peer_id, project_id, display_name);
            }
            return false;
        }

        let node = RegisteredNode {
            peer_id: peer_id.clone(),
            display_name: peer_id.to_string(),
            endpoints: vec![],
            state: PeerState::Connected,
            rtt_ms: None,
            project_ids: project_ids.clone(),
            bandwidth_bps: 0,
            last_seen: Instant::now(),
            synergos_version: String::new(),
        };
        self.nodes.insert(peer_id.clone(), node);
        tracing::info!("peer {} connected; added to presence", peer_id.short());
        self.emit_connected(
            peer_id,
            project_ids.first().cloned().unwrap_or_default(),
            peer_id.to_string(),
        );
        true
    }

    fn emit_connected(&self, peer_id: &PeerId, project_id: String, display_name: String) {
        self.event_bus.emit(PeerConnectedEvent {
            project_id,
            peer_id: peer_id.to_string(),
            display_name,
            route: "quic".to_string(),
            rtt_ms: 0,
        });
    }

    /// QUIC 接続が閉じたピアを Disconnected に落とす。一覧からは消さない
    /// (再接続で Connected に戻る)。既に Disconnected / 未登録なら何もしない。
    pub fn observe_disconnected(&self, peer_id: &PeerId, reason: &str) {
        let project_id = {
            let Some(mut entry) = self.nodes.get_mut(peer_id) else {
                return;
            };
            let node = entry.value_mut();
            if node.state == PeerState::Disconnected {
                return;
            }
            node.state = PeerState::Disconnected;
            node.project_ids.first().cloned().unwrap_or_default()
        };
        tracing::info!("peer {} disconnected: {reason}", peer_id.short());
        self.event_bus.emit(PeerDisconnectedEvent {
            project_id,
            peer_id: peer_id.to_string(),
            reason: reason.to_string(),
        });
    }

    /// `peer add-url` / 招待 join / bootstrap で得た登録情報を反映する。
    /// 発信接続の確立通知 (`observe_connected`) が先に相手を載せていることがある
    /// ので、登録済みでもエラーにせず表示名・版・プロジェクトを上書きマージする。
    pub async fn register_or_merge(
        &self,
        registration: NodeRegistration,
    ) -> Result<(), NodeRegistryError> {
        if self.merge_registered_connection(&registration) {
            return Ok(());
        }
        let peer_id = registration.peer_id.clone();
        match self.register_node(registration.clone()).await {
            Ok(_) => {
                let _ = self.update_node_state(&peer_id, PeerState::Connected).await;
                return Ok(());
            }
            // 判定と登録の間に observe_connected が割り込んだ場合は、そこで
            // 作られた仮エントリへ peer-info の表示名・版・project を改めてマージする。
            Err(NodeRegistryError::AlreadyRegistered(_))
                if self.merge_registered_connection(&registration) => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn merge_registered_connection(&self, registration: &NodeRegistration) -> bool {
        let Some(mut entry) = self.nodes.get_mut(&registration.peer_id) else {
            return false;
        };
        let node = entry.value_mut();
        let reconnected = node.state != PeerState::Connected;
        node.state = PeerState::Connected;
        node.last_seen = Instant::now();
        if !registration.display_name.is_empty() {
            node.display_name.clone_from(&registration.display_name);
        }
        if !registration.synergos_version.is_empty() {
            node.synergos_version
                .clone_from(&registration.synergos_version);
        }
        if !registration.endpoints.is_empty() {
            node.endpoints.clone_from(&registration.endpoints);
        }
        for project_id in &registration.project_ids {
            if !node.project_ids.contains(project_id) {
                node.project_ids.push(project_id.clone());
            }
        }
        let project_id = node.project_ids.first().cloned().unwrap_or_default();
        let display_name = node.display_name.clone();
        drop(entry);
        if reconnected {
            self.emit_connected(&registration.peer_id, project_id, display_name);
        }
        true
    }

    /// 切断時に張り直す `/peer-info` URL を覚える。
    pub fn remember_reconnect_url(&self, peer_id: &PeerId, url: &str) {
        self.reconnect_urls.insert(peer_id.clone(), url.to_string());
    }

    /// 再接続対象 (peer_id, URL) の一覧。
    pub fn reconnect_targets(&self) -> Vec<(PeerId, String)> {
        self.reconnect_urls
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::event_bus::CoreEventBus;

    fn presence() -> PresenceService {
        PresenceService::new(Arc::new(CoreEventBus::new()))
    }

    #[tokio::test]
    async fn inbound_peer_becomes_visible_in_project_list() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        assert!(p.observe_connected(&peer, vec!["proj".into()]));
        let nodes = p.list_nodes(Some("proj")).await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].state, PeerState::Connected);
    }

    #[tokio::test]
    async fn reobserve_merges_projects_without_duplicate() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.observe_connected(&peer, vec!["a".into()]);
        assert!(!p.observe_connected(&peer, vec!["a".into(), "b".into()]));
        let node = p.get_node(&peer).await.unwrap();
        assert_eq!(node.project_ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn disconnect_keeps_entry_and_reconnect_restores_state() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.observe_connected(&peer, vec!["proj".into()]);
        p.observe_disconnected(&peer, "closed");
        assert_eq!(
            p.get_node(&peer).await.unwrap().state,
            PeerState::Disconnected
        );
        p.observe_connected(&peer, vec![]);
        assert_eq!(p.get_node(&peer).await.unwrap().state, PeerState::Connected);
    }

    #[tokio::test]
    async fn reconnect_emits_connected_event_again() {
        let p = presence();
        let mut events = p.event_bus.subscribe::<PeerConnectedEvent>();
        let peer = PeerId::new("peer-a");
        p.observe_connected(&peer, vec!["proj".into()]);
        let _ = events.try_recv().unwrap();

        p.observe_disconnected(&peer, "closed");
        p.observe_connected(&peer, vec![]);

        let event = events.try_recv().unwrap();
        assert_eq!(event.peer_id, peer.to_string());
        assert_eq!(event.project_id, "proj");
    }

    #[tokio::test]
    async fn observe_keeps_registered_metadata() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.register_node(NodeRegistration {
            peer_id: peer.clone(),
            display_name: "host-a".into(),
            project_ids: vec!["proj".into()],
            synergos_version: "1.2.3".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        p.observe_connected(&peer, vec!["proj".into()]);
        let node = p.get_node(&peer).await.unwrap();
        assert_eq!(node.display_name, "host-a");
        assert_eq!(node.synergos_version, "1.2.3");
        assert_eq!(node.state, PeerState::Connected);
    }

    #[tokio::test]
    async fn register_after_observed_connection_merges_instead_of_failing() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.observe_connected(&peer, vec!["other".into()]);
        p.register_or_merge(NodeRegistration {
            peer_id: peer.clone(),
            display_name: "host-a".into(),
            project_ids: vec!["proj".into()],
            synergos_version: "1.2.3".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        let node = p.get_node(&peer).await.unwrap();
        assert_eq!(node.display_name, "host-a");
        assert_eq!(node.synergos_version, "1.2.3");
        assert_eq!(
            node.project_ids,
            vec!["other".to_string(), "proj".to_string()]
        );
        assert_eq!(node.state, PeerState::Connected);
    }

    #[tokio::test]
    async fn register_or_merge_registers_new_peer_as_connected() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.register_or_merge(NodeRegistration {
            peer_id: peer.clone(),
            display_name: "host-a".into(),
            project_ids: vec!["proj".into()],
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(p.get_node(&peer).await.unwrap().state, PeerState::Connected);
    }

    #[tokio::test]
    async fn unregister_forgets_reconnect_url() {
        let p = presence();
        let peer = PeerId::new("peer-a");
        p.observe_connected(&peer, vec![]);
        p.remember_reconnect_url(&peer, "http://h:7780");
        assert_eq!(p.reconnect_targets().len(), 1);
        p.unregister_node(&peer).await.unwrap();
        assert!(p.reconnect_targets().is_empty());
    }

    #[tokio::test]
    async fn register_self_does_not_list_local_node_as_peer() {
        let p = presence();
        p.register_self(NodeRegistration {
            peer_id: PeerId::new("me"),
            display_name: "me".into(),
            project_ids: vec!["proj".into()],
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(p.list_nodes(None).await.is_empty());
    }
}
