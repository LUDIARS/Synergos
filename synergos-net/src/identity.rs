//! ノード固有の暗号鍵と PeerId の束
//!
//! Synergos のピアは起動時に ed25519 キーペアを 1 つ持つ:
//!   - 秘密鍵は `identity.key` に保存 (Unix では 0600 パーミッション)
//!   - 公開鍵は BLAKE3 でハッシュして PeerId を導出する
//!
//! ## 保存形式 (v0.1.x)
//!
//! ファイルは以下 2 形式のいずれか。先頭の magic で自動判別する。
//!
//! 1. **plaintext**: 32 byte 生バイト列 (互換性のため引き続きサポート)
//! 2. **encrypted (V1)**: `b"SYNERGOS-ID-V1\0"` (15 byte magic) + salt(16) +
//!    nonce(12) + ciphertext(32) + AES-GCM tag(16) = 91 byte
//!
//! 暗号化は **環境変数 `SYNERGOS_IDENTITY_PASSPHRASE` が設定されているときだけ**
//! 有効になる (opt-in、既存ノードを壊さないため)。passphrase から Argon2id で
//! 32 byte の鍵を導出して AES-256-GCM で暗号化する。
//!
//! 設計上の制約:
//!   - passphrase は memory に常駐するが秘匿対象は disk のみなので許容
//!   - Argon2id 計算は ~100 ms 程度で起動時に 1 回だけ走る
//!   - passphrase が変わると当然復号できないので、運用側でローテーション必要時は
//!     旧 passphrase で復号 → 新 passphrase で保存し直す

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use rand::RngCore;

use crate::types::PeerId;

/// 公開鍵から導出する PeerId の byte 長 (BLAKE3 を 20 byte に切り詰め)
pub const PEER_ID_PREFIX_LEN: usize = 20;

/// passphrase 受け渡し用の環境変数名
pub const PASSPHRASE_ENV: &str = "SYNERGOS_IDENTITY_PASSPHRASE";

/// 暗号化フォーマット V1 の magic
const ENC_MAGIC_V1: &[u8] = b"SYNERGOS-ID-V1\0";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// 暗号鍵の操作に関するエラー
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid key file: {reason}")]
    InvalidKeyFile { reason: String },

    #[error("signature verification failed")]
    VerifyFailed,

    #[error("identity is encrypted but {} is not set", PASSPHRASE_ENV)]
    PassphraseRequired,

    #[error("identity decryption failed: {reason}")]
    DecryptFailed { reason: String },

    #[error("identity encryption failed: {reason}")]
    EncryptFailed { reason: String },
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
    ///
    /// 環境変数 `SYNERGOS_IDENTITY_PASSPHRASE` が設定されていれば暗号化形式で保存し、
    /// 既存ファイルが暗号化形式なら同じ passphrase で復号する。
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

    /// 秘密鍵をファイルから読み出す。plaintext / encrypted を自動判別する。
    pub fn load_from_file(path: &Path) -> Result<Self, IdentityError> {
        let bytes = std::fs::read(path)?;
        let secret = decode_key_bytes(&bytes)?;
        let signing = SigningKey::from_bytes(&secret);
        let verifying = signing.verifying_key();
        let peer_id = peer_id_from_verifying(&verifying);
        Ok(Self {
            signing,
            verifying,
            peer_id,
        })
    }

    /// 秘密鍵をファイルに保存する。`SYNERGOS_IDENTITY_PASSPHRASE` が設定されていれば
    /// 暗号化、それ以外は plaintext。Unix では `0600` に chmod。
    pub fn save_to_file(&self, path: &Path) -> Result<(), IdentityError> {
        let secret = self.signing.to_bytes();
        let payload = match passphrase_from_env() {
            Some(pw) => encode_encrypted(&secret, pw.as_bytes())?,
            None => secret.to_vec(),
        };
        std::fs::write(path, payload)?;
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

fn passphrase_from_env() -> Option<String> {
    std::env::var(PASSPHRASE_ENV).ok().filter(|s| !s.is_empty())
}

/// ファイル先頭バイトを見て plaintext / encrypted を判別し、生 secret 32 byte を返す。
fn decode_key_bytes(bytes: &[u8]) -> Result<[u8; SECRET_KEY_LENGTH], IdentityError> {
    if bytes.starts_with(ENC_MAGIC_V1) {
        let pw = passphrase_from_env().ok_or(IdentityError::PassphraseRequired)?;
        return decode_encrypted(bytes, pw.as_bytes());
    }
    if bytes.len() != SECRET_KEY_LENGTH {
        return Err(IdentityError::InvalidKeyFile {
            reason: format!(
                "expected {SECRET_KEY_LENGTH}-byte secret key (or encrypted V1 file), got {} bytes",
                bytes.len()
            ),
        });
    }
    let mut secret = [0u8; SECRET_KEY_LENGTH];
    secret.copy_from_slice(bytes);
    Ok(secret)
}

fn encode_encrypted(
    secret: &[u8; SECRET_KEY_LENGTH],
    passphrase: &[u8],
) -> Result<Vec<u8>, IdentityError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ciphertext_with_tag =
        cipher
            .encrypt(nonce, secret.as_slice())
            .map_err(|e| IdentityError::EncryptFailed {
                reason: e.to_string(),
            })?;
    if ciphertext_with_tag.len() != SECRET_KEY_LENGTH + TAG_LEN {
        return Err(IdentityError::EncryptFailed {
            reason: format!("unexpected ciphertext length {}", ciphertext_with_tag.len()),
        });
    }

    let mut out =
        Vec::with_capacity(ENC_MAGIC_V1.len() + SALT_LEN + NONCE_LEN + SECRET_KEY_LENGTH + TAG_LEN);
    out.extend_from_slice(ENC_MAGIC_V1);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.append(&mut ciphertext_with_tag);
    Ok(out)
}

