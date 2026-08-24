mod api;
mod auth;
mod cloudflare;
mod config;
mod error;
mod reconcile;
mod store;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing::info;

use crate::api::AppState;
use crate::auth::AdminToken;
use crate::cloudflare::CloudflareClient;
use crate::config::ControlConfig;
use crate::store::JsonStore;

#[derive(Parser)]
#[command(
    name = "synergos-control",
    version,
    about = "Synergos 管制サーバー — 組織別ノードレジストリ + Cloudflare Mesh 自動化"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 管制サーバーを起動する
    Serve {
        /// 設定ファイル (TOML)
        #[arg(long)]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "synergos_control=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve { config } => serve(config).await,
    }
}

async fn serve(config_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = ControlConfig::load(&config_path)?;
    // 必須秘密情報は起動時に解決。欠けていれば即終了 (fail-fast)
    let secrets = config.resolve_secrets()?;

    let store = JsonStore::open(config.store_path.clone())?;
    let cloudflare = CloudflareClient::new(
        config.cloudflare.api_base.clone(),
        config.cloudflare.account_id.clone(),
        secrets.cloudflare_api_token,
    )?;

    let state = Arc::new(AppState {
        store,
        cloudflare,
        cf_api_base: config.cloudflare.api_base.clone(),
        cf_account_id: config.cloudflare.account_id.clone(),
        ui_dist: config.ui.dist_path.clone(),
    });
    let router = api::build_router(state, AdminToken(secrets.admin_token));

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(addr = %config.bind_addr, "synergos-control listening");
    match &config.ui.dist_path {
        Some(_) => info!("admin UI served at /ui/"),
        None => info!("admin UI is not configured ([ui] dist_path); /ui/ returns 503"),
    }

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    // Ctrl+C / SIGTERM で graceful shutdown
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received");
}
