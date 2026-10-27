//! Explicit offline preparation and capability-only push/pull command surface.
use super::{BlockCache, TransferOptions, TransferPlan, DEFAULT_CHUNK_BYTES};
use anyhow::{ensure, Result};
use clap::{Args, Subcommand};
use std::{io::Read, path::PathBuf, sync::Arc};

#[derive(Args)]
pub struct ConnectionArgs {
    /// Independently trusted BLAKE3 root, not taken from the incoming transfer plan.
    #[arg(long)]
    root: String,
    /// Private persistent block cache. Only one command may own it at a time.
    #[arg(long)]
    cache: PathBuf,
    /// Explicit HTTP(S) origin; repeat for each authorized storage origin.
    #[arg(long = "origin", required = true)]
    origins: Vec<String>,
    /// PEM CA bundle provisioned by the environment/operator (for an HTTPS proxy).
    #[arg(long)]
    ca_bundle: Option<PathBuf>,
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    #[arg(long, default_value_t = 2)]
    retries: usize,
}

#[derive(Subcommand)]
pub enum ContentCommand {
    /// Hash/chunk a stable input file; emit a public manifest JSON, without network access.
    Prepare {
        source: PathBuf,
        #[arg(long)]
        cache: PathBuf,
        #[arg(long, default_value_t = DEFAULT_CHUNK_BYTES)]
        chunk_bytes: usize,
    },
    /// Read a short-lived transfer plan from stdin; upload cached blocks, root last.
    Push {
        #[command(flatten)]
        connection: ConnectionArgs,
    },
    /// Read a transfer plan from stdin; resume verified blocks and create a new file.
    Pull {
        destination: PathBuf,
        #[command(flatten)]
        connection: ConnectionArgs,
    },
}

pub async fn run(command: ContentCommand) -> Result<()> {
    match command {
        ContentCommand::Prepare {
            source,
            cache,
            chunk_bytes,
        } => {
            let cache = BlockCache::open(&cache)?;
            let manifest = super::prepare(&cache, &source, chunk_bytes).await?;
            println!("{}", serde_json::to_string(&manifest)?);
        }
        ContentCommand::Push { connection } => {
            let plan = read_plan()?;
            let cache = Arc::new(BlockCache::open(&connection.cache)?);
            let report = super::push(cache, plan, &connection.root, options(&connection)).await?;
            println!("{}", serde_json::to_string(&report)?);
        }
        ContentCommand::Pull {
            destination,
            connection,
        } => {
            let plan = read_plan()?;
            let cache = Arc::new(BlockCache::open(&connection.cache)?);
            let report = super::pull(
                cache,
                plan,
                &connection.root,
                &destination,
                options(&connection),
            )
            .await?;
            println!("{}", serde_json::to_string(&report)?);
        }
    }
    Ok(())
}

fn options(args: &ConnectionArgs) -> TransferOptions {
    TransferOptions {
        allowed_origins: args.origins.clone(),
        ca_bundle: args.ca_bundle.clone(),
        concurrency: args.concurrency,
        retries: args.retries,
    }
}

fn read_plan() -> Result<TransferPlan> {
    const LIMIT: u64 = 64 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "transfer plan exceeds 64 MiB");
    // Serde errors can quote unknown keys supplied by a caller; never echo a capability.
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid transfer plan JSON"))
}