fn decode_encrypted(
    bytes: &[u8],
    passphrase: &[u8],
) -> Result<[u8; SECRET_KEY_LENGTH], IdentityError> {
    let expected_len = ENC_MAGIC_V1.len() + SALT_LEN + NONCE_LEN + SECRET_KEY_LENGTH + TAG_LEN;
    if bytes.len() != expected_len {
        return Err(IdentityError::InvalidKeyFile {
            reason: format!(
                "encrypted V1 length should be {expected_len}, got {}",
                bytes.len()
            ),
        });
    }
    let mut cursor = ENC_MAGIC_V1.len();
    let salt = &bytes[cursor..cursor + SALT_LEN];
    cursor += SALT_LEN;
    let nonce_bytes = &bytes[cursor..cursor + NONCE_LEN];
    cursor += NONCE_LEN;
    let ciphertext_with_tag = &bytes[cursor..];

    let key = derive_key(passphrase, salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext =
        cipher
            .decrypt(nonce, ciphertext_with_tag)
            .map_err(|e| IdentityError::DecryptFailed {
                reason: e.to_string(),
            })?;
    if plaintext.len() != SECRET_KEY_LENGTH {
        return Err(IdentityError::DecryptFailed {
            reason: format!(
                "decrypted length {} != {SECRET_KEY_LENGTH}",
                plaintext.len()
            ),
        });
    }
    let mut secret = [0u8; SECRET_KEY_LENGTH];
    secret.copy_from_slice(&plaintext);
    Ok(secret)
}

fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32], IdentityError> {
    let argon = Argon2::default();
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut out)
        .map_err(|e| IdentityError::EncryptFailed {
            reason: format!("argon2: {e}"),
        })?;
    Ok(out)
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

    fn unset_passphrase_for_test() {
        // 環境変数を一時的に外す。serial_test 等は使わず、各テストで明示的に
        // set/unset することで干渉を避ける。
        std::env::remove_var(PASSPHRASE_ENV);
    }

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
        unset_passphrase_for_test();
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

    #[test]
    fn encrypted_roundtrip_with_passphrase() {
        let secret = [7u8; SECRET_KEY_LENGTH];
        let blob = encode_encrypted(&secret, b"correct horse battery staple").unwrap();
        // magic prefix を確認
        assert!(blob.starts_with(ENC_MAGIC_V1));
        // 復号
        let recovered = decode_encrypted(&blob, b"correct horse battery staple").unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn wrong_passphrase_rejected() {
        let secret = [3u8; SECRET_KEY_LENGTH];
        let blob = encode_encrypted(&secret, b"good").unwrap();
        let err = decode_encrypted(&blob, b"bad").unwrap_err();
        assert!(matches!(err, IdentityError::DecryptFailed { .. }));
    }

    #[test]
    fn plaintext_file_still_loadable_when_no_passphrase() {
        unset_passphrase_for_test();
        let tmp = std::env::temp_dir().join(format!("synergos-id-pt-{}.key", uuid::Uuid::new_v4()));
        // 32 byte plaintext を直接書く (旧フォーマット相当)
        std::fs::write(&tmp, [9u8; SECRET_KEY_LENGTH]).unwrap();
        let id = Identity::load_from_file(&tmp).unwrap();
        // peer_id が deterministic に算出されている (空でないこと)
        assert!(!id.peer_id().0.is_empty());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn encrypted_file_requires_passphrase_to_load() {
        let tmp =
            std::env::temp_dir().join(format!("synergos-id-enc-{}.key", uuid::Uuid::new_v4()));
        let blob = encode_encrypted(&[1u8; SECRET_KEY_LENGTH], b"hunter2").unwrap();
        std::fs::write(&tmp, blob).unwrap();

        // 環境変数なし → エラー (Result の Err 側を pattern match で確認、Debug 要求を回避)
        unset_passphrase_for_test();
        match Identity::load_from_file(&tmp) {
            Err(IdentityError::PassphraseRequired) => {}
            Err(other) => panic!("expected PassphraseRequired, got {other:?}"),
            Ok(_) => panic!("expected Err but got Ok"),
        }

        // 環境変数あり → ロード成功
        std::env::set_var(PASSPHRASE_ENV, "hunter2");
        let id = Identity::load_from_file(&tmp).unwrap();
        assert!(!id.peer_id().0.is_empty());
        std::env::remove_var(PASSPHRASE_ENV);
        let _ = std::fs::remove_file(&tmp);
    }
}
