//! アプリケーション層の相互ピア認証 Hello。
//!
//! HLO1 の wire 型は互換のため残すが、server は replay 耐性のない HLO1 を
//! 受理しない。実接続は HLO2 の server nonce challenge と TLS exporter の
//! channel binding を使う。

use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::error::{Result, SynergosNetError};
use crate::identity::{self, Identity};
use crate::types::PeerId;

/// replay 耐性のない旧 magic。server は明示的に拒否する。
pub const HELLO1_STREAM_MAGIC: &[u8; 4] = b"HLO1";
/// server nonce challenge を使う現行 magic。
pub const HELLO2_STREAM_MAGIC: &[u8; 4] = b"HLO2";
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

const SERVER_NONCE_LEN: usize = 32;
const TLS_EXPORTER_LEN: usize = 32;
const TLS_EXPORTER_LABEL: &[u8] = b"synergos-hlo2";
const TLS_EXPORTER_CONTEXT: &[u8] = b"";
const MAX_HELLO_BODY: usize = 4096;
const MAX_CLOCK_SKEW_MS: u64 = 60_000;

/// HLO1 本体。wire decode 互換のためのみ残す。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub peer_id: PeerId,
    pub public_key: Vec<u8>,
    pub ts_ms: u64,
}

impl Hello {
    fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.public_key.len());
        out.extend_from_slice(self.peer_id.0.as_bytes());
        out.push(0);
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(&self.ts_ms.to_le_bytes());
        out
    }
}

/// HLO1 署名 envelope。server 検証経路からは使用しない。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedHello {
    pub hello: Hello,
    pub signature: Vec<u8>,
}

impl SignedHello {
    pub fn new(identity: &Identity) -> Self {
        let hello = Hello {
            peer_id: identity.peer_id().clone(),
            public_key: identity.public_key_bytes().to_vec(),
            ts_ms: now_ms(),
        };
        let signature = identity.sign(&hello.signing_bytes()).to_vec();
        Self { hello, signature }
    }
}

/// HLO2 の署名対象と署名を一体化した wire 型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedHello2 {
    pub peer_id: PeerId,
    pub public_key: Vec<u8>,
    pub server_nonce: [u8; SERVER_NONCE_LEN],
    pub tls_exporter: [u8; TLS_EXPORTER_LEN],
    pub ts_ms: u64,
    pub signature: Vec<u8>,
}

impl SignedHello2 {
    pub fn new(
        identity: &Identity,
        server_nonce: [u8; SERVER_NONCE_LEN],
        tls_exporter: [u8; TLS_EXPORTER_LEN],
    ) -> Self {
        Self::new_at(identity, server_nonce, tls_exporter, now_ms())
    }

    fn new_at(
        identity: &Identity,
        server_nonce: [u8; SERVER_NONCE_LEN],
        tls_exporter: [u8; TLS_EXPORTER_LEN],
        ts_ms: u64,
    ) -> Self {
        let mut signed = Self {
            peer_id: identity.peer_id().clone(),
            public_key: identity.public_key_bytes().to_vec(),
            server_nonce,
            tls_exporter,
            ts_ms,
            signature: Vec::new(),
        };
        signed.signature = identity.sign(&signed.signing_bytes()).to_vec();
        signed
    }

    fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(160 + self.public_key.len());
        out.extend_from_slice(HELLO2_STREAM_MAGIC);
        out.extend_from_slice(self.peer_id.0.as_bytes());
        out.push(0);
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(&self.server_nonce);
        out.extend_from_slice(&self.tls_exporter);
        out.extend_from_slice(&self.ts_ms.to_le_bytes());
        out
    }

    fn verify_fields(
        &self,
        expected_nonce: &[u8; SERVER_NONCE_LEN],
        expected_exporter: &[u8; TLS_EXPORTER_LEN],
        current_ms: u64,
    ) -> Result<PeerId> {
        if self.public_key.len() != 32 {
            return Err(SynergosNetError::Identity(
                "HLO2 public_key length != 32".into(),
            ));
        }
        if self.signature.len() != 64 {
            return Err(SynergosNetError::Identity(
                "HLO2 signature length != 64".into(),
            ));
        }
        if &self.server_nonce != expected_nonce {
            return Err(SynergosNetError::Identity(
                "HLO2 server nonce mismatch".into(),
            ));
        }
        if &self.tls_exporter != expected_exporter {
            return Err(SynergosNetError::Identity(
                "HLO2 TLS exporter mismatch".into(),
            ));
        }
        if self.ts_ms.abs_diff(current_ms) > MAX_CLOCK_SKEW_MS {
            return Err(SynergosNetError::Identity(
                "HLO2 timestamp outside allowed skew".into(),
            ));
        }

        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&self.public_key);
        let derived = identity::peer_id_from_public_bytes(&public_key);
        if derived != self.peer_id {
            return Err(SynergosNetError::Identity(
                "HLO2 peer_id does not match public_key".into(),
            ));
        }

        let mut signature = [0u8; 64];
        signature.copy_from_slice(&self.signature);
        identity::verify(&public_key, &self.signing_bytes(), &signature)
            .map_err(|_| SynergosNetError::Identity("HLO2 signature invalid".into()))?;
        Ok(derived)
    }
}

/// 1 接続専用の server challenge。一度検証を試みた nonce は再使用できない。
struct ServerChallenge {
    nonce: [u8; SERVER_NONCE_LEN],
    consumed: bool,
}

