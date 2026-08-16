use serde::Serialize;

use crate::cloudflare::{DeviceRegistration, MeshConnector};
use crate::store::{Node, NodeKind, Org};

/// Cloudflare 側の実態とレジストリの突合結果。
///
/// dark node = Cloudflare の Mesh に存在するのに管制サーバーに登録されていない参加者。
/// クローズド運用ではこれをゼロに保つのが目標。
#[derive(Debug, Clone, Serialize)]
pub struct ReconcileReport {
    pub generated_at_ms: u64,
    /// レジストリと一致した Mesh node。
    pub known_connectors: Vec<ConnectorMatch>,
    /// Cloudflare に存在するがレジストリに無い Mesh node (= dark node)。
    pub dark_connectors: Vec<MeshConnector>,
    /// レジストリにあるが Cloudflare 側に見つからない Mesh node (手動削除等)。
    pub missing_connectors: Vec<NodeRef>,
    /// メンバーのメールに紐づくデバイス登録。
    pub known_devices: Vec<DeviceMatch>,
    /// どの組織のメンバーでもないデバイス登録 (= dark node)。
    pub dark_devices: Vec<DeviceRegistration>,
    /// heartbeat の自己申告 Mesh IP と管理者が登録した期待値の不一致。
    pub mesh_ip_mismatches: Vec<MeshIpMismatch>,
    /// dark に対して実施したアクション (revoke 実行時のみ)。
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectorMatch {
    pub connector: MeshConnector,
    pub node: NodeRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeshIpMismatch {
    pub node: NodeRef,
    /// heartbeat が自己申告した IP。
    pub reported_mesh_ip: Option<String>,
    /// 管理者がノードレコードへ登録した期待値。
    pub expected_mesh_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceMatch {
    pub registration: DeviceRegistration,
    /// 同じメールが複数組織に所属する場合があるため、全ての所属先を返す。
    pub org_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeRef {
    pub node_id: String,
    pub org_id: String,
    pub display_name: String,
    pub owner_email: String,
}

impl From<&Node> for NodeRef {
    fn from(node: &Node) -> Self {
        Self {
            node_id: node.id.clone(),
            org_id: node.org_id.clone(),
            display_name: node.display_name.clone(),
            owner_email: node.owner_email.clone(),
        }
    }
}

/// 突合の純粋ロジック。I/O を持たないので単体テスト可能。
pub fn classify(
    orgs: &[Org],
    nodes: &[Node],
    connectors: &[MeshConnector],
    registrations: &[DeviceRegistration],
    now_ms: u64,
) -> ReconcileReport {
    // --- Mesh node (connector) の突合: cf_connector_id で一致を取る ---
    let mut known_connectors = Vec::new();
    let mut dark_connectors = Vec::new();
    let mut mesh_ip_mismatches = Vec::new();
    for connector in connectors {
        match nodes
            .iter()
            .find(|n| n.cf_connector_id.as_deref() == Some(connector.id.as_str()))
        {
            Some(node) => {
                // peer_id↔Mesh IP 照合: heartbeat 自己申告と管理者設定値が両方あって
                // 食い違う場合のみ mismatch (片方欠けは情報不足であって異常ではない)
                if let (Some(reported), Some(expected)) =
                    (node.reported_mesh_ip.as_deref(), node.mesh_ip.as_deref())
                {
                    if reported != expected {
                        mesh_ip_mismatches.push(MeshIpMismatch {
                            node: NodeRef::from(node),
                            reported_mesh_ip: Some(reported.to_string()),
                            expected_mesh_ip: Some(expected.to_string()),
                        });
                    }
                }
                known_connectors.push(ConnectorMatch {
                    connector: connector.clone(),
                    node: NodeRef::from(node),
                });
            }
            None => dark_connectors.push(connector.clone()),
        }
    }

    let missing_connectors = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::MeshNode)
        .filter(|n| {
            n.cf_connector_id
                .as_deref()
                .is_some_and(|id| !connectors.iter().any(|c| c.id == id))
        })
        .map(NodeRef::from)
        .collect();

    // --- デバイス登録の突合: エンロールユーザのメールを組織メンバーと照合 ---
    let mut known_devices = Vec::new();
    let mut dark_devices = Vec::new();
    for registration in registrations {
        // 失効済みは dark 扱いから除外 (すでに対処済みのため)
        if registration.revoked_at.is_some() {
            continue;
        }
        let owner_org_ids: Vec<String> = registration
            .user_email
            .as_deref()
            .map(|email| {
                orgs.iter()
                    .filter(|org| {
                        org.members
                            .iter()
                            .any(|member| member.eq_ignore_ascii_case(email))
                    })
                    .map(|org| org.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if owner_org_ids.is_empty() {
            dark_devices.push(registration.clone());
        } else {
            known_devices.push(DeviceMatch {
                registration: registration.clone(),
                org_ids: owner_org_ids,
            });
        }
    }

    ReconcileReport {
        generated_at_ms: now_ms,
        known_connectors,
        dark_connectors,
        missing_connectors,
        known_devices,
        dark_devices,
        mesh_ip_mismatches,
        actions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::now_ms;

    fn org(id: &str, members: &[&str]) -> Org {
        Org {
            id: id.to_string(),
            name: id.to_string(),
            members: members.iter().map(|m| m.to_string()).collect(),
            created_at_ms: 0,
        }
    }

    fn mesh_node(id: &str, org_id: &str, connector_id: Option<&str>) -> Node {
        Node {
            id: id.to_string(),
            org_id: org_id.to_string(),
            display_name: id.to_string(),
            owner_email: "owner@example.com".to_string(),
            kind: NodeKind::MeshNode,
            cf_connector_id: connector_id.map(|s| s.to_string()),
            synergos_peer_id: None,
            mesh_ip: None,
            reported_mesh_ip: None,
            node_key_hash: None,
            last_heartbeat_ms: None,
            synergos_version: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn connector(id: &str) -> MeshConnector {
        MeshConnector {
            id: id.to_string(),
            name: format!("syn-{id}"),
            status: Some("healthy".to_string()),
            created_at: None,
            deleted_at: None,
        }
    }

    fn registration(id: &str, email: Option<&str>) -> DeviceRegistration {
        DeviceRegistration {
            id: id.to_string(),
            name: Some(format!("dev-{id}")),
            user_email: email.map(|e| e.to_string()),
            virtual_ipv4: None,
            last_seen_at: None,
            revoked_at: None,
        }
    }

    #[test]
    fn detects_dark_and_missing_connectors() {
        let orgs = vec![org("acme", &["alice@acme.test"])];
        let nodes = vec![
            mesh_node("n1", "acme", Some("cf-1")),
            mesh_node("n2", "acme", Some("cf-gone")),
        ];
        let connectors = vec![connector("cf-1"), connector("cf-dark")];

        let report = classify(&orgs, &nodes, &connectors, &[], now_ms());

        assert_eq!(report.known_connectors.len(), 1);
        assert_eq!(report.dark_connectors.len(), 1);
        assert_eq!(report.dark_connectors[0].id, "cf-dark");
        assert_eq!(report.missing_connectors.len(), 1);
        assert_eq!(report.missing_connectors[0].node_id, "n2");
    }

    #[test]
    fn classifies_devices_by_org_membership() {
        let orgs = vec![
            org("acme", &["alice@acme.test"]),
            org("globex", &["bob@globex.test"]),
        ];
        let registrations = vec![
            registration("d1", Some("Alice@ACME.test")), // 大文字小文字は無視して一致
            registration("d2", Some("mallory@evil.test")),
            registration("d3", None), // メール不明も dark
        ];

        let report = classify(&orgs, &[], &[], &registrations, now_ms());

        assert_eq!(report.known_devices.len(), 1);
        assert_eq!(report.known_devices[0].org_ids, vec!["acme".to_string()]);
        assert_eq!(report.dark_devices.len(), 2);
    }

    #[test]
    fn revoked_registrations_are_ignored() {
        let mut reg = registration("d1", Some("mallory@evil.test"));
        reg.revoked_at = Some("2026-07-30T00:00:00Z".to_string());

        let report = classify(&[], &[], &[], &[reg], now_ms());

        assert!(report.dark_devices.is_empty());
        assert!(report.known_devices.is_empty());
    }

    #[test]
    fn flags_mesh_ip_mismatch_against_expected_value() {
        let orgs = vec![org("acme", &["alice@acme.test"])];
        let mut node = mesh_node("n1", "acme", Some("cf-1"));
        node.reported_mesh_ip = Some("100.96.0.99".to_string());
        node.mesh_ip = Some("100.96.0.5".to_string());
        let connectors = vec![connector("cf-1")];

        let report = classify(&orgs, &[node], &connectors, &[], now_ms());

        assert_eq!(report.mesh_ip_mismatches.len(), 1);
        let mismatch = &report.mesh_ip_mismatches[0];
        assert_eq!(mismatch.reported_mesh_ip.as_deref(), Some("100.96.0.99"));
        assert_eq!(mismatch.expected_mesh_ip.as_deref(), Some("100.96.0.5"));
    }

    #[test]
    fn matching_heartbeat_ip_is_not_flagged() {
        let orgs = vec![org("acme", &["alice@acme.test"])];
        let mut node = mesh_node("n1", "acme", Some("cf-1"));
        node.reported_mesh_ip = Some("100.96.0.5".to_string());
        node.mesh_ip = Some("100.96.0.5".to_string());
        let connectors = vec![connector("cf-1")];

        let report = classify(&orgs, &[node], &connectors, &[], now_ms());

        assert!(report.mesh_ip_mismatches.is_empty());
    }
}
