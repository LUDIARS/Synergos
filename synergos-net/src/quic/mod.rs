//! QUIC セッション管理
//!
//! quinn を使った QUIC 接続の確立・管理・ストリーム制御を提供する。
//! サーバー（リスナー）とクライアント（コネクタ）の両方の役割を担う。
//!
//! ## S1 ピア真性認証
//!
//! Synergos の PeerId は `blake3(pubkey)[:20]` である。S1 対策として
//! QUIC 層では以下を強制している:
//!
//! 1. サーバ (着信側) は、ローカル `Identity` の ed25519 キーから rcgen で
//!    自己署名証明書を生成する。証明書の `subjectPublicKeyInfo` には
//!    ed25519 公開鍵がそのまま載るため、クライアントは SPKI の公開鍵を
//!    取り出して `blake3(pubkey)[:20] == expected_peer_id` を検証できる。
//! 2. クライアント (発信側) は `connect()` で期待する PeerId を明示的に
//!    渡す必要があり、内部で `PeerPinningVerifier` を差し込んだ
//!    `ClientConfig` を都度生成して `endpoint.connect_with()` する。
//! 3. ピアの公開鍵が PeerId に一致しない場合は TLS ハンドシェイク時点で
//!    拒否する。緩和パス (旧 `DevOnlySkipVerify`) は完全に削除した。

pub mod hello;
mod verifier;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::RwLock;

use crate::config::QuicConfig;
use crate::error::{Result, SynergosNetError};
use crate::identity::Identity;
use crate::types::{PeerId, TransferId};

pub use verifier::PeerPinningVerifier;

/// QUIC ストリーム種別
#[derive(Debug, Clone)]
pub enum StreamType {
    /// 制御メッセージ（Handshake, Ping/Pong, Bye）
    Control,
    /// ファイルデータ転送
    Data { transfer_id: TransferId },
}

/// QUIC コネクションの情報
#[derive(Debug)]
pub struct QuicConnection {
    /// 相手のピアID
    pub peer_id: PeerId,
    /// 接続先アドレス
    pub remote_addr: SocketAddr,
    /// アクティブストリーム数
    pub active_streams: u32,
    /// 接続状態
    pub state: QuicConnectionState,
    /// RTT（ミリ秒）
    pub rtt_ms: u32,
    /// quinn コネクションハンドル
    connection: Option<quinn::Connection>,
}

/// QUIC 接続状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuicConnectionState {
    /// 接続中
    Connecting,
    /// 接続済み
    Connected,
    /// 切断中
    Closing,
    /// 切断済み
    Closed,
}

/// スピードテスト結果
#[derive(Debug, Clone)]
pub struct SpeedTestResult {
    /// 往復遅延 (RTT) の中央値 (ms)
    pub rtt_median_ms: u32,
    /// RTT のジッター (ms)
    pub rtt_jitter_ms: u32,
    /// ダウンロード帯域 (bps)
    pub download_bps: u64,
    /// アップロード帯域 (bps)
    pub upload_bps: u64,
    /// 実効並列ストリーム容量
    pub effective_streams: u16,
}

/// コネクションパラメータの算出結果
#[derive(Debug, Clone)]
pub struct CalibratedParams {
    pub max_connections: u16,
    pub large_streams: u16,
    pub medium_streams: u16,
    pub small_streams: u16,
    pub chunk_size_multiplier: f64,
    pub initial_bandwidth_limit: u64,
}

/// コネクションキャリブレータ
pub struct ConnectionCalibrator;

impl ConnectionCalibrator {
    /// スピードテスト結果からパラメータを算出
    pub fn calibrate(result: &SpeedTestResult) -> CalibratedParams {
        let bandwidth = (result.download_bps).min(result.upload_bps);

        CalibratedParams {
            max_connections: match bandwidth {
                b if b < 10_000_000 => 1,
                b if b < 100_000_000 => 2,
                _ => 4,
            },
            large_streams: (result.effective_streams / 2).max(1),
            medium_streams: (result.effective_streams / 4).max(1),
            small_streams: (result.effective_streams / 8).max(1),
            chunk_size_multiplier: if result.rtt_median_ms > 100 {
                2.0
            } else if result.rtt_median_ms > 50 {
                1.5
            } else {
                1.0
            },
            initial_bandwidth_limit: (bandwidth as f64 * 0.8) as u64,
        }
    }
}