impl ServerChallenge {
    fn random() -> Self {
        let mut nonce = [0u8; SERVER_NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        Self {
            nonce,
            consumed: false,
        }
    }

    #[cfg(test)]
    fn fixed(nonce: [u8; SERVER_NONCE_LEN]) -> Self {
        Self {
            nonce,
            consumed: false,
        }
    }

    fn verify_once(
        &mut self,
        signed: &SignedHello2,
        expected_exporter: &[u8; TLS_EXPORTER_LEN],
        current_ms: u64,
    ) -> Result<PeerId> {
        if self.consumed {
            return Err(SynergosNetError::Identity(
                "HLO2 server nonce already consumed".into(),
            ));
        }
        self.consumed = true;
        signed.verify_fields(&self.nonce, expected_exporter, current_ms)
    }
}

fn tls_exporter(connection: &quinn::Connection) -> Result<[u8; TLS_EXPORTER_LEN]> {
    let mut output = [0u8; TLS_EXPORTER_LEN];
    connection
        .export_keying_material(&mut output, TLS_EXPORTER_LABEL, TLS_EXPORTER_CONTEXT)
        .map_err(|e| SynergosNetError::Identity(format!("HLO2 TLS exporter failed: {e:?}")))?;
    Ok(output)
}

/// client: HLO2 stream を開き、server nonce を受け取って channel-bound Hello を返す。
pub async fn send_hello2(connection: &quinn::Connection, identity: &Identity) -> Result<()> {
    let exchange = async {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 open_bi: {e}")))?;
        send.write_all(HELLO2_STREAM_MAGIC)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 magic: {e}")))?;

        let mut server_nonce = [0u8; SERVER_NONCE_LEN];
        recv.read_exact(&mut server_nonce)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 server nonce: {e}")))?;
        let signed = SignedHello2::new(identity, server_nonce, tls_exporter(connection)?);
        let payload = rmp_serde::to_vec(&signed)
            .map_err(|e| SynergosNetError::Serialization(format!("HLO2 encode: {e}")))?;
        send.write_all(&(payload.len() as u32).to_be_bytes())
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 len: {e}")))?;
        send.write_all(&payload)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 body: {e}")))?;
        send.finish()
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 finish: {e}")))?;
        Ok(())
    };

    timeout(HELLO_TIMEOUT, exchange)
        .await
        .map_err(|_| SynergosNetError::Identity("HLO2 send timed out".into()))?
}

/// server: 最初の bidi stream で HLO2 challenge を実行し、認証済み PeerId を返す。
pub async fn recv_hello2(connection: &quinn::Connection) -> Result<PeerId> {
    let exchange = async {
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 accept_bi: {e}")))?;
        let mut magic = [0u8; 4];
        recv.read_exact(&mut magic)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 magic: {e}")))?;
        if &magic == HELLO1_STREAM_MAGIC {
            return Err(SynergosNetError::Identity(
                "HLO1 rejected: replay-resistant HLO2 required".into(),
            ));
        }
        if &magic != HELLO2_STREAM_MAGIC {
            return Err(SynergosNetError::Identity(format!(
                "unexpected first-stream magic: expected HLO2, got {magic:?}"
            )));
        }

        let mut challenge = ServerChallenge::random();
        send.write_all(&challenge.nonce)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 server nonce: {e}")))?;
        send.finish()
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 nonce finish: {e}")))?;

        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 len: {e}")))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_HELLO_BODY {
            return Err(SynergosNetError::Identity(format!("HLO2 too large: {len}")));
        }
        let mut body = vec![0u8; len];
        recv.read_exact(&mut body)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("HLO2 body: {e}")))?;
        let signed: SignedHello2 = rmp_serde::from_slice(&body)
            .map_err(|e| SynergosNetError::Serialization(format!("HLO2 decode: {e}")))?;
        challenge.verify_once(&signed, &tls_exporter(connection)?, now_ms())
    };

    timeout(HELLO_TIMEOUT, exchange)
        .await
        .map_err(|_| SynergosNetError::Identity("HLO2 receive timed out".into()))?
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE_A: [u8; SERVER_NONCE_LEN] = [0xA5; SERVER_NONCE_LEN];
    const NONCE_B: [u8; SERVER_NONCE_LEN] = [0xB6; SERVER_NONCE_LEN];
    const EXPORTER_A: [u8; TLS_EXPORTER_LEN] = [0x11; TLS_EXPORTER_LEN];
    const EXPORTER_B: [u8; TLS_EXPORTER_LEN] = [0x22; TLS_EXPORTER_LEN];

    #[test]
    fn signed_hello2_roundtrip() {
        let identity = Identity::generate();
        let signed = SignedHello2::new(&identity, NONCE_A, EXPORTER_A);
        let mut challenge = ServerChallenge::fixed(NONCE_A);
        assert_eq!(
            challenge
                .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
                .unwrap(),
            *identity.peer_id()
        );
    }

    #[test]
    fn tampered_public_key_rejected() {
        let identity = Identity::generate();
        let mut signed = SignedHello2::new(&identity, NONCE_A, EXPORTER_A);
        signed.public_key[0] ^= 0xFF;
        let mut challenge = ServerChallenge::fixed(NONCE_A);
        assert!(challenge
            .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
            .is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let identity = Identity::generate();
        let mut signed = SignedHello2::new(&identity, NONCE_A, EXPORTER_A);
        signed.signature[0] ^= 0xFF;
        let mut challenge = ServerChallenge::fixed(NONCE_A);
        assert!(challenge
            .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
            .is_err());
    }

    #[test]
    fn replayed_hello_rejected() {
        let identity = Identity::generate();
        let signed = SignedHello2::new(&identity, NONCE_A, EXPORTER_A);

        let mut different_nonce = ServerChallenge::fixed(NONCE_B);
        assert!(different_nonce
            .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
            .is_err());
        let mut different_connection = ServerChallenge::fixed(NONCE_A);
        assert!(different_connection
            .verify_once(&signed, &EXPORTER_B, signed.ts_ms)
            .is_err());
    }

    #[test]
    fn stale_timestamp_rejected() {
        let identity = Identity::generate();
        let current = now_ms();
        let signed = SignedHello2::new_at(&identity, NONCE_A, EXPORTER_A, current - 61_000);
        let mut challenge = ServerChallenge::fixed(NONCE_A);
        assert!(challenge
            .verify_once(&signed, &EXPORTER_A, current)
            .is_err());
    }

    #[test]
    fn nonce_reuse_rejected() {
        let identity = Identity::generate();
        let signed = SignedHello2::new(&identity, NONCE_A, EXPORTER_A);
        let mut challenge = ServerChallenge::fixed(NONCE_A);
        assert!(challenge
            .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
            .is_ok());
        assert!(challenge
            .verify_once(&signed, &EXPORTER_A, signed.ts_ms)
            .is_err());
    }
}
