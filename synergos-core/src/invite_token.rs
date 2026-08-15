//! 自己完結型 (self-contained) 招待トークン — 別マシンの daemon で `project join` できる形式。
//!
//! 従来の招待トークンは発行 daemon のメモリ (`ProjectManager::invites`) にしか無く、
//! 別マシンで `project join <token>` すると「invalid invite token」になっていた
//! (トークンが相手に伝わる経路が無い)。本モジュールのトークンは
//! **参加に必要な情報をすべて内包し、発行ノードの鍵で署名**する:
//!
//! ```text
//! syn1.<base64url(payload JSON)>.<base64url(ed25519 signature)>
//! ```
//!
//! payload = { project_id, display_name, host_peer_id, host_pubkey, peer_info_url,
//!             expires_at, nonce }
//!
//! 参加側は署名を検証 → 同じ project_id でローカルに open → `peer_info_url` に
//! bootstrap (QUIC 接続) → 相手 peer_id が `host_peer_id` と一致することを確認する。
//! 発行側での使用回数管理はできない (相手が検証するため)。有効期限で縛る。
//!
//! 注意: 現状 Synergos にはプロジェクト単位の参加 ACL が無い (project_id と
//! 到達性さえあれば誰でも参加できる) ので、このトークンは「参加手順の同梱」
//! であって認可境界ではない。認可は Cloudflare Mesh 等のネットワーク層で行う。

use base64::Engine;
use serde::{Deserialize, Serialize};
use synergos_net::identity::{peer_id_from_public_bytes, verify, Identity};
use synergos_net::types::PeerId;

