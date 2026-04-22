//! WebSocket Relay クライアント (Conduit の `RouteKind::Relay` 実装)。
//!
//! シナリオ: 両ノードが NAT 越えで直接接続できず Cloudflare Tunnel も
//! 使えない場合のフォールバック。信頼できる中継サーバに WSS で繋ぎ、
//! `room_id` 毎のブロードキャストで相手ピアに data frame を流す。
//!
//! ## プロトコル
//!
//! サーバ実装は別リポジトリ想定 (`synergos-relay` バイナリ) だが、
//! クライアント ⇔ サーバの wire format は本モジュールで規定する:
//!
//! クライアント → サーバ (最初の 1 メッセージで join):
//! ```json
//! { "type": "join", "room_id": "<uuid>", "peer_id": "<local peer>" }
//! ```
//!
//! その後の data 送信 / 受信は binary WebSocket フレームを使う。
//! 先頭に `u16 be length` + msgpack `(from_peer_id, payload)` を書く。
//!
//! 本実装ではサーバ側は提供せず、クライアントだけ。テストは in-process
//! WebSocket サーバを立ててラウンドトリップを検証する。

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{Result, SynergosNetError};
use crate::types::PeerId;

/// Room join リクエスト (text frame)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayControl {
    Join { room_id: String, peer_id: PeerId },
    Leave,
}

/// relay 経由で送る data frame。from_peer_id は binary payload の前に
/// msgpack で 1 shot 直列化する。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayData {
    pub from_peer_id: PeerId,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}

/// Relay session 1 本の suspend/run。`incoming` から相手ピアの
/// data を受け取り、`send` で自分から相手へ流す。
pub struct RelaySession {
    /// 外部から binary data を送るキュー
    send_tx: mpsc::Sender<RelayData>,
    /// 外部が受信する binary data のキュー
    recv_rx: Arc<Mutex<mpsc::Receiver<RelayData>>>,
    /// run_loop のハンドル (session shutdown で abort)
    task: tokio::task::JoinHandle<()>,
}

impl RelaySession {
    /// 指定 WSS URL に接続し、room に join する。
    /// 成功すると双方向のチャネルを生やした `RelaySession` を返す。
    pub async fn connect(url: &str, room_id: &str, local_peer: PeerId) -> Result<Self> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("relay connect: {e}")))?;

        let (mut writer, mut reader) = ws_stream.split();

        // Join 制御メッセージを最初に送る
        let join = RelayControl::Join {
            room_id: room_id.to_string(),
            peer_id: local_peer.clone(),
        };
        let text = serde_json::to_string(&join)
            .map_err(|e| SynergosNetError::Serialization(format!("relay join: {e}")))?;
        writer
            .send(Message::text(text))
            .await
            .map_err(|e| SynergosNetError::Quic(format!("relay join send: {e}")))?;

        let (send_tx, mut send_rx) = mpsc::channel::<RelayData>(64);
        let (recv_tx, recv_rx) = mpsc::channel::<RelayData>(64);

        // writer 側: send_rx を読んでバイナリフレームで送信
        let writer_task = tokio::spawn(async move {
            while let Some(data) = send_rx.recv().await {
                let body = match rmp_serde::to_vec(&data) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!("relay encode: {e}");
                        continue;
                    }
                };
                if let Err(e) = writer.send(Message::binary(body)).await {
                    tracing::warn!("relay send: {e}");
                    break;
                }
            }
        });

        // reader 側: WS から受信した binary フレームを decode して recv_tx に流す
        let reader_task = tokio::spawn(async move {
            while let Some(msg) = reader.next().await {
                let Ok(msg) = msg else {
                    break;
                };
                match msg {
                    Message::Binary(bytes) => match rmp_serde::from_slice::<RelayData>(&bytes) {
                        Ok(data) => {
                            if recv_tx.send(data).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => tracing::debug!("relay decode: {e}"),
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        // supervisor: 片方が終わったらもう片方も止める
        let task = tokio::spawn(async move {
            let _ = tokio::try_join!(writer_task, reader_task);
        });

        Ok(Self {
            send_tx,
            recv_rx: Arc::new(Mutex::new(recv_rx)),
            task,
        })
    }

    /// `target_peer_id` は使わない (relay サーバが room 内に bcast する前提)
    /// — 実運用ではサーバ側で to フィールドを見てフィルタするが、本最小版は
    /// bcast + 受信側がフィルタするモデル。
    pub async fn send(&self, data: RelayData) -> Result<()> {
        self.send_tx
            .send(data)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("relay send queue: {e}")))
    }

    /// タイムアウト付きで 1 メッセージを受信する。
    pub async fn recv(&self, timeout: Duration) -> Result<RelayData> {
        let mut rx = self.recv_rx.lock().await;
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Some(d)) => Ok(d),
            Ok(None) => Err(SynergosNetError::Quic("relay recv closed".into())),
            Err(_) => Err(SynergosNetError::Quic("relay recv timeout".into())),
        }
    }

    pub fn shutdown(&self) {
        self.task.abort();
    }
}

impl Drop for RelaySession {
    fn drop(&mut self) {
        self.task.abort();
    }
}