/// QUIC セッションマネージャ
///
/// quinn の Endpoint を管理し、ピアとの QUIC コネクションの確立・管理を行う。
/// サーバー（着信接続の受付）とクライアント（発信接続の確立）の両方を担当する。
pub struct QuicManager {
    config: QuicConfig,
    /// 自ノードの暗号アイデンティティ (S1 の真性認証で使う)
    identity: Arc<Identity>,
    /// アクティブなコネクション（PeerId → QuicConnection）
    connections: DashMap<PeerId, QuicConnection>,
    /// quinn エンドポイント（初期化後にセット）
    endpoint: RwLock<Option<quinn::Endpoint>>,
    /// ローカルバインドアドレス
    local_addr: RwLock<Option<SocketAddr>>,
    /// コネクションごとの帯域推定値
    bandwidth_estimates: DashMap<PeerId, u64>,
}

impl QuicManager {
    pub fn new(config: QuicConfig, identity: Arc<Identity>) -> Self {
        install_default_crypto_provider();
        Self {
            config,
            identity,
            connections: DashMap::new(),
            endpoint: RwLock::new(None),
            local_addr: RwLock::new(None),
            bandwidth_estimates: DashMap::new(),
        }
    }

    /// QUIC エンドポイントを初期化し、リスンを開始する。
    ///
    /// サーバ設定はローカル `Identity` の ed25519 キーから派生した自己署名
    /// 証明書を使う。クライアント接続側は S1 仕様で `expected_peer_id` を
    /// 必須としているため、ここでデフォルトの `ClientConfig` はセットしない。
    pub async fn bind(&self, addr: SocketAddr) -> Result<SocketAddr> {
        let server_config = self.build_server_config()?;
        let endpoint = quinn::Endpoint::server(server_config, addr)
            .map_err(|e| SynergosNetError::Quic(format!("Failed to bind: {}", e)))?;

        let actual_addr = endpoint
            .local_addr()
            .map_err(|e| SynergosNetError::Quic(format!("Failed to get local addr: {}", e)))?;

        tracing::info!("QUIC endpoint bound to {}", actual_addr);

        *self.local_addr.write().await = Some(actual_addr);
        *self.endpoint.write().await = Some(endpoint);

        Ok(actual_addr)
    }

    /// 指定アドレスに QUIC 接続を確立する。
    ///
    /// `expected_peer_id` は接続先ピアから期待する PeerId。サーバ側から
    /// 提示される自己署名証明書の ed25519 公開鍵が `blake3[:20]` でこれと
    /// 一致しなかった場合、TLS ハンドシェイクは失敗し `connect` は
    /// `SynergosNetError::Identity` を返す。
    pub async fn connect(
        &self,
        expected_peer_id: PeerId,
        addr: SocketAddr,
        server_name: &str,
    ) -> Result<()> {
        let endpoint = self.endpoint.read().await;
        let endpoint = endpoint
            .as_ref()
            .ok_or_else(|| SynergosNetError::Quic("Endpoint not initialized".into()))?;

        tracing::info!(
            "Connecting to peer {} at {}",
            expected_peer_id.short(),
            addr
        );

        let client_config = self.build_client_config_for(expected_peer_id.clone())?;

        let connection = endpoint
            .connect_with(client_config, addr, server_name)
            .map_err(|e| SynergosNetError::Quic(format!("Connect error: {}", e)))?
            .await
            .map_err(|e| {
                // TLS 真性認証失敗は SynergosNetError::Identity に寄せる
                let msg = format!("{e}");
                if msg.contains("PeerPinning") || msg.contains("UnknownIssuer") {
                    SynergosNetError::Identity(format!("peer auth failed: {msg}"))
                } else {
                    SynergosNetError::Quic(format!("Connection failed: {msg}"))
                }
            })?;

        let rtt = connection.rtt().as_millis() as u32;

        // 相互認識のため HLO1 ストリームで自分の署名 Hello を送る。
        // 失敗しても接続は閉じる (相手にこちらの PeerId を確信させられない)。
        if let Err(e) = hello::send_hello(&connection, &self.identity).await {
            connection.close(0u32.into(), b"hello failed");
            return Err(SynergosNetError::Identity(format!(
                "hello send failed: {e}"
            )));
        }

        let quic_conn = QuicConnection {
            peer_id: expected_peer_id.clone(),
            remote_addr: addr,
            active_streams: 0,
            state: QuicConnectionState::Connected,
            rtt_ms: rtt,
            connection: Some(connection),
        };

        self.connections.insert(expected_peer_id, quic_conn);
        Ok(())
    }

