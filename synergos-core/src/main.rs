//! synergos-core: Synergos 常駐デーモン
//!
//! クロスプラットフォーム対応のバックグラウンドデーモン。
//! EventBus + IPC サーバーを提供し、GUI / CLI / Ars Plugin からの
//! コマンドを受け付ける。

use clap::Parser;
use synergos_core::cli::{self, Cli};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ロギング初期化
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("synergos_core=info")),
        )
        .init();

    let cli = Cli::parse();
    cli::run(cli).await
}
