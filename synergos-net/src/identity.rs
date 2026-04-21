//! ノード固有の暗号鍵と PeerId の束
//!
//! Synergos のピアは起動時に ed25519 キーペアを 1 つ持つ:
//!   - 秘密鍵は `identity.key` に保存 (0600 パーミッション)
//!   - 公開鍵は BLAKE3 でハッシュして PeerId を導出する
//!
//! これにより以下のセキュリティ課題をまとめて改善する:
//!   - S1 QUIC 自己署名 + no_client_auth  → 公開鍵を TLS cert に埋め、
//!     peer 側でハッシュを `PeerId` と照合して MITM / なりすましを防ぐ
//!   - S2 PeerId が裸文字列で鍵と未結合 → PeerId = BLAKE3(pubkey)[:20] (hex)
//!   - S3 Gossip 無署名          → 送信者鍵で署名 + 受信側で検証
//!   - S4 ChainBlock 無署名      → author 鍵で署名 + 受信側で検証

use std::path::{Path, PathBuf};

use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};

use crate::types::PeerId;

/// 公開鍵から導出する PeerId の byte 長 (BLAKE3 を 20 byte に切り詰め)
pub const PEER_ID_PREFIX_LEN: usize = 20;

/// 暗号鍵の操作に関するエラー
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid key file: {reason}")]
    InvalidKeyFile { reason: String },

    #[error("signature verification failed")]
    VerifyFailed,
}

/// ノード自身の暗号アイデンティティ。署名鍵 + 公開鍵 + 派生 PeerId をまとめ持つ。
pub struct Identity {
    signing: SigningKey,
    verifying: VerifyingKey,
    peer_id: PeerId,
}

impl Identity {
    /// 指定パスに `identity.key` があれば読み込み、無ければ生成して保存する。
    /// 親ディレクトリが存在しない場合は作成する。Unix では `0600` に chmod する。
    pub fn load_or_generate(path: &Path) -> Result<Self, IdentityError> {
        if path.exists() {
            Self::load_from_file(path)
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let id = Self::generate();
            id.save_to_file(path)?;
            Ok(id)
        }
    }

    /// 新規キーペアを生成する (OS 乱数)。
    pub fn generate() -> Self {
        use rand::rngs::OsRng;
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        let peer_id = peer_id_from_verifying(&verifying);
        Self {
            signing,
            verifying,
            peer_id,
        }
    }

    /// 秘密鍵 32 bytes をファイルから読み出す。
    pub fn load_from_file(path: &Path) -> Result<Self, IdentityError> {
        let bytes = std::fs::read(path)?;
        if bytes.len() != SECRET_KEY_LENGTH {
            return Err(IdentityError::InvalidKeyFile {
                reason: format!(
                    "expected {SECRET_KEY_LENGTH}-byte secret key, got {} bytes",
                    bytes.len()
                ),
            });
        }
        let mut secret = [0u8; SECRET_KEY_LENGTH];
        secret.copy_from_slice(&bytes);
        let signing = SigningKey::from_bytes(&secret);
        let verifying = signing.verifying_key();
        let peer_id = peer_id_from_verifying(&verifying);
        Ok(Self {
            signing,
            verifying,
            peer_id,
        })
    }

