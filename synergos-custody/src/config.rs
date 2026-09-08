// @implements SPEC-SY-custody-durability
use std::{env, io, path::PathBuf};
use url::Url;

/// Secrets stay in process memory; they are never part of receipts or store metadata.
pub struct Config {
    pub root: PathBuf,
    pub public_url: Url,
    pub node_id: String,
    pub token: String,
    pub receipt_secret: String,
    pub max_object_bytes: u64,
    pub client_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> io::Result<Option<Self>> {
        match env::var("SYNERGOS_CUSTODY_ENABLED").as_deref() {
            Err(_) | Ok("0") => return Ok(None),
            Ok("1") => {}
            _ => return Err(io::Error::other("SYNERGOS_CUSTODY_ENABLED must be 0 or 1")),
        }
        let config = Self {
            root: required("SYNERGOS_CUSTODY_ROOT")?.into(),
            public_url: Url::parse(&required("SYNERGOS_CUSTODY_PUBLIC_URL")?)
                .map_err(|_| io::Error::other("invalid custody public URL"))?,
            node_id: required("SYNERGOS_CUSTODY_NODE_ID")?,
            token: required("SYNERGOS_CUSTODY_TOKEN")?,
            receipt_secret: required("SYNERGOS_CUSTODY_RECEIPT_SECRET")?,
            max_object_bytes: required("SYNERGOS_CUSTODY_MAX_OBJECT_BYTES")?
                .parse()
                .map_err(|_| io::Error::other("invalid maximum object size"))?,
            client_origins: required("SYNERGOS_CUSTODY_CLIENT_ORIGINS")?
                .split(',')
                .map(|v| v.trim().to_owned())
                .collect(),
        };
        config.validate()?;
        Ok(Some(config))
    }

    pub fn validate(&self) -> io::Result<()> {
        if !self.root.is_absolute()
            || self.token.len() < 32
            || self.receipt_secret.len() < 32
            || self.max_object_bytes == 0
            || self.max_object_bytes > 9_007_199_254_740_991
            || uuid::Uuid::parse_str(&self.node_id).is_err()
            || !is_origin(&self.public_url)
            || self.client_origins.iter().any(|s| {
                Url::parse(s).map_or(true, |u| {
                    !is_origin(&u) || u.origin().ascii_serialization() != *s
                })
            })
        {
            return Err(io::Error::other("invalid custody configuration; absolute root, UUID node, strong secrets and HTTP(S) origins required"));
        }
        Ok(())
    }
}
fn required(name: &str) -> io::Result<String> {
    env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| io::Error::other(format!("{name} is required")))
}
fn is_origin(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/"
}