    /// ピアとの接続を切断する
    pub async fn disconnect(&self, peer_id: &PeerId, reason: &str) {
        if let Some(mut entry) = self.connections.get_mut(peer_id) {
            entry.state = QuicConnectionState::Closing;
            if let Some(conn) = entry.connection.as_ref() {
                conn.close(0u32.into(), reason.as_bytes());
            }
            entry.state = QuicConnectionState::Closed;
        }
        self.connections.remove(peer_id);
        self.bandwidth_estimates.remove(peer_id);
        tracing::info!("Disconnected from peer {}: {}", peer_id.short(), reason);
    }

    /// 指定ピアとの双方向ストリームを開く
    pub async fn open_stream(
        &self,
        peer_id: &PeerId,
        stream_type: StreamType,
    ) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        // entry を読み終えた直後にロックを解放してから get_mut を取ることで
        // DashMap の同一 shard 内再取得によるデッドロックを避ける。
        let conn = {
            let entry = self
                .connections
                .get(peer_id)
                .ok_or_else(|| SynergosNetError::PeerNotFound(peer_id.to_string()))?;
            entry
                .connection
                .as_ref()
                .ok_or_else(|| SynergosNetError::Quic("Connection not established".into()))?
                .clone()
        };

        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| SynergosNetError::Quic(format!("Failed to open stream: {}", e)))?;

        tracing::debug!("Opened {:?} stream to peer {}", stream_type, peer_id);

        if let Some(mut entry) = self.connections.get_mut(peer_id) {
            entry.active_streams += 1;
        }

        Ok((send, recv))
    }

    /// 指定ピアに制御データを送信する
    pub async fn send_control(&self, peer_id: &PeerId, data: &[u8]) -> Result<()> {
        let (mut send, _recv) = self.open_stream(peer_id, StreamType::Control).await?;

        send.write_all(data)
            .await
            .map_err(|e| SynergosNetError::Quic(format!("Send error: {}", e)))?;

        send.finish()
            .map_err(|e| SynergosNetError::Quic(format!("Finish error: {}", e)))?;

        Ok(())
    }

    /// コネクション情報を取得
    pub fn get_connection(&self, peer_id: &PeerId) -> Option<QuicConnectionInfo> {
        self.connections
            .get(peer_id)
            .map(|entry| QuicConnectionInfo {
                peer_id: entry.peer_id.clone(),
                remote_addr: entry.remote_addr,
                active_streams: entry.active_streams,
                state: entry.state.clone(),
                rtt_ms: entry.rtt_ms,
            })
    }

    /// 低レベルの `quinn::Connection` を取得する。テスト / 外部の accept_bi ループ
    /// 等で直接使う用。通常は `open_stream` を使うこと。
    pub fn raw_connection(&self, peer_id: &PeerId) -> Option<quinn::Connection> {
        self.connections
            .get(peer_id)
            .and_then(|e| e.connection.clone())
    }

    /// 全接続の情報を取得
    pub fn list_connections(&self) -> Vec<QuicConnectionInfo> {
        self.connections
            .iter()
            .map(|entry| QuicConnectionInfo {
                peer_id: entry.peer_id.clone(),
                remote_addr: entry.remote_addr,
                active_streams: entry.active_streams,
                state: entry.state.clone(),
                rtt_ms: entry.rtt_ms,
            })
            .collect()
    }

    /// 着信した接続を受け付ける (サーバ側のハンドシェイク完了を待つ)。
    /// S1 を相互 (サーバ側も相手の公開鍵から PeerId を再計算して検証する)
    /// 構造にするための土台。戻り値として相手から派生した PeerId を返す。
    pub async fn accept(&self) -> Result<Option<AcceptedConnection>> {
        let endpoint_guard = self.endpoint.read().await;
        let endpoint = match endpoint_guard.as_ref() {
            Some(ep) => ep.clone(),
            None => return Ok(None),
        };
        drop(endpoint_guard);

        let incoming = match endpoint.accept().await {
            Some(inc) => inc,
            None => return Ok(None),
        };

        let connection = incoming
            .await
            .map_err(|e| SynergosNetError::Quic(format!("Incoming handshake failed: {e}")))?;

        let remote_addr = connection.remote_address();
        let rtt = connection.rtt().as_millis() as u32;

        // クライアントからの HLO1 Hello を待って peer_id を確定させる。
        // タイムアウト / 不正 Hello ならコネクションを閉じて拒否する。
        let peer_id = match hello::recv_hello(&connection).await {
            Ok(pid) => pid,
            Err(e) => {
                connection.close(0u32.into(), b"hello rejected");
                return Err(e);
            }
        };

        let quic_conn = QuicConnection {
            peer_id: peer_id.clone(),
            remote_addr,
            active_streams: 0,
            state: QuicConnectionState::Connected,
            rtt_ms: rtt,
            connection: Some(connection.clone()),
        };
        self.connections.insert(peer_id.clone(), quic_conn);

        Ok(Some(AcceptedConnection {
            peer_id,
            remote_addr,
            connection,
        }))
    }

    /// 全接続を切断する
    pub async fn shutdown(&self) {
        let peer_ids: Vec<PeerId> = self.connections.iter().map(|e| e.key().clone()).collect();
        for peer_id in peer_ids {
            self.disconnect(&peer_id, "shutdown").await;
        }
        if let Some(endpoint) = self.endpoint.write().await.take() {
            endpoint.close(0u32.into(), b"shutdown");
        }
        tracing::info!("QUIC manager shut down");
    }

    /// RTT を更新（ピンポン計測結果）
    pub fn update_rtt(&self, peer_id: &PeerId, rtt_ms: u32) {
        if let Some(mut entry) = self.connections.get_mut(peer_id) {
            entry.rtt_ms = rtt_ms;
        }
    }

    /// 帯域推定値を更新
    pub fn update_bandwidth_estimate(&self, peer_id: &PeerId, bps: u64) {
        self.bandwidth_estimates.insert(peer_id.clone(), bps);
    }

    /// 帯域推定値を取得
    pub fn get_bandwidth_estimate(&self, peer_id: &PeerId) -> u64 {
        self.bandwidth_estimates
            .get(peer_id)
            .map(|e| *e.value())
            .unwrap_or(0)
    }

    /// 簡易スピードテスト（Ping × N → RTT 中央値を算出）
    pub async fn probe_rtt(&self, peer_id: &PeerId, count: u32) -> Result<u32> {
        let mut rtts = Vec::with_capacity(count as usize);

        for _ in 0..count {
            let start = std::time::Instant::now();
            self.send_control(peer_id, b"ping").await?;
            let elapsed = start.elapsed().as_millis() as u32;
            rtts.push(elapsed);
        }

        rtts.sort();
        let median = rtts[rtts.len() / 2];
        self.update_rtt(peer_id, median);

        Ok(median)
    }

    // ── 内部ヘルパー ──

    /// クライアント用 TLS 設定を構築。
    ///
    /// `expected_peer_id` をサーバ証明書の公開鍵派生 PeerId と照合する
    /// `PeerPinningVerifier` を差し込む。connect 1 回ごとに構築する。
    fn build_client_config_for(&self, expected_peer_id: PeerId) -> Result<quinn::ClientConfig> {
        let verifier = Arc::new(PeerPinningVerifier::new(expected_peer_id));

        let client_crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        let qcc = quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| SynergosNetError::Quic(format!("QUIC client crypto: {e}")))?;
        let mut cc = quinn::ClientConfig::new(Arc::new(qcc));

        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(self.config.max_concurrent_streams.into());
        transport.max_idle_timeout(Some(
            Duration::from_millis(self.config.idle_timeout_ms)
                .try_into()
                .unwrap_or_else(|_| quinn::IdleTimeout::from(quinn::VarInt::from_u32(30_000))),
        ));
        cc.transport_config(Arc::new(transport));
        Ok(cc)
    }

    fn build_server_config(&self) -> Result<quinn::ServerConfig> {
        let (cert_der, key_der) = build_server_cert(&self.identity)?;

        let server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .map_err(|e| SynergosNetError::Quic(format!("TLS config error: {}", e)))?;

        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(self.config.max_concurrent_streams.into());
        transport.max_idle_timeout(Some(
            Duration::from_millis(self.config.idle_timeout_ms)
                .try_into()
                .unwrap_or_else(|_| quinn::IdleTimeout::from(quinn::VarInt::from_u32(30_000))),
        ));

        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)
                .map_err(|e| SynergosNetError::Quic(format!("QUIC config error: {}", e)))?,
        ));
        server_config.transport_config(Arc::new(transport));

        Ok(server_config)
    }
}