pub const PREFIX: &str = "syn1";
const MAX_TOKEN_LEN: usize = 16 * 1024;
const MAX_PROJECT_ID_LEN: usize = 256;
const MAX_DISPLAY_NAME_LEN: usize = 256;
const MAX_PEER_INFO_URL_LEN: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvitePayload {
    pub project_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub host_peer_id: String,
    /// ed25519 公開鍵 (32 bytes, base64url)
    pub host_pubkey: String,
    /// 発行ノードの `/peer-info` サーブレット URL (例 `http://100.96.0.5:7780`)
    pub peer_info_url: String,
    /// epoch 秒。None = 無期限
    #[serde(default)]
    pub expires_at: Option<u64>,
    pub nonce: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteTokenError {
    #[error("not a self-contained invite token")]
    NotSelfContained,
    #[error("malformed invite token: {0}")]
    Malformed(String),
    #[error("invite token signature invalid")]
    BadSignature,
    #[error("invite token host key does not match host peer id")]
    HostMismatch,
    #[error("invite token expired")]
    Expired,
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

/// トークンを発行する。
pub fn encode(identity: &Identity, payload: &InvitePayload) -> String {
    let json = serde_json::to_vec(payload).expect("payload serializable");
    let sig = identity.sign(&json);
    format!("{PREFIX}.{}.{}", b64().encode(&json), b64().encode(sig))
}

/// 自己完結型トークンかどうか (先頭プレフィクスで判定)。
pub fn is_self_contained(token: &str) -> bool {
    token.starts_with(&format!("{PREFIX}."))
}

/// 署名・鍵・期限を検証して payload を返す。
pub fn decode(token: &str, now_epoch_secs: u64) -> Result<InvitePayload, InviteTokenError> {
    if !is_self_contained(token) {
        return Err(InviteTokenError::NotSelfContained);
    }
    if token.len() > MAX_TOKEN_LEN {
        return Err(InviteTokenError::Malformed("token too large".into()));
    }
    let mut parts = token.splitn(3, '.');
    let _prefix = parts.next();
    let payload_b64 = parts
        .next()
        .ok_or_else(|| InviteTokenError::Malformed("missing payload".into()))?;
    let sig_b64 = parts
        .next()
        .ok_or_else(|| InviteTokenError::Malformed("missing signature".into()))?;
    let json = b64()
        .decode(payload_b64)
        .map_err(|e| InviteTokenError::Malformed(format!("payload b64: {e}")))?;
    let sig_vec = b64()
        .decode(sig_b64)
        .map_err(|e| InviteTokenError::Malformed(format!("signature b64: {e}")))?;
    let sig: [u8; 64] = sig_vec
        .try_into()
        .map_err(|_| InviteTokenError::Malformed("signature length".into()))?;
    let payload: InvitePayload = serde_json::from_slice(&json)
        .map_err(|e| InviteTokenError::Malformed(format!("payload json: {e}")))?;
    if payload.project_id.is_empty()
        || payload.project_id.len() > MAX_PROJECT_ID_LEN
        || payload.project_id.chars().any(char::is_control)
        || payload.project_id.contains('/')
        || payload.project_id.contains('\\')
    {
        return Err(InviteTokenError::Malformed("invalid project id".into()));
    }
    if payload.peer_info_url.is_empty() || payload.peer_info_url.len() > MAX_PEER_INFO_URL_LEN {
        return Err(InviteTokenError::Malformed(
            "invalid peer-info URL length".into(),
        ));
    }
    if payload.display_name.as_ref().is_some_and(|name| {
        name.len() > MAX_DISPLAY_NAME_LEN || name.chars().any(char::is_control)
    }) {
        return Err(InviteTokenError::Malformed("invalid display name".into()));
    }
    let pk_vec = b64()
        .decode(&payload.host_pubkey)
        .map_err(|e| InviteTokenError::Malformed(format!("pubkey b64: {e}")))?;
    let pk: [u8; 32] = pk_vec
        .try_into()
        .map_err(|_| InviteTokenError::Malformed("pubkey length".into()))?;
    verify(&pk, &json, &sig).map_err(|_| InviteTokenError::BadSignature)?;
    if peer_id_from_public_bytes(&pk) != PeerId::new(payload.host_peer_id.clone()) {
        return Err(InviteTokenError::HostMismatch);
    }
    if let Some(exp) = payload.expires_at {
        if now_epoch_secs > exp {
            return Err(InviteTokenError::Expired);
        }
    }
    Ok(payload)
}

/// 発行ヘルパ: identity から host_peer_id / host_pubkey を埋める。
pub fn issue(
    identity: &Identity,
    project_id: &str,
    display_name: Option<String>,
    peer_info_url: &str,
    expires_at: Option<u64>,
) -> (String, InvitePayload) {
    let payload = InvitePayload {
        project_id: project_id.to_string(),
        display_name,
        host_peer_id: identity.peer_id().0.clone(),
        host_pubkey: b64().encode(identity.public_key_bytes()),
        peer_info_url: peer_info_url.to_string(),
        expires_at,
        nonce: uuid::Uuid::new_v4().to_string(),
    };
    let token = encode(identity, &payload);
    (token, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_verify() {
        let id = Identity::generate();
        let (token, payload) = issue(
            &id,
            "proj",
            Some("P".into()),
            "http://10.0.0.1:7780",
            Some(2_000_000_000),
        );
        assert!(is_self_contained(&token));
        let decoded = decode(&token, 1_000_000_000).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(decoded.host_peer_id, id.peer_id().0);
    }

    #[test]
    fn expired_is_rejected() {
        let id = Identity::generate();
        let (token, _) = issue(&id, "proj", None, "http://h:1", Some(100));
        assert_eq!(decode(&token, 101), Err(InviteTokenError::Expired));
        assert!(decode(&token, 100).is_ok());
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let id = Identity::generate();
        let (token, mut payload) = issue(&id, "proj", None, "http://h:1", None);
        payload.project_id = "other".into();
        let json = serde_json::to_vec(&payload).unwrap();
        let sig_part = token.rsplit('.').next().unwrap();
        let forged = format!("{PREFIX}.{}.{}", b64().encode(&json), sig_part);
        assert_eq!(decode(&forged, 0), Err(InviteTokenError::BadSignature));
    }

    #[test]
    fn foreign_key_is_rejected() {
        let host = Identity::generate();
        let other = Identity::generate();
        let (_, mut payload) = issue(&host, "proj", None, "http://h:1", None);
        // 別鍵で署名し直し、pubkey も差し替えるが host_peer_id は元のまま
        payload.host_pubkey = b64().encode(other.public_key_bytes());
        let token = encode(&other, &payload);
        assert_eq!(decode(&token, 0), Err(InviteTokenError::HostMismatch));
    }

    #[test]
    fn legacy_token_is_not_self_contained() {
        assert_eq!(
            decode("2b1c8b0e-1111-2222-3333-444444444444", 0),
            Err(InviteTokenError::NotSelfContained)
        );
    }
}
