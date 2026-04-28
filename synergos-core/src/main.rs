//! synergos-core: Synergos 常駐デーモン
//!
//! クロスプラットフォーム対応のバックグラウンドデーモン。
//! EventBus + IPC サーバーを提供し、GUI / CLI / Ars Plugin からの
//! コマンドを受け付ける。
//!
//! ## ロギング書式
//!
//! `SYNERGOS_LOG_FORMAT` 環境変数で出力形式を切り替える:
//! - 未指定 / `"text"` / `"pretty"` → 既定のヒューマンリーダブル fmt (開発時)
//! - `"json"` → 1 行 1 イベントの JSON 構造化ログ (production / log 集約用)
//!
//! ログレベルは従来通り `RUST_LOG` で制御 (例: `RUST_LOG=synergos_core=debug`)。

use clap::Parser;
use synergos_core::cli::{self, Cli};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();

    let cli = Cli::parse();
    cli::run(cli).await
}

fn init_logging() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("synergos_core=info"));

    let format = std::env::var("SYNERGOS_LOG_FORMAT")
        .unwrap_or_default()
        .to_ascii_lowercase();

    match format.as_str() {
        "json" => {
            tracing_subscriber::fmt()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .with_env_filter(env_filter)
                .init();
        }
        // 既定値 (空文字 / "text" / "pretty" / その他) はヒューマンリーダブル
        _ => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }
}
