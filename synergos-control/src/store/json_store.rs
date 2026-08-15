use std::path::PathBuf;

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::error::{ControlError, ControlResult};

use super::{Node, Org, RegistrySnapshot};

/// JSON ファイル 1 枚に永続化する単純なレジストリストア。
///
/// 想定規模 (クローズド運用の組織×ノード数十件) では DB は過剰なため、
/// atomic write (temp + rename) の JSON ファイルで十分とする。
pub struct JsonStore {
    path: PathBuf,
    state: Mutex<RegistrySnapshot>,
}

/// `org_id` 指定時はその組織のノードかを判定する (None は組織を問わない)。
fn in_org(node: &Node, org_id: Option<&str>) -> bool {
    match org_id {
        Some(org) => node.org_id == org,
        None => true,
    }
}

impl JsonStore {
    /// ファイルが存在すれば読み込み、無ければ空レジストリで開始する。
    pub fn open(path: PathBuf) -> ControlResult<Self> {
        let state = if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| ControlError::Store(format!("read {}: {e}", path.display())))?;
            serde_json::from_str(&raw)
                .map_err(|e| ControlError::Store(format!("parse {}: {e}", path.display())))?
        } else {
            RegistrySnapshot::default()
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    async fn persist(&self, snapshot: &RegistrySnapshot) -> ControlResult<()> {
        let json = serde_json::to_string_pretty(snapshot)
            .map_err(|e| ControlError::Store(format!("serialize: {e}")))?;
        let tmp = self.path.with_extension(format!(
            "{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let result = async {
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .await
                .map_err(|e| ControlError::Store(format!("create {}: {e}", tmp.display())))?;
            #[cfg(unix)]
            tokio::fs::set_permissions(
                &tmp,
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
            )
            .await
            .map_err(|e| ControlError::Store(format!("secure {}: {e}", tmp.display())))?;
            file.write_all(json.as_bytes())
                .await
                .map_err(|e| ControlError::Store(format!("write {}: {e}", tmp.display())))?;
            file.sync_all()
                .await
                .map_err(|e| ControlError::Store(format!("sync {}: {e}", tmp.display())))?;
            drop(file);
            tokio::fs::rename(&tmp, &self.path).await.map_err(|e| {
                ControlError::Store(format!("rename to {}: {e}", self.path.display()))
            })?;
            Ok(())
        }
        .await;
        if result.is_err() {
            // best-effort: 元のエラーを保持しつつ未完成の一時ファイルを片付ける。
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    pub async fn snapshot(&self) -> RegistrySnapshot {
        self.state.lock().await.clone()
    }

    // --- Org ---

    pub async fn insert_org(&self, org: Org) -> ControlResult<Org> {
        let mut state = self.state.lock().await;
        if state.orgs.iter().any(|o| o.id == org.id) {
            return Err(ControlError::Conflict(format!(
                "org {} already exists",
                org.id
            )));
        }
        let mut next = state.clone();
        next.orgs.push(org.clone());
        self.persist(&next).await?;
        *state = next;
        Ok(org)
    }

    pub async fn update_org(&self, org: Org) -> ControlResult<Org> {
        let mut state = self.state.lock().await;
        if let Some(node) = state.nodes.iter().find(|node| {
            node.org_id == org.id
                && !org
                    .members
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(&node.owner_email))
        }) {
            return Err(ControlError::Conflict(format!(
                "member {} still owns node {}; transfer or remove the node first",
                node.owner_email, node.id
            )));
        }
        let mut next = state.clone();
        let slot = next
            .orgs
            .iter_mut()
            .find(|o| o.id == org.id)
            .ok_or_else(|| ControlError::NotFound(format!("org {}", org.id)))?;
        *slot = org.clone();
        self.persist(&next).await?;
        *state = next;
        Ok(org)
    }

    pub async fn get_org(&self, org_id: &str) -> ControlResult<Org> {
        self.state
            .lock()
            .await
            .orgs
            .iter()
            .find(|o| o.id == org_id)
            .cloned()
            .ok_or_else(|| ControlError::NotFound(format!("org {org_id}")))
    }

    // --- Node ---

    pub async fn insert_node(&self, node: Node) -> ControlResult<Node> {
        let mut state = self.state.lock().await;
        let org = state
            .orgs
            .iter()
            .find(|o| o.id == node.org_id)
            .ok_or_else(|| ControlError::NotFound(format!("org {}", node.org_id)))?;
        if !org
            .members
            .iter()
            .any(|member| member.eq_ignore_ascii_case(&node.owner_email))
        {
            return Err(ControlError::Conflict(format!(
                "owner {} is no longer a member of org {}",
                node.owner_email, node.org_id
            )));
        }
        if state.nodes.iter().any(|existing| existing.id == node.id) {
            return Err(ControlError::Conflict(format!(
                "node {} already exists",
                node.id
            )));
        }
        let mut next = state.clone();
        next.nodes.push(node.clone());
        self.persist(&next).await?;
        *state = next;
        Ok(node)
    }

    /// ノードをロック保持のまま read-modify-write する。
    ///
    /// `get_node` → 手元で書き換え → `update_node` の 2 段構えだと、heartbeat と
    /// reconcile が同時に走ったときに後勝ちでレコード全体を上書きし、相手が書いた
    /// フィールド (mesh_ip / reported_mesh_ip など) を巻き戻してしまう。
    /// 更新はこの関数に集約し、ロック内で必要なフィールドだけ差分適用する。
    ///
    /// `org_id` を渡すとその組織のノードに限定する (組織跨ぎの更新を防ぐ)。
    /// `mutate` の中からストアの他メソッドを呼ぶとデッドロックするので、
    /// 検証など他レコードを要する処理は呼び出し側で先に済ませること。
    pub async fn mutate_node<F>(
        &self,
        org_id: Option<&str>,
        node_id: &str,
        mutate: F,
    ) -> ControlResult<Node>
    where
        F: FnOnce(&mut Node),
    {
        self.try_mutate_node(org_id, node_id, |node| {
            mutate(node);
            Ok(())
        })
        .await
    }

    /// 検証と更新を同じロック内で行う fallible な read-modify-write。
    /// 永続化に失敗した場合はメモリ上の状態も変更しない。
    pub async fn try_mutate_node<F>(
        &self,
        org_id: Option<&str>,
        node_id: &str,
        mutate: F,
    ) -> ControlResult<Node>
    where
        F: FnOnce(&mut Node) -> ControlResult<()>,
    {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        let slot = next
            .nodes
            .iter_mut()
            .find(|n| n.id == node_id && in_org(n, org_id))
            .ok_or_else(|| ControlError::NotFound(format!("node {node_id}")))?;
        mutate(slot)?;
        let updated = slot.clone();
        let org = next
            .orgs
            .iter()
            .find(|org| org.id == updated.org_id)
            .ok_or_else(|| ControlError::NotFound(format!("org {}", updated.org_id)))?;
        if !org
            .members
            .iter()
            .any(|member| member.eq_ignore_ascii_case(&updated.owner_email))
        {
            return Err(ControlError::Conflict(format!(
                "owner {} is not a member of org {}",
                updated.owner_email, updated.org_id
            )));
        }
        self.persist(&next).await?;
        *state = next;
        Ok(updated)
    }

    pub async fn list_nodes(&self, org_id: &str) -> Vec<Node> {
        self.state
            .lock()
            .await
            .nodes
            .iter()
            .filter(|n| n.org_id == org_id)
            .cloned()
            .collect()
    }

    pub async fn get_node(&self, org_id: &str, node_id: &str) -> ControlResult<Node> {
        self.state
            .lock()
            .await
            .nodes
            .iter()
            .find(|n| n.org_id == org_id && n.id == node_id)
            .cloned()
            .ok_or_else(|| ControlError::NotFound(format!("node {node_id}")))
    }

    pub async fn remove_node(&self, org_id: &str, node_id: &str) -> ControlResult<Node> {
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        let idx = next
            .nodes
            .iter()
            .position(|n| n.org_id == org_id && n.id == node_id)
            .ok_or_else(|| ControlError::NotFound(format!("node {node_id}")))?;
        let removed = next.nodes.remove(idx);
        self.persist(&next).await?;
        *state = next;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{now_ms, NodeKind};

    fn org(id: &str) -> Org {
        Org {
            id: id.to_string(),
            name: id.to_string(),
            members: vec!["owner@example.com".to_string()],
            created_at_ms: now_ms(),
        }
    }

    fn node(id: &str, org_id: &str) -> Node {
        Node {
            id: id.to_string(),
            org_id: org_id.to_string(),
            display_name: id.to_string(),
            owner_email: "owner@example.com".to_string(),
            kind: NodeKind::MeshNode,
            cf_connector_id: None,
            synergos_peer_id: None,
            mesh_ip: None,
            reported_mesh_ip: None,
            node_key_hash: None,
            last_heartbeat_ms: None,
            synergos_version: None,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        }
    }

    #[tokio::test]
    async fn round_trips_via_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");

        let store = JsonStore::open(path.clone()).unwrap();
        store.insert_org(org("acme")).await.unwrap();
        store.insert_node(node("n1", "acme")).await.unwrap();

        let reopened = JsonStore::open(path).unwrap();
        let snap = reopened.snapshot().await;
        assert_eq!(snap.orgs.len(), 1);
        assert_eq!(snap.nodes.len(), 1);
    }

    #[tokio::test]
    async fn rejects_duplicate_org_and_unknown_org_node() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("s.json")).unwrap();
        store.insert_org(org("a")).await.unwrap();
        assert!(store.insert_org(org("a")).await.is_err());
        assert!(store.insert_node(node("n1", "missing")).await.is_err());
    }

    #[tokio::test]
    async fn refuses_to_remove_member_who_still_owns_a_node() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("s.json")).unwrap();
        store.insert_org(org("a")).await.unwrap();
        store.insert_node(node("n1", "a")).await.unwrap();

        let mut updated = org("a");
        updated.members.clear();
        assert!(matches!(
            store.update_org(updated).await,
            Err(ControlError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn failed_persist_does_not_change_memory_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("missing").join("s.json")).unwrap();

        assert!(store.insert_org(org("a")).await.is_err());
        assert!(store.snapshot().await.orgs.is_empty());
    }

    #[tokio::test]
    async fn remove_node_scoped_by_org() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::open(dir.path().join("s.json")).unwrap();
        store.insert_org(org("a")).await.unwrap();
        store.insert_org(org("b")).await.unwrap();
        store.insert_node(node("n1", "a")).await.unwrap();
        assert!(store.remove_node("b", "n1").await.is_err());
        assert!(store.remove_node("a", "n1").await.is_ok());
    }
}