    /// 秘密鍵 32 bytes をファイルに保存する。Unix では `0600` に chmod。
    pub fn save_to_file(&self, path: &Path) -> Result<(), IdentityError> {
        std::fs::write(path, self.signing.to_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }
        Ok(())
    }

    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying.to_bytes()
    }

    /// 任意のバイト列に対する署名 (64 bytes)
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }

    /// 秘密鍵を PKCS#8 v2 DER で出力する (rcgen / rustls 取り込み用)。
    pub fn to_pkcs8_der(&self) -> Result<Vec<u8>, IdentityError> {
        self.signing
            .to_pkcs8_der()
            .map(|doc| doc.as_bytes().to_vec())
            .map_err(|e| IdentityError::InvalidKeyFile {
                reason: format!("pkcs8 export failed: {e}"),
            })
    }

    /// PKCS#8 DER で受け取った ed25519 秘密鍵から Identity を組み立てる。
    /// テスト用にしか使わない想定だが、`identity.key` が PKCS#8 形式で
    /// 供給された場合の読み込みにも使える。
    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self, IdentityError> {
        let signing =
            SigningKey::from_pkcs8_der(der).map_err(|e| IdentityError::InvalidKeyFile {
                reason: format!("pkcs8 import failed: {e}"),
            })?;
        let verifying = signing.verifying_key();
        let peer_id = peer_id_from_verifying(&verifying);
        Ok(Self {
            signing,
            verifying,
            peer_id,
        })
    }

    /// 既定のアイデンティティ格納パス。プロセス内で一度だけ解決する想定。
    /// - Linux: `$XDG_STATE_HOME/synergos/identity.key` or `~/.local/state/synergos/identity.key`
    /// - macOS: `~/Library/Application Support/Synergos/identity.key`
    /// - Windows: `%APPDATA%/Synergos/identity.key`
    pub fn default_path() -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            if let Ok(state) = std::env::var("XDG_STATE_HOME") {
                return PathBuf::from(state).join("synergos").join("identity.key");
            }
            if let Ok(home) = std::env::var("HOME") {
                return PathBuf::from(home).join(".local/state/synergos/identity.key");
            }
            PathBuf::from("/tmp/synergos-identity.key")
        }

        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Synergos")
                .join("identity.key")
        }

        #[cfg(target_os = "windows")]
        {
            let base = std::env::var("APPDATA")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            base.join("Synergos").join("identity.key")
        }
    }
}

/// 公開鍵から PeerId を導出する (BLAKE3(pubkey) の先頭 20 bytes を hex 化)。
///
/// PeerId をこの関数で導出した peer はネットワーク上で「鍵所有を示せる」
/// ため、相手の署名検証に成功すれば真性と扱える (S2 対策)。
pub fn peer_id_from_verifying(key: &VerifyingKey) -> PeerId {
    peer_id_from_public_bytes(&key.to_bytes())
}

pub fn peer_id_from_public_bytes(bytes: &[u8; 32]) -> PeerId {
    let hash = blake3::hash(bytes);
    let truncated = &hash.as_bytes()[..PEER_ID_PREFIX_LEN];
    PeerId::new(hex::encode(truncated))
}

/// 公開鍵 + メッセージ + 署名で検証する汎用ヘルパ。
pub fn verify(
    public_key: &[u8; 32],
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), IdentityError> {
    let vk = VerifyingKey::from_bytes(public_key).map_err(|_| IdentityError::VerifyFailed)?;
    let sig = Signature::from_bytes(signature);
    vk.verify(message, &sig)
        .map_err(|_| IdentityError::VerifyFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = Identity::generate();
        let msg = b"hello synergos";
        let sig = id.sign(msg);
        verify(&id.public_key_bytes(), msg, &sig).expect("valid signature");
    }

    #[test]
    fn tampered_message_fails_verify() {
        let id = Identity::generate();
        let sig = id.sign(b"original");
        assert!(verify(&id.public_key_bytes(), b"tampered", &sig).is_err());
    }

    #[test]
    fn load_or_generate_persists_same_peer_id() {
        let tmp = std::env::temp_dir().join(format!("synergos-id-{}.key", uuid::Uuid::new_v4()));
        let a = Identity::load_or_generate(&tmp).unwrap();
        let pid_a = a.peer_id().clone();
        drop(a);
        let b = Identity::load_or_generate(&tmp).unwrap();
        assert_eq!(&pid_a, b.peer_id());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn peer_id_is_deterministic_from_pubkey() {
        let id = Identity::generate();
        let pid = peer_id_from_public_bytes(&id.public_key_bytes());
        assert_eq!(&pid, id.peer_id());
    }
}
