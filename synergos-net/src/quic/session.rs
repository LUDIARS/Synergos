use std::collections::HashSet;
use std::sync::RwLock;

use crate::error::{Result, SynergosNetError};
use crate::types::PeerId;

pub type ProjectId = String;

const MAX_PROJECT_ID_LEN: usize = 1024;

/// 認証済みconnectionに紐づくproject membership認可状態。
#[derive(Debug)]
pub struct ConnectionSession {
    peer_id: PeerId,
    authorized_projects: RwLock<HashSet<ProjectId>>,
}

impl ConnectionSession {
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            authorized_projects: RwLock::new(HashSet::new()),
        }
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn authorize_project(&self, project_id: ProjectId) {
        if let Ok(mut projects) = self.authorized_projects.write() {
            projects.insert(project_id);
        }
    }

    pub fn authorize_projects(&self, projects: impl IntoIterator<Item = ProjectId>) {
        if let Ok(mut authorized) = self.authorized_projects.write() {
            authorized.extend(projects);
        }
    }

    pub fn is_authorized(&self, project_id: &str) -> bool {
        self.authorized_projects
            .read()
            .map(|projects| projects.contains(project_id))
            .unwrap_or(false)
    }
}

/// protocol magic直後に置くproject scope。payload側のproject_idと必ず二重照合する。
pub async fn write_project_id(send: &mut quinn::SendStream, project_id: &str) -> Result<()> {
    let bytes = project_id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_PROJECT_ID_LEN {
        return Err(SynergosNetError::Serialization(
            "project_id length outside allowed range".into(),
        ));
    }
    send.write_all(&(bytes.len() as u16).to_be_bytes())
        .await
        .map_err(|e| SynergosNetError::Quic(format!("project_id len write: {e}")))?;
    send.write_all(bytes)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("project_id write: {e}")))?;
    Ok(())
}

pub async fn read_project_id(recv: &mut quinn::RecvStream) -> Result<ProjectId> {
    let mut len = [0u8; 2];
    recv.read_exact(&mut len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("project_id len read: {e}")))?;
    let len = u16::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_PROJECT_ID_LEN {
        return Err(SynergosNetError::Serialization(
            "project_id length outside allowed range".into(),
        ));
    }
    let mut bytes = vec![0u8; len];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("project_id read: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|_| SynergosNetError::Serialization("project_id is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_denies_until_project_is_authorized() {
        let session = ConnectionSession::new(PeerId::new("peer"));
        assert!(!session.is_authorized("project-a"));
        session.authorize_project("project-a".into());
        assert!(session.is_authorized("project-a"));
        assert!(!session.is_authorized("project-b"));
    }
}
