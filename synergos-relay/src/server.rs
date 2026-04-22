//! Relay server 本体。TCP/WS accept → join → room bcast の最小実装。

use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

use crate::config::RelayConfig;

/// クライアントからの制御メッセージ (text frame)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientControl {
    Join {
        room_id: String,
        /// クライアント自己申告の peer_id (ログ / デバッグ用のみ、信用しない)
        #[serde(default)]
        peer_id: Option<serde_json::Value>,
    },
    Leave,
}

/// 1 クライアントに流し込む binary frame の送信側。
#[derive(Clone)]
struct ClientSender {
    id: u64,
    tx: mpsc::Sender<Vec<u8>>,
}

/// 全体の room 管理。
struct Rooms {
    cfg: RelayConfig,
    /// room_id → 登録クライアント
    rooms: DashMap<String, Vec<ClientSender>>,
    /// 内部 id 採番 (broadcast 時に sender 自身を除外する用)
    next_id: std::sync::atomic::AtomicU64,
}

impl Rooms {
    fn new(cfg: RelayConfig) -> Arc<Self> {
        Arc::new(Self {
            cfg,
            rooms: DashMap::new(),
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    fn alloc_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn join(&self, room_id: String, sender: ClientSender) -> Result<(), String> {
        if self.rooms.len() >= self.cfg.max_rooms && !self.rooms.contains_key(&room_id) {
            return Err(format!("max_rooms {} reached", self.cfg.max_rooms));
        }
        let mut entry = self.rooms.entry(room_id.clone()).or_default();
        if entry.len() >= self.cfg.max_clients_per_room {
            return Err(format!("room {room_id} full ({} clients)", entry.len()));
        }
        entry.push(sender);
        Ok(())
    }

    fn leave(&self, room_id: &str, client_id: u64) {
        if let Some(mut entry) = self.rooms.get_mut(room_id) {
            entry.retain(|c| c.id != client_id);
        }
        // 空になった room は即除去 (max_rooms 圧迫を防ぐ)
        self.rooms.retain(|_, clients| !clients.is_empty());
    }

    /// room 内の他クライアントにバイナリ frame を配る (送信者自身は除く)。
    fn broadcast(&self, room_id: &str, from_id: u64, payload: &[u8]) {
        if let Some(entry) = self.rooms.get(room_id) {
            for client in entry.iter() {
                if client.id != from_id {
                    // backpressure: mpsc::try_send で落ちたらそのクライアントを切り捨てる
                    let _ = client.tx.try_send(payload.to_vec());
                }
            }
        }
    }
}

/// サーバ制御ハンドル。shutdown 可能。
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    shutdown_tx: broadcast::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.task.await;
    }
}

/// 指定 config で accept ループを起動し、`ServerHandle` を返す。
pub async fn run(cfg: RelayConfig) -> anyhow::Result<ServerHandle> {
    let listener = TcpListener::bind(cfg.listen_addr).await?;
    let local_addr = listener.local_addr()?;
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let shutdown_sub = shutdown_tx.clone();
    let rooms = Rooms::new(cfg);

    tracing::info!("relay server listening on {local_addr}");

    let task = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_sub.subscribe();
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!("relay server shutting down");
                    break;
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let rooms = rooms.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, peer, rooms).await {
                                    tracing::debug!("client {peer} ended: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("accept error: {e}");
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
    });

    Ok(ServerHandle {
        local_addr,
        shutdown_tx,
        task,
    })
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    rooms: Arc<Rooms>,
) -> anyhow::Result<()> {
    let ws = accept_async(stream).await?;
    let (mut ws_out, mut ws_in) = ws.split();

    let client_id = rooms.alloc_id();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<Vec<u8>>(64);

    // writer task: outbound_rx に流れてきた bytes を相手に送る
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            if ws_out.send(Message::binary(bytes)).await.is_err() {
                break;
            }
        }
        let _ = ws_out.close().await;
    });

    let mut current_room: Option<String> = None;

    while let Some(msg) = ws_in.next().await {
        let msg = msg?;
        match msg {
            Message::Text(txt) => {
                // join / leave 制御
                let ctrl: ClientControl = match serde_json::from_str(&txt) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::debug!("client {peer} bad control: {e}");
                        continue;
                    }
                };
                match ctrl {
                    ClientControl::Join { room_id, .. } => {
                        if current_room.is_some() {
                            tracing::debug!("client {peer} sent join twice");
                            continue;
                        }
                        let sender = ClientSender {
                            id: client_id,
                            tx: outbound_tx.clone(),
                        };
                        if let Err(reason) = rooms.join(room_id.clone(), sender) {
                            tracing::warn!("join rejected for {peer}: {reason}");
                            break;
                        }
                        current_room = Some(room_id);
                    }
                    ClientControl::Leave => {
                        if let Some(room) = current_room.take() {
                            rooms.leave(&room, client_id);
                        }
                    }
                }
            }
            Message::Binary(bytes) => {
                if let Some(room) = &current_room {
                    rooms.broadcast(room, client_id, &bytes);
                } else {
                    tracing::debug!("client {peer} sent binary before join");
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // cleanup
    if let Some(room) = current_room {
        rooms.leave(&room, client_id);
    }
    drop(outbound_tx);
    let _ = writer_task.await;
    Ok(())
}
