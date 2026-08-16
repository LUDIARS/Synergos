//! ノードローカルのファイル版番号高水位。
//!
//! `.synergos/manifest.json` は git checkout / restore で過去版へ戻るため、
//! publish の単調増加番号を守るための「観測済み最大版」は別のローカル状態に
//! 永続化する。`state.json` は git 管理せず、プロジェクトごとに保持する。

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::manifest::{
    metadata_file_path, prepare_metadata_dir, replace_file_atomically, safe_rel_to_local,
};

pub const VERSION_STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionState {
    #[serde(default = "default_format")]
    format: u32,
    project_id: String,
    #[serde(default)]
    versions: BTreeMap<String, u64>,
}

fn default_format() -> u32 {
    1
}

impl VersionState {
    pub fn new(project_id: &str) -> Self {
        Self {
            format: 1,
            project_id: project_id.to_string(),
            versions: BTreeMap::new(),
        }
    }

    pub async fn load(root: &Path, project_id: &str) -> io::Result<Self> {
        let path = metadata_file_path(root, VERSION_STATE_FILE)?;
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::new(project_id));
            }
            Err(error) => return Err(error),
        };
        let state: Self = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if state.format != 1 || state.project_id != project_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "version state does not match the open project",
            ));
        }
        if state
            .versions
            .iter()
            .any(|(rel, version)| !is_valid_entry(rel, *version))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "version state contains an invalid path or version",
            ));
        }
        Ok(state)
    }

    pub async fn save(&self, root: &Path) -> io::Result<()> {
        let metadata_dir = prepare_metadata_dir(root).await?;
        let path = metadata_dir.join(VERSION_STATE_FILE);
        let tmp = metadata_dir.join(format!("state-{}.tmp", uuid::Uuid::new_v4()));
        let json = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        tokio::fs::write(&tmp, json).await?;
        if let Err(error) = replace_file_atomically(&tmp, &path).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        Ok(())
    }

    pub fn highest(&self, rel: &str) -> u64 {
        self.versions.get(rel).copied().unwrap_or(0)
    }

    /// 高水位が上がった場合だけ `true` を返す。
    pub fn note(&mut self, rel: &str, version: u64) -> io::Result<bool> {
        if !is_valid_entry(rel, version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid path or version for version state",
            ));
        }
        let current = self.highest(rel);
        if version <= current {
            return Ok(false);
        }
        self.versions.insert(rel.to_string(), version);
        Ok(true)
    }
}

fn is_valid_entry(rel: &str, version: u64) -> bool {
    safe_rel_to_local(rel).is_some() && version > 0 && version < u64::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_only_moves_forward_and_rejects_unsafe_entries() {
        let mut state = VersionState::new("p");
        assert!(state.note("assets/a.bin", 4).unwrap());
        assert!(!state.note("assets/a.bin", 2).unwrap());
        assert_eq!(state.highest("assets/a.bin"), 4);
        assert!(state.note("../escape", 5).is_err());
        assert!(state.note("assets/a.bin", u64::MAX).is_err());
    }
}
