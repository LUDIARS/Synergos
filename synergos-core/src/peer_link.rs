//! QUIC 接続 1 本ごとの生存期間を daemon 側で扱う。
//!
//! 2026-07-12 の運用テストで特定した通信 blocker のうち、接続の生存期間に
//! 関わるものをここで扱う (`review/2026-07-12/communication-failure-review.md`)。
//!
//! - B3: 発信 (`connect`) で張った接続にも `accept_bi` ループを張る。
//!   従来は着信接続にしか張っておらず、相手発の gossip / 転送が滞留していた。
//! - B4: 着信・発信どちらの接続でも相手を Presence に Connected として載せる。
//! - B5: 接続が閉じたら QuicManager のエントリを外して Disconnected に落とし、
//!   再接続先 URL が分かっているピアへ定期的に張り直す。
//!   (keep-alive 自体は QuicManager のトランスポート設定で送る)

use std::sync::Arc;
use std::time::Duration;

use synergos_net::quic::AcceptedConnection;
use synergos_net::types::PeerId;
use tokio::sync::broadcast;

use crate::daemon::{dispatch_peer_streams, NetworkHandles};
use crate::ipc_server::ServiceContext;

/// 再接続ループの確認間隔。
const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);
/// 再接続 1 回あたりの `/peer-info` 取得 + QUIC 接続のタイムアウト。
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// リモート由来の切断理由をログ / IPC イベントへ渡す最大文字数。
const MAX_CLOSE_REASON_CHARS: usize = 256;

/// 発信接続の確立通知を購読し、各接続を [`spawn_peer_connection`] に渡す。
/// 購読は呼び出し時点で同期的に行うので、この関数を `connect` するタスクより
/// 先に呼べば取りこぼさない。
pub(crate) fn spawn_outbound_dispatch(
    net: Arc<NetworkHandles>,
    ctx: Arc<ServiceContext>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    let mut outbound_rx = net.quic.subscribe_outbound();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                received = outbound_rx.recv() => match received {
                    Ok(conn) => spawn_peer_connection(net.clone(), ctx.clone(), conn),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            "outbound connection notifications lagged by {n}; \
                             those connections will not receive peer-initiated streams"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    })
}

/// 1 本の QUIC 接続について、相手を Presence に載せ、相手発ストリームを
/// 振り分け、閉じたら後始末する。着信・発信の両方から呼ぶ。
pub(crate) fn spawn_peer_connection(
    net: Arc<NetworkHandles>,
    ctx: Arc<ServiceContext>,
    conn: AcceptedConnection,
) {
    tokio::spawn(async move {
        let peer = conn.peer_id;
        let connection = conn.connection;
        let stable_id = connection.stable_id();

        ctx.presence
            .observe_connected(&peer, open_project_ids(&ctx));

        dispatch_peer_streams(
            connection.clone(),
            peer.clone(),
            net.dht.clone(),
            net.gossip.clone(),
            ctx.exchange.clone(),
            ctx.content_store.clone(),
            ctx.event_bus.clone(),
        )
        .await;

        let reason = connection
            .close_reason()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "stream loop ended".to_string());
        let reason = sanitize_close_reason(&reason);
        // 同じピアへ張り直した新しい接続が既にあるなら、そちらを生かす
        if net.quic.remove_closed(&peer, stable_id) {
            ctx.presence.observe_disconnected(&peer, &reason);
        }
    });
}

/// QUIC application close reason は相手が指定できるため、ログを改行できない形へ
/// escape し、イベント配送量も制限する。
fn sanitize_close_reason(reason: &str) -> String {
    reason
        .chars()
        .take(MAX_CLOSE_REASON_CHARS)
        .flat_map(char::escape_default)
        .collect()
}

/// 切断されたピアのうち、`/peer-info` URL が分かっているものへ定期的に
/// 張り直す。成功した接続は発信通知経由で [`spawn_peer_connection`] に乗る。
pub(crate) fn spawn_reconnect_loop(
    ctx: Arc<ServiceContext>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RECONNECT_INTERVAL);
        // 起動直後は bootstrap_urls の初回接続と競合させない
        interval.tick().await;
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = interval.tick() => {
                    for (peer, url) in reconnect_candidates(&ctx) {
                        reconnect_one(&ctx, &peer, &url).await;
                    }
                }
            }
        }
    })
}

/// 再接続が必要なピア (URL を知っていて、QUIC 接続が無いもの)。
fn reconnect_candidates(ctx: &ServiceContext) -> Vec<(PeerId, String)> {
    ctx.presence
        .reconnect_targets()
        .into_iter()
        .filter(|(peer, _)| ctx.quic.get_connection(peer).is_none())
        .collect()
}

async fn reconnect_one(ctx: &ServiceContext, peer: &PeerId, url: &str) {
    match crate::peer_bootstrap::bootstrap_from_url_expected(
        url,
        &ctx.quic,
        RECONNECT_TIMEOUT,
        Some(peer),
    )
    .await
    {
        Ok(_) => tracing::info!("reconnected to {}", peer.short()),
        // URL は private endpoint や秘密を含む path の可能性があるため、定期再試行の
        // ログには出さない。詳細は最初の明示的な bootstrap 応答で返される。
        Err(_) => tracing::debug!("reconnect to {} failed", peer.short()),
    }
}

/// 自ノードで open 中の全プロジェクト。接続してきた相手はこれらの相手として
/// 扱う (bootstrap_urls と同じ規則)。
pub(crate) fn open_project_ids(ctx: &ServiceContext) -> Vec<String> {
    crate::project::ProjectConfiguration::list_projects(&*ctx.project_manager)
        .into_iter()
        .map(|info| info.project_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_reason_is_bounded_and_cannot_inject_log_lines() {
        let sanitized = sanitize_close_reason(&format!("bad\r\n{}", "x".repeat(300)));
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\n'));
        assert!(sanitized.starts_with("bad\\r\\n"));
        assert!(sanitized.len() <= MAX_CLOSE_REASON_CHARS + 8);
    }
}
