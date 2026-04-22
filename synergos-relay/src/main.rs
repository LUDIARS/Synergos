use std::path::PathBuf;

use clap::Parser;
use synergos_relay::{run, RelayConfig};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "synergos-relay", about = "Synergos WebSocket relay server")]
struct Args {
    /// 設定ファイルパス (TOML)。未指定なら既定値で起動。
    #[arg(long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("synergos_relay=info")),
        )
        .init();

    let args = Args::parse();
    let cfg = match args.config.as_deref() {
        Some(p) => RelayConfig::from_path(p)?,
        None => RelayConfig::default(),
    };

    let handle = run(cfg).await?;
    tracing::info!("relay ready on {}", handle.local_addr);

    // Ctrl+C で graceful shutdown
    tokio::signal::ctrl_c().await?;
    tracing::info!("SIGINT received; shutting down");
    handle.shutdown().await;
    Ok(())
}
