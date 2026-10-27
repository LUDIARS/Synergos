//! Explicit origin policy and bounded HTTP I/O. Capability URLs never enter errors.
use super::MAX_BLOCK_BYTES;
use anyhow::{bail, ensure, Result};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, StatusCode, Url,
};
use std::{collections::BTreeMap, io::Read, path::Path, time::Duration};

pub(crate) struct HttpClient {
    client: Client,
    origins: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("block request returned HTTP {0}")]
pub(crate) struct HttpStatus(pub u16);

impl HttpClient {
    pub fn new(origins: &[String], ca_bundle: Option<&Path>) -> Result<Self> {
        ensure!(
            !origins.is_empty(),
            "at least one explicitly allowed origin is required"
        );
        let origins = origins
            .iter()
            .map(|s| {
                let url = parse_url(s)?;
                ensure!(
                    url.path() == "/" && url.query().is_none(),
                    "allowlist entries must be origins"
                );
                Ok(url.origin().ascii_serialization())
            })
            .collect::<Result<Vec<_>>>()?;
        let mut builder = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120));
        if let Some(path) = ca_bundle {
            let mut pem = Vec::new();
            std::fs::File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut pem)?;
            ensure!(pem.len() <= 1024 * 1024, "CA bundle exceeds 1 MiB");
            let certificates = reqwest::Certificate::from_pem_bundle(&pem)?;
            ensure!(
                !certificates.is_empty(),
                "CA bundle contains no certificates"
            );
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        Ok(Self {
            origins,
            client: builder.build()?,
        })
    }

    pub fn validate(&self, value: &str, headers: &BTreeMap<String, String>) -> Result<()> {
        let url = parse_url(value)?;
        ensure!(
            self.origins.contains(&url.origin().ascii_serialization()),
            "transfer origin is not allowed"
        );
        let _ = request_headers(headers)?;
        Ok(())
    }

    pub async fn get(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        size: usize,
    ) -> Result<Option<Vec<u8>>> {
        self.validate(url, headers)?;
        ensure!(size <= MAX_BLOCK_BYTES, "wire block too large");
        let mut response = self
            .client
            .get(url)
            .headers(request_headers(headers)?)
            .header("Accept-Encoding", "identity")
            .send()
            .await
            .map_err(|e| e.without_url())?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status() != StatusCode::OK {
            return Err(HttpStatus(response.status().as_u16()).into());
        }
        if let Some(length) = response.content_length() {
            ensure!(length == size as u64, "wire length mismatch");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
            ensure!(
                chunk.len() <= size.saturating_sub(bytes.len()),
                "wire response exceeds declared length"
            );
            bytes.extend_from_slice(&chunk);
        }
        ensure!(bytes.len() == size, "incomplete wire block");
        Ok(Some(bytes))
    }

    pub async fn put(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        bytes: Vec<u8>,
    ) -> Result<()> {
        self.validate(url, headers)?;
        let response = self
            .client
            .put(url)
            .headers(request_headers(headers)?)
            .body(bytes)
            .send()
            .await
            .map_err(|e| e.without_url())?;
        if !response.status().is_success() && response.status() != StatusCode::PRECONDITION_FAILED {
            return Err(HttpStatus(response.status().as_u16()).into());
        }
        // Caller must read back and verify, including an already-present immutable object.
        Ok(())
    }
}

fn parse_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow::anyhow!("invalid transfer URL"))?;
    ensure!(
        url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
        "URL userinfo/fragment is forbidden"
    );
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "HTTPS required outside loopback"
    );
    Ok(url)
}

fn request_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap> {
    ensure!(headers.len() <= 16, "too many capability headers");
    let mut out = HeaderMap::new();
    for (key, value) in headers {
        if [
            "host",
            "content-length",
            "transfer-encoding",
            "connection",
            "proxy-authorization",
            "range",
            "accept-encoding",
        ]
        .contains(&key.to_ascii_lowercase().as_str())
        {
            bail!("forbidden capability header");
        }
        ensure!(value.len() <= 8192, "capability header too long");
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid capability header name"))?;
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| anyhow::anyhow!("invalid capability header value"))?;
        value.set_sensitive(true);
        out.insert(name, value);
    }
    Ok(out)
}
