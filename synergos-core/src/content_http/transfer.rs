//! Bounded parallel transfer of a verified closure, independent of QUIC and peer discovery.
use super::{
    assembly, codec, http::HttpClient, prepare::wire_digest, BlockCache, BlockDescription,
    TransferGrant, TransferPlan,
};
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use synergos_net::{content::Block, types::Cid};

pub struct TransferOptions {
    pub allowed_origins: Vec<String>,
    /// Operator-provided proxy CA bundle; TLS verification always remains enabled.
    pub ca_bundle: Option<PathBuf>,
    pub concurrency: usize,
    /// Additional attempts on transport failures and transient HTTP statuses.
    pub retries: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct TransferReport {
    pub transferred_blocks: usize,
    pub reused_blocks: usize,
    /// Completed payload bytes only; excludes headers and failed/partial attempts.
    pub wire_bytes: u64,
}

struct Session {
    cache: Arc<BlockCache>,
    http: HttpClient,
    grants: HashMap<String, TransferGrant>,
    retries: usize,
}

impl Session {
    fn new(
        cache: Arc<BlockCache>,
        plan: &TransferPlan,
        root: &str,
        options: &TransferOptions,
    ) -> Result<Arc<Self>> {
        plan.manifest.validate(root)?;
        ensure!(
            (1..=16).contains(&options.concurrency) && options.retries <= 5,
            "invalid transfer concurrency/retries"
        );
        ensure!(
            plan.grants.len() <= plan.manifest.blocks.len(),
            "too many grants"
        );
        let http = HttpClient::new(&options.allowed_origins, options.ca_bundle.as_deref())?;
        let mut grants = HashMap::new();
        let descriptions: HashSet<_> = plan
            .manifest
            .blocks
            .iter()
            .map(|b| b.cid.as_str())
            .collect();
        for grant in &plan.grants {
            super::validate_cid(&grant.cid)?;
            http.validate(&grant.get_url, &grant.get_headers)?;
            if let Some(url) = &grant.put_url {
                http.validate(url, &grant.put_headers)?;
            }
            ensure!(
                descriptions.contains(grant.cid.as_str()),
                "grant outside manifest"
            );
            ensure!(
                grants.insert(grant.cid.clone(), grant.clone()).is_none(),
                "duplicate grant"
            );
        }
        Ok(Arc::new(Self {
            cache,
            http,
            grants,
            retries: options.retries,
        }))
    }

    async fn fetch(&self, desc: &BlockDescription, grant: &TransferGrant) -> Result<Option<Block>> {
        let Some(wire) = self
            .http
            .get(&grant.get_url, &grant.get_headers, desc.wire_bytes)
            .await?
        else {
            return Ok(None);
        };
        ensure!(
            wire_digest(&wire) == desc.wire_sha256,
            "wire digest mismatch"
        );
        let block = Block {
            cid: Cid(desc.cid.clone()),
            bytes: codec::decode(&wire, desc.encoding, desc.decoded_bytes)?,
        };
        ensure!(block.verify(), "decoded block CID mismatch");
        Ok(Some(block))
    }

    async fn one(&self, desc: &BlockDescription, upload: bool) -> Result<(bool, u64)> {
        if !upload && self.cache.load(&desc.cid).await?.is_some() {
            return Ok((false, 0));
        }
        let grant = self
            .grants
            .get(&desc.cid)
            .context("missing block capability; renew the transfer plan")?;
        if let Some(block) = self.fetch(desc, grant).await? {
            if !upload {
                self.cache.save(block).await?;
            }
            return Ok((!upload, desc.wire_bytes as u64));
        }
        ensure!(upload, "remote block is missing");
        let block = self
            .cache
            .load(&desc.cid)
            .await?
            .context("upload cache block missing")?;
        ensure!(
            block.bytes.len() == desc.decoded_bytes,
            "upload block size mismatch"
        );
        let wire = codec::encode(&block.bytes, desc.encoding)?;
        ensure!(
            wire.len() == desc.wire_bytes && wire_digest(&wire) == desc.wire_sha256,
            "prepared wire encoding changed"
        );
        let url = grant
            .put_url
            .as_ref()
            .context("upload capability missing")?;
        self.http.put(url, &grant.put_headers, wire).await?;
        self.fetch(desc, grant)
            .await?
            .context("uploaded block is not readable")?;
        Ok((true, 2 * desc.wire_bytes as u64))
    }

