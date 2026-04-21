//! QUIC セッション管理
//!
//! quinn を使った QUIC 接続の確立・管理・ストリーム制御を提供する。
//! サーバー（リスナー）とクライアント（コネクタ）の両方の役割を担う。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::RwLock;

use crate::config::QuicConfig;
use crate::error::{Result, SynergosNetError};
use crate::types::{PeerId, TransferId};

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
    pub fn new(config: QuicConfig) -> Self {
        Self {
            config,
            connections: DashMap::new(),
            endpoint: RwLock::new(None),
            local_addr: RwLock::new(None),
            bandwidth_estimates: DashMap::new(),
        }
    }

    /// QUIC エンドポイントを初期化し、リスンを開始する。
    ///
    /// サーバ設定に加え、クライアント既定設定も同時に差し込むので
    /// 以降 `connect()` は同 endpoint で使える (これをしないと `connect()`
    /// は "no default client config" エラーで必ず失敗する既知バグがあった)。
    pub async fn bind(&self, addr: SocketAddr) -> Result<SocketAddr> {
        let server_config = self.build_server_config()?;
        let mut endpoint = quinn::Endpoint::server(server_config, addr)
            .map_err(|e| SynergosNetError::Quic(format!("Failed to bind: {}", e)))?;

        // Phase D で導入した dev 向け ClientConfig を差し込む。
        // S1 の最終対策 (ピア公開鍵派生 cert + カスタム CertVerifier) は
        // 後続作業で build_client_config をハード化することで置き換える。
        match self.build_client_config() {
            Ok(cc) => endpoint.set_default_client_config(cc),
            Err(e) => tracing::warn!("QUIC client config init failed: {e}"),
        }

        let actual_addr = endpoint
            .local_addr()
            .map_err(|e| SynergosNetError::Quic(format!("Failed to get local addr: {}", e)))?;

        tracing::info!("QUIC endpoint bound to {}", actual_addr);

        *self.local_addr.write().await = Some(actual_addr);
        *self.endpoint.write().await = Some(endpoint);

        Ok(actual_addr)
    }

    /// 指定アドレスに QUIC 接続を確立する
    pub async fn connect(
        &self,
        peer_id: PeerId,
        addr: SocketAddr,
        server_name: &str,
    ) -> Result<()> {
        let endpoint = self.endpoint.read().await;
        let endpoint = endpoint
            .as_ref()
            .ok_or_else(|| SynergosNetError::Quic("Endpoint not initialized".into()))?;

        tracing::info!("Connecting to peer {} at {}", peer_id, addr);

        let connection = endpoint
            .connect(addr, server_name)
            .map_err(|e| SynergosNetError::Quic(format!("Connect error: {}", e)))?
            .await
            .map_err(|e| SynergosNetError::Quic(format!("Connection failed: {}", e)))?;

        let rtt = connection.rtt().as_millis() as u32;

        let quic_conn = QuicConnection {
            peer_id: peer_id.clone(),
            remote_addr: addr,
            active_streams: 0,
            state: QuicConnectionState::Connected,
            rtt_ms: rtt,
            connection: Some(connection),
        };

        self.connections.insert(peer_id, quic_conn);
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
        tracing::info!("Disconnected from peer {}: {}", peer_id, reason);
    }

    /// 指定ピアとの双方向ストリームを開く
    pub async fn open_stream(
        &self,
        peer_id: &PeerId,
        stream_type: StreamType,
    ) -> Result<(quinn::SendStream, quinn::RecvStream)> {
        let entry = self
            .connections
            .get(peer_id)
            .ok_or_else(|| SynergosNetError::PeerNotFound(peer_id.to_string()))?;

        let conn = entry
            .connection
            .as_ref()
            .ok_or_else(|| SynergosNetError::Quic("Connection not established".into()))?;

        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| SynergosNetError::Quic(format!("Failed to open stream: {}", e)))?;

        tracing::debug!("Opened {:?} stream to peer {}", stream_type, peer_id);

        // ストリーム数を更新
        drop(entry);
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
            // 小さいデータでラウンドトリップ計測
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

    /// サーバー用 TLS 設定を構築（自己署名証明書）
    /// 開発時の簡易 ClientConfig。**本物のピア認証ではない**。
    ///
    /// TODO(S1): `identity::Identity` の公開鍵を cert に埋め、ピア側で
    /// (1) cert から公開鍵を取り出し、(2) hash(pubkey) == PeerId を検証、
    /// (3) その公開鍵を後続の署名検証に転用する CertVerifier を実装する。
    /// 現状は dev/テスト用に `SkipServerVerification` を採用し、`connect()`
    /// を「そもそも通る」ようにするに留める。リリースビルドでは使わせない。
    fn build_client_config(&self) -> Result<quinn::ClientConfig> {
        #[derive(Debug)]
        struct DevOnlySkipVerify;
        impl rustls::client::danger::ServerCertVerifier for DevOnlySkipVerify {
            fn verify_server_cert(
                &self,
                _end_entity: &rustls::pki_types::CertificateDer<'_>,
                _intermediates: &[rustls::pki_types::CertificateDer<'_>],
                _server_name: &rustls::pki_types::ServerName<'_>,
                _ocsp: &[u8],
                _now: rustls::pki_types::UnixTime,
            ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error>
            {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn verify_tls13_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                vec![
                    rustls::SignatureScheme::ED25519,
                    rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                    rustls::SignatureScheme::RSA_PSS_SHA256,
                ]
            }
        }

        let client_crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DevOnlySkipVerify))
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
        let cert = rcgen::generate_simple_self_signed(vec!["synergos".into()])
            .map_err(|e| SynergosNetError::Quic(format!("Cert generation error: {}", e)))?;

        let cert_der = rustls::pki_types::CertificateDer::from(cert.cert);
        let key_der = rustls::pki_types::PrivateKeyDer::try_from(cert.key_pair.serialize_der())
            .map_err(|e| SynergosNetError::Quic(format!("Key error: {}", e)))?;

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

/// コネクション情報（外部公開用、quinn ハンドルなし）
#[derive(Debug, Clone)]
pub struct QuicConnectionInfo {
    pub peer_id: PeerId,
    pub remote_addr: SocketAddr,
    pub active_streams: u32,
    pub state: QuicConnectionState,
    pub rtt_ms: u32,
}