/// rustls 0.23 は `ServerConfig::builder()` / `ClientConfig::builder()` が
/// 既定の `CryptoProvider` を要求する。`rustls` の feature `ring` 有効化と
/// `aws-lc-rs` 未有効の条件でのみ auto-detect が働くが、依存グラフの都合で
/// 両方の provider が見えることがあるので、プロセス起動時に明示的に
/// install する。`set_default` は 2 度目以降はエラーになるだけで副作用なし。
fn install_default_crypto_provider() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// accept 後に呼び出し側へ返す情報。相手 PeerId を証明書から再計算しておく。
pub struct AcceptedConnection {
    pub peer_id: PeerId,
    pub remote_addr: SocketAddr,
    pub connection: quinn::Connection,
}

/// rcgen で ed25519 keypair から自己署名証明書を作り、rustls が受け取れる
/// `(CertificateDer, PrivateKeyDer)` ペアを返す。
fn build_server_cert(
    identity: &Identity,
) -> Result<(
    rustls::pki_types::CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
)> {
    let pkcs8 = identity
        .to_pkcs8_der()
        .map_err(|e| SynergosNetError::Quic(format!("identity pkcs8 export: {e}")))?;

    let key_pair = rcgen::KeyPair::try_from(pkcs8.as_slice())
        .map_err(|e| SynergosNetError::Quic(format!("rcgen keypair: {e}")))?;

    let mut params =
        rcgen::CertificateParams::new(vec!["synergos".to_string(), identity.peer_id().to_string()])
            .map_err(|e| SynergosNetError::Quic(format!("rcgen params: {e}")))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, identity.peer_id().to_string());

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| SynergosNetError::Quic(format!("rcgen self-sign: {e}")))?;

    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der = rustls::pki_types::PrivateKeyDer::try_from(pkcs8)
        .map_err(|e| SynergosNetError::Quic(format!("rustls key der: {e}")))?;

    Ok((cert_der, key_der))
}

/// コネクション情報（外部公開用、quinn ハンドルなし）
#[derive(Debug, Clone)]
pub struct QuicConnectionInfo {
    pub peer_id: PeerId,
    pub remote_addr: SocketAddr,
    pub active_streams: u32,
    pub state: QuicConnectionState,
    pub rtt_ms: u32,
}
