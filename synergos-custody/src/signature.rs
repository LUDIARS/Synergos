// @implements SPEC-SY-opaque-custody
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub fn object_key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}
pub fn canonical(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        Value::Object(items) => {
            let mut pairs = items.iter().collect::<Vec<_>>();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            format!(
                "{{{}}}",
                pairs
                    .into_iter()
                    .map(|(k, v)| format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("string serialization"),
                        canonical(v)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        _ => value.to_string(),
    }
}
pub fn sign(secret: &str, value: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    format!("{:x}", mac.finalize().into_bytes())
}
pub fn verify(secret: &str, value: &str, signature: &str) -> bool {
    if !crate::contract::hex_digest(signature) {
        return false;
    }
    let bytes: Vec<u8> = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&signature[i..i + 2], 16).expect("validated hex"))
        .collect();
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(value.as_bytes());
    mac.verify_slice(&bytes).is_ok()
}
pub fn capability_message(method: &str, key: &str, expires: i64, name: &str) -> String {
    canonical(&serde_json::json!({"method":method,"key":key,"expires":expires,"name":name}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_signature_matches_curare_node_crypto_fixture() {
        let receipt = serde_json::json!({"operationId":"11111111-1111-4111-8111-111111111111","opaqueObjectId":"uploads/asset/object",
            "verifiedDigests":[{"algorithm":"sha256","value":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}],
            "byteLength":3,"durability":"durable","locations":["22222222-2222-4222-8222-222222222222"],"expiresAt":"2030-01-01T00:00:00.000Z"});
        // Public dummy fixture, shared with Cu; never an operational credential.
        let secret = "dummy-receipt-key-for-cu-synergos-tests";
        let signature = sign(secret, &canonical(&receipt));
        assert_eq!(
            signature,
            "cb241fe664ef6b04b86724ce98a6a6d410ff4b6e0f964ed88d68fb73210863cf"
        );
        assert!(verify(secret, &canonical(&receipt), &signature));
        assert!(!verify(secret, "tampered", &signature));
    }
}