    async fn retry_one(&self, desc: &BlockDescription, upload: bool) -> Result<(bool, u64)> {
        for attempt in 0..=self.retries {
            match self.one(desc, upload).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let transient = e
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.is_timeout() || e.is_connect() || e.is_body())
                        || e.downcast_ref::<super::http::HttpStatus>()
                            .is_some_and(|status| {
                                matches!(status.0, 408 | 429 | 500 | 502 | 503 | 504)
                            });
                    if !transient || attempt == self.retries {
                        return Err(e);
                    }
                    tokio::time::sleep(Duration::from_millis(200 * (1u64 << attempt))).await;
                }
            }
        }
        unreachable!("bounded retry loop returns on its last attempt")
    }
}

fn account(report: &mut TransferReport, result: (bool, u64)) {
    if result.0 {
        report.transferred_blocks += 1;
    } else {
        report.reused_blocks += 1;
    }
    report.wire_bytes += result.1;
}

async fn parallel(
    session: Arc<Session>,
    blocks: Vec<BlockDescription>,
    concurrency: usize,
    upload: bool,
    report: &mut TransferReport,
) -> Result<()> {
    let mut pending = tokio::task::JoinSet::new();
    let mut blocks = blocks.into_iter();
    loop {
        while pending.len() < concurrency {
            let Some(block) = blocks.next() else { break };
            let session = session.clone();
            pending.spawn(async move { session.retry_one(&block, upload).await });
        }
        let Some(result) = pending.join_next().await else {
            break;
        };
        match result {
            Ok(Ok(value)) => account(report, value),
            other => {
                pending.abort_all();
                while pending.join_next().await.is_some() {}
                return match other {
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(error.into()),
                    Ok(Ok(_)) => unreachable!(),
                };
            }
        }
    }
    Ok(())
}

pub async fn pull(
    cache: Arc<BlockCache>,
    plan: TransferPlan,
    expected_root: &str,
    destination: &Path,
    options: TransferOptions,
) -> Result<TransferReport> {
    ensure!(
        tokio::fs::symlink_metadata(destination)
            .await
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "destination already exists or cannot be inspected"
    );
    let session = Session::new(cache.clone(), &plan, expected_root, &options)?;
    let mut report = TransferReport::default();
    let root = plan
        .manifest
        .blocks
        .iter()
        .find(|b| b.cid == expected_root)
        .context("root missing")?;
    account(&mut report, session.retry_one(root, false).await?);
    let dag = assembly::validated_dag(&cache, &plan.manifest).await?;
    let children = plan
        .manifest
        .blocks
        .into_iter()
        .filter(|b| b.cid != expected_root)
        .collect();
    parallel(session, children, options.concurrency, false, &mut report).await?;
    assembly::assemble(&cache, &dag, destination).await?;
    Ok(report)
}

pub async fn push(
    cache: Arc<BlockCache>,
    plan: TransferPlan,
    expected_root: &str,
    options: TransferOptions,
) -> Result<TransferReport> {
    let session = Session::new(cache.clone(), &plan, expected_root, &options)?;
    let _ = assembly::validated_dag(&cache, &plan.manifest).await?;
    for block in &plan.manifest.blocks {
        ensure!(
            session
                .grants
                .get(&block.cid)
                .is_some_and(|g| g.put_url.is_some()),
            "upload requires read and write capabilities for every block"
        );
        ensure!(
            cache.load(&block.cid).await?.is_some(),
            "upload cache is incomplete"
        );
    }
    let mut report = TransferReport::default();
    let root = plan
        .manifest
        .blocks
        .iter()
        .find(|b| b.cid == expected_root)
        .context("root missing")?
        .clone();
    let children = plan
        .manifest
        .blocks
        .into_iter()
        .filter(|b| b.cid != expected_root)
        .collect();
    parallel(
        session.clone(),
        children,
        options.concurrency,
        true,
        &mut report,
    )
    .await?;
    // Publish the root only after every referenced block is readable and verified.
    account(&mut report, session.retry_one(&root, true).await?);
    Ok(report)
}
