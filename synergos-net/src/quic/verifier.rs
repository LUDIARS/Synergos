//! S1 ピア真性認証の要: サーバ証明書の ed25519 公開鍵を取り出し、
//! `blake3(pubkey)[:20]` が期待する PeerId と一致するかを検証する。
//!
//! rustls の `ServerCertVerifier` として差し込むことで、TLS ハンドシェイク
//! 時点でなりすましを拒否できる。`verify_tls13_signature` /
//! `verify_tls12_signature` は rustls の ring backend に委譲し、
//! ハンドシェイクトランスクリプト署名の検証は暗号ライブラリに任せる。

use std::sync::Arc;

use rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

use crate::identity::peer_id_from_public_bytes;
use crate::types::PeerId;

/// ed25519 公開鍵の OID: `1.3.101.112`
const OID_ED25519: &[u64] = &[1, 3, 101, 112];

#[derive(Debug)]
pub struct PeerPinningVerifier {
    expected_peer_id: PeerId,
    /// 実際に signature 検証を担う rustls の ring provider
    crypto_provider: Arc<rustls::crypto::CryptoProvider>,
}

impl PeerPinningVerifier {
    pub fn new(expected_peer_id: PeerId) -> Self {
        Self {
            expected_peer_id,
            crypto_provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }

    /// 証明書から ed25519 pubkey を取り出して期待値と照合する。
    /// 取り出した生の 32-byte pubkey を返す (後続で署名検証に使える)。
    fn check_peer_binding<'a>(
        &self,
        end_entity: &'a CertificateDer<'a>,
    ) -> Result<Vec<u8>, rustls::Error> {
        let pubkey = extract_ed25519_spki(end_entity.as_ref()).ok_or_else(|| {
            rustls::Error::General(
                "PeerPinning: server cert does not expose an ed25519 public key".to_string(),
            )
        })?;

        if pubkey.len() != 32 {
            return Err(rustls::Error::General(format!(
                "PeerPinning: unexpected ed25519 public key length {}",
                pubkey.len()
            )));
        }
        let mut key_arr = [0u8; 32];
        key_arr.copy_from_slice(&pubkey);

        let derived = peer_id_from_public_bytes(&key_arr);
        if derived != self.expected_peer_id {
            return Err(rustls::Error::General(format!(
                "PeerPinning: peer identity mismatch (expected {}, got {})",
                self.expected_peer_id.short(),
                derived.short(),
            )));
        }
        Ok(pubkey)
    }
}

impl ServerCertVerifier for PeerPinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.check_peer_binding(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.crypto_provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.crypto_provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // ed25519 のみを許可することで、相手が ed25519 以外の鍵で
        // 署名した場合はここで弾ける (= Synergos の鍵と一致しない)。
        vec![SignatureScheme::ED25519]
    }
}

/// サーバ証明書 DER から ed25519 SPKI を取り出すヘルパ。
/// x509-parser で `subjectPublicKeyInfo.algorithm.algorithm` が
/// `1.3.101.112` (id-Ed25519) のときだけ pubkey bytes を返す。
pub(crate) fn extract_ed25519_spki(cert_der: &[u8]) -> Option<Vec<u8>> {
    use x509_parser::prelude::*;

    let (_, parsed) = X509Certificate::from_der(cert_der).ok()?;
    let spki = parsed.public_key();
    let oid_iter: Vec<u64> = spki.algorithm.algorithm.iter()?.collect();
    if oid_iter.as_slice() != OID_ED25519 {
        return None;
    }
    Some(spki.subject_public_key.data.to_vec())
}

/// `accept` 時に相手の証明書 DER から PeerId を再計算するためのヘルパ。
/// サーバ側は現状クライアント証明書を要求していないので、ここが呼ばれるのは
/// 相互認証を導入したときに限られる。
pub(crate) fn peer_id_from_cert(cert_der: &[u8]) -> Option<PeerId> {
    let pubkey = extract_ed25519_spki(cert_der)?;
    if pubkey.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pubkey);
    Some(peer_id_from_public_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn self_signed_cert(identity: &Identity) -> rustls::pki_types::CertificateDer<'static> {
        let pkcs8 = identity.to_pkcs8_der().unwrap();
        let key_pair = rcgen::KeyPair::try_from(pkcs8.as_slice()).unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["synergos".into()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, identity.peer_id().to_string());
        let cert = params.self_signed(&key_pair).unwrap();
        rustls::pki_types::CertificateDer::from(cert.der().to_vec())
    }

    #[test]
    fn verifier_accepts_matching_peer_id() {
        let id = Identity::generate();
        let cert = self_signed_cert(&id);
        let verifier = PeerPinningVerifier::new(id.peer_id().clone());
        let server_name = rustls::pki_types::ServerName::try_from("synergos").unwrap();
        let now = rustls::pki_types::UnixTime::now();
        let verified = verifier
            .verify_server_cert(&cert, &[], &server_name, &[], now)
            .unwrap();
        let _ = verified;
    }

    #[test]
    fn verifier_rejects_mismatched_peer_id() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let cert = self_signed_cert(&id_a);
        let verifier = PeerPinningVerifier::new(id_b.peer_id().clone());
        let server_name = rustls::pki_types::ServerName::try_from("synergos").unwrap();
        let now = rustls::pki_types::UnixTime::now();
        let err = verifier
            .verify_server_cert(&cert, &[], &server_name, &[], now)
            .unwrap_err();
        match err {
            rustls::Error::General(msg) => assert!(msg.contains("mismatch")),
            other => panic!("expected General, got {other:?}"),
        }
    }

    #[test]
    fn peer_id_from_cert_matches_identity() {
        let id = Identity::generate();
        let cert = self_signed_cert(&id);
        let recovered = peer_id_from_cert(cert.as_ref()).unwrap();
        assert_eq!(&recovered, id.peer_id());
    }
}
