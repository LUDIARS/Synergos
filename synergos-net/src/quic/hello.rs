//! アプリケーション層のピア認識 Hello (HLO1)。
//!
//! S1 QUIC 認証は「クライアントがサーバ (expected) を特定する」一方向の
//! 真性検証。サーバ側は `no_client_auth` で動くので、相手クライアントの
//! PeerId は cert から取れない (pending-{addr} になる)。本プロトコルで
//! 相手も ed25519 署名付き Hello を送ることで、サーバは相手を確定できる。
//!
//! プロトコル:
//!   1. クライアントは connect 直後に QUIC bidi `HLO1` ストリームを開き、
//!      `SignedHello { peer_id, public_key, ts_ms }` を送る。
//!   2. サーバは accept_bi 後に magic `HLO1` を確認し、署名を検証して
//!      `blake3(public_key)[:20] == peer_id` を照合する。
//!   3. サーバ側はそれを以降の全ての認識に使い、DHT / Transfer / Gossip の
//!      ストリームディスパッチもこの peer_id で紐付ける。
//!
//! 失敗時はコネクションを close する (なりすましを受け入れない)。

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::error::{Result, SynergosNetError};
use crate::identity::{self, Identity};
use crate::types::PeerId;

/// Hello ストリーム識別マジック。
pub const HELLO_STREAM_MAGIC: &[u8; 4] = b"HLO1";

/// Hello 往復のタイムアウト。サーバ側 accept 後ここでブロックする。
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// 送信される `Hello` 本体。`signing_bytes` でバイト列に落として署名する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub peer_id: PeerId,
    pub public_key: Vec<u8>, // 32 bytes
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedHello {
    pub hello: Hello,
    pub signature: Vec<u8>, // 64 bytes
}

impl SignedHello {
    pub fn new(identity: &Identity) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let hello = Hello {
            peer_id: identity.peer_id().clone(),
            public_key: identity.public_key_bytes().to_vec(),
            ts_ms: now_ms,
        };
        let sig = identity.sign(&hello.signing_bytes());
        Self {
            hello,
            signature: sig.to_vec(),
        }
    }

    /// 署名と peer_id = blake3(public_key) を検証する。成功時は中の PeerId を返す。
    pub fn verify(&self) -> Result<PeerId> {
        if self.hello.public_key.len() != 32 {
            return Err(SynergosNetError::Identity(
                "hello public_key length != 32".into(),
            ));
        }
        if self.signature.len() != 64 {
            return Err(SynergosNetError::Identity(
                "hello signature length != 64".into(),
            ));
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&self.hello.public_key);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&self.signature);

        let derived = identity::peer_id_from_public_bytes(&pk);
        if derived != self.hello.peer_id {
            return Err(SynergosNetError::Identity(
                "hello peer_id does not match public_key".into(),
            ));
        }
        identity::verify(&pk, &self.hello.signing_bytes(), &sig)
            .map_err(|_| SynergosNetError::Identity("hello signature invalid".into()))?;

        Ok(derived)
    }
}

/// クライアント側: connect 成功直後に呼ぶ。`HLO1` bidi ストリームを 1 本開き、
/// 自分の SignedHello を送る。サーバ側からの応答 (ack) は任意扱いで待たない。
pub async fn send_hello(connection: &quinn::Connection, identity: &Identity) -> Result<()> {
    let signed = SignedHello::new(identity);
    let payload = rmp_serde::to_vec(&signed)
        .map_err(|e| SynergosNetError::Serialization(format!("hello encode: {e}")))?;

    let (mut send, _recv) = connection
        .open_bi()
        .await
        .map_err(|e| SynergosNetError::Quic(format!("hello open_bi: {e}")))?;
    send.write_all(HELLO_STREAM_MAGIC)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("hello magic: {e}")))?;
    let len = (payload.len() as u32).to_be_bytes();
    send.write_all(&len)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("hello len: {e}")))?;
    send.write_all(&payload)
        .await
        .map_err(|e| SynergosNetError::Quic(format!("hello body: {e}")))?;
    send.finish()
        .map_err(|e| SynergosNetError::Quic(format!("hello finish: {e}")))?;
    Ok(())
}

/// サーバ側: QUIC 接続確立直後、最初の bidi ストリームから HLO1 を読んで
/// 相手の PeerId を返す。タイムアウトまでに届かなければエラー。
pub async fn recv_hello(connection: &quinn::Connection) -> Result<PeerId> {
    let accept_fut = async {
        let (_send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|e| SynergosNetError::Quic(format!("hello accept_bi: {e}")))?;

        let mut magic = [0u8; 4];
        recv.read_exact(&mut magic)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("hello magic: {e}")))?;
        if &magic != HELLO_STREAM_MAGIC {
            return Err(SynergosNetError::Identity(format!(
                "unexpected first-stream magic: expected HLO1, got {magic:?}"
            )));
        }
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("hello len: {e}")))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 4096 {
            return Err(SynergosNetError::Identity(format!(
                "hello too large: {len}"
            )));
        }
        let mut body = vec![0u8; len];
        recv.read_exact(&mut body)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("hello body: {e}")))?;

        let signed: SignedHello = rmp_serde::from_slice(&body)
            .map_err(|e| SynergosNetError::Serialization(format!("hello decode: {e}")))?;
        signed.verify()
    };

    timeout(HELLO_TIMEOUT, accept_fut)
        .await
        .map_err(|_| SynergosNetError::Identity("hello timed out".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_hello_roundtrip() {
        let id = Identity::generate();
        let signed = SignedHello::new(&id);
        let peer_id = signed.verify().unwrap();
        assert_eq!(&peer_id, id.peer_id());
    }

    #[test]
    fn tampered_public_key_rejected() {
        let id = Identity::generate();
        let mut signed = SignedHello::new(&id);
        // 公開鍵の 1 バイトをフリップ → derived != peer_id もしくは署名検証失敗
        signed.hello.public_key[0] ^= 0xFF;
        assert!(signed.verify().is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let id = Identity::generate();
        let mut signed = SignedHello::new(&id);
        signed.signature[0] ^= 0xFF;
        assert!(signed.verify().is_err());
    }
}
