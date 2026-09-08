// @implements SPEC-SY-opaque-custody
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Admission {
    pub operation_id: String,
    pub opaque_object_id: String,
    pub content_hash: String,
    pub byte_length: u64,
    pub expires_at: String,
    pub content_type: String,
}
impl Admission {
    pub fn validate(&self, max_size: u64, now: DateTime<Utc>) -> Result<()> {
        let expires = DateTime::parse_from_rfc3339(&self.expires_at).map_err(|_| Error::Invalid)?;
        if uuid::Uuid::parse_str(&self.operation_id).is_err()
            || self.opaque_object_id.is_empty()
            || self.opaque_object_id.len() > 2000
            || !self.content_hash.starts_with("sha256:")
            || !hex_digest(&self.content_hash[7..])
            || self.byte_length > max_size
            || self.content_type.is_empty()
            || self.content_type.len() > 200
            || self.content_type.contains(['\r', '\n'])
            || expires <= now
            || expires > now + chrono::Duration::hours(24) + chrono::Duration::minutes(1)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        DateTime::parse_from_rfc3339(&self.expires_at).map_or(true, |expires| expires <= now)
    }
}
pub fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectRequest {
    pub opaque_object_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DownloadRequest {
    pub opaque_object_id: String,
    pub expires_in: u32,
    pub download_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub expires: i64,
    pub signature: String,
    #[serde(default)]
    pub name: String,
}
