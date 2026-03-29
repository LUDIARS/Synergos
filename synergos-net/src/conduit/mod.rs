//! Conduit — 接続管理
//!
//! ピアとのコネクションのライフサイクルを管理するコアコンポーネント。
//! Route Discovery、接続確立、Route Migration、Keepalive を担当する。

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::config::NetConfig;
use crate::error::{Result, SynergosNetError};
use crate::mesh::{Mesh, ProbeResult};
use crate::quic::{QuicConnectionInfo, QuicManager};
use crate::tunnel::{TunnelManager, TunnelState};
use crate::types::{ConnectionState, PeerId, PeerEndpoint, Route, RouteKind};

/// 経路検出結果
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// IPv6 到達性プローブ結果
    pub ipv6: ProbeResult,
    /// Tunnel プローブ結果
    pub tunnel: TunnelProbeResult,
    /// 推奨経路
    pub recommended: RouteKind,
}

/// Tunnel プローブ結果
#[derive(Debug, Clone)]
pub struct TunnelProbeResult {
    pub available: bool,
    pub rtt_ms: Option<u32>,
}

/// ピア接続の管理情報
#[derive(Debug, Clone)]
struct ManagedPeer {
    peer_id: PeerId,
    display_name: String,
    routes: Vec<Route>,
    active_route: Option<RouteKind>,
    state: ConnectionState,
    last_seen: Instant,
    /// 最新のプローブ結果
    last_detection: Option<DetectionResult>,
}

/// 接続管理
///
/// 各ピアへの最適な接続経路を選択し、接続の確立・維持・切り替えを管理する。
/// IPv6 Direct → Tunnel → Relay の優先度で接続を試行する。
pub struct Conduit {
    /// 管理中のピア（PeerId → ManagedPeer）
    peers: DashMap<PeerId, ManagedPeer>,
    /// QUIC セッションマネージャ
    quic: Arc<QuicManager>,
    /// Tunnel マネージャ
    tunnel: Arc<TunnelManager>,
    /// Mesh マネージャ
    mesh: Arc<Mesh>,
    /// Keepalive 間隔
    keepalive_interval: Duration,
}

impl Conduit {
    pub fn new(
        quic: Arc<QuicManager>,
        tunnel: Arc<TunnelManager>,
        mesh: Arc<Mesh>,
        keepalive_interval: Duration,
    ) -> Self {
        Self {
            peers: DashMap::new(),
            quic,
            tunnel,
            mesh,
            keepalive_interval,
        }
    }

    /// ピアを登録し、経路を検出する
    pub async fn register_peer(&self, endpoint: PeerEndpoint) -> Result<RouteKind> {
        let peer_id = endpoint.peer_id.clone();

        let managed = ManagedPeer {
            peer_id: peer_id.clone(),
            display_name: endpoint.display_name.clone(),
            routes: endpoint.routes.clone(),
            active_route: None,
            state: ConnectionState::Discovered,
            last_seen: Instant::now(),
            last_detection: None,
        };

        self.peers.insert(peer_id.clone(), managed);

        // 経路検出を実行
        let detection = self.detect_route(&endpoint).await;
        let recommended = detection.recommended;

        if let Some(mut entry) = self.peers.get_mut(&peer_id) {
            entry.last_detection = Some(detection);
        }

        tracing::info!(
            "Registered peer {} — recommended route: {:?}",
            peer_id,
            recommended
        );

        Ok(recommended)
    }

    /// ピアに接続する（最適な経路を自動選択）
    pub async fn connect_peer(&self, peer_id: &PeerId) -> Result<RouteKind> {
        let peer = self
            .peers
            .get(peer_id)
            .ok_or_else(|| SynergosNetError::PeerNotFound(peer_id.to_string()))?;

        // 状態を Connecting に更新
        drop(peer);
        if let Some(mut entry) = self.peers.get_mut(peer_id) {
            entry.state = ConnectionState::Connecting;
        }

        let peer = self
            .peers
            .get(peer_id)
            .ok_or_else(|| SynergosNetError::PeerNotFound(peer_id.to_string()))?;

        // 経路の優先度順に接続を試行
        let route_result = self.try_connect_routes(peer_id, &peer.routes).await;

        match route_result {
            Ok(route_kind) => {
                drop(peer);
                if let Some(mut entry) = self.peers.get_mut(peer_id) {
                    entry.active_route = Some(route_kind);
                    entry.state = ConnectionState::Connected {
                        rtt_ms: 0,
                        route: route_kind,
                    };
                    entry.last_seen = Instant::now();
                }
                tracing::info!("Connected to peer {} via {:?}", peer_id, route_kind);
                Ok(route_kind)
            }
            Err(e) => {
                drop(peer);
                if let Some(mut entry) = self.peers.get_mut(peer_id) {
                    entry.state = ConnectionState::Disconnected {
                        reason: e.to_string(),
                    };
                }
                Err(e)
            }
        }
    }

    /// ピアとの接続を切断する
    pub async fn disconnect_peer(&self, peer_id: &PeerId, reason: &str) -> Result<()> {
        self.quic.disconnect(peer_id, reason).await;

        if let Some(mut entry) = self.peers.get_mut(peer_id) {
            entry.state = ConnectionState::Disconnected {
                reason: reason.to_string(),
            };
            entry.active_route = None;
        }

        tracing::info!("Disconnected peer {}: {}", peer_id, reason);
        Ok(())
    }

    /// ピアを登録解除する
    pub fn unregister_peer(&self, peer_id: &PeerId) {
        self.peers.remove(peer_id);
    }

    /// 接続中のピア一覧を取得
    pub fn connected_peers(&self) -> Vec<PeerConnectionStatus> {
        self.peers
            .iter()
            .filter_map(|entry| {
                let peer = entry.value();
                if let ConnectionState::Connected { rtt_ms, route } = &peer.state {
                    Some(PeerConnectionStatus {
                        peer_id: peer.peer_id.clone(),
                        display_name: peer.display_name.clone(),
                        route: *route,
                        rtt_ms: *rtt_ms,
                        bandwidth_bps: self.quic.get_bandwidth_estimate(&peer.peer_id),
                        last_seen: peer.last_seen,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    /// 指定ピアの接続状態を取得
    pub fn get_peer_state(&self, peer_id: &PeerId) -> Option<ConnectionState> {
        self.peers
            .get(peer_id)
            .map(|entry| entry.state.clone())
    }

    /// Route Migration: より高優先度の経路が利用可能になった場合に切り替え
    pub async fn try_migrate_route(&self, peer_id: &PeerId) -> Result<Option<RouteKind>> {
        let peer = self
            .peers
            .get(peer_id)
            .ok_or_else(|| SynergosNetError::PeerNotFound(peer_id.to_string()))?;

        let current_route = match &peer.active_route {
            Some(r) => *r,
            None => return Ok(None),
        };

        let endpoint = PeerEndpoint {
            peer_id: peer.peer_id.clone(),
            display_name: peer.display_name.clone(),
            routes: peer.routes.clone(),
            state: peer.state.clone(),
        };
        drop(peer);

        // 新しい経路検出
        let detection = self.detect_route(&endpoint).await;
        let new_route = detection.recommended;

        // より高優先度の経路が利用可能なら切り替え
        if route_priority(new_route) > route_priority(current_route) {
            tracing::info!(
                "Migrating peer {} route: {:?} → {:?}",
                peer_id,
                current_route,
                new_route
            );

            // 旧接続を切断
            self.quic.disconnect(peer_id, "route migration").await;

            // 新経路で接続
            let routes: Vec<Route> = endpoint.routes.clone();
            let result = self.try_connect_routes(peer_id, &routes).await;

            match result {
                Ok(actual_route) => {
                    if let Some(mut entry) = self.peers.get_mut(peer_id) {
                        entry.active_route = Some(actual_route);
                        entry.state = ConnectionState::Connected {
                            rtt_ms: 0,
                            route: actual_route,
                        };
                        entry.last_detection = Some(detection);
                    }
                    Ok(Some(actual_route))
                }
                Err(e) => {
                    tracing::warn!("Route migration failed for {}: {}", peer_id, e);
                    // 元の経路で再接続を試みる
                    let _ = self.try_connect_routes(peer_id, &routes).await;
                    Ok(None)
                }
            }
        } else {
            // 現在の経路が最適
            if let Some(mut entry) = self.peers.get_mut(peer_id) {
                entry.last_detection = Some(detection);
            }
            Ok(None)
        }
    }

    /// 全接続を切断する
    pub async fn shutdown(&self) {
        let peer_ids: Vec<PeerId> = self.peers.iter().map(|e| e.key().clone()).collect();
        for peer_id in peer_ids {
            let _ = self.disconnect_peer(&peer_id, "shutdown").await;
        }
        self.peers.clear();
    }

    // ── 内部ヘルパー ──

    /// 経路を検出する
    async fn detect_route(&self, endpoint: &PeerEndpoint) -> DetectionResult {
        // IPv6 と Tunnel のプローブを並列実行
        let ipv6_probe = self.probe_ipv6_route(endpoint);
        let tunnel_probe = self.probe_tunnel_route();

        let (ipv6, tunnel) = tokio::join!(ipv6_probe, tunnel_probe);

        let recommended = match (ipv6.reachable, tunnel.available) {
            (true, _) => RouteKind::Direct,   // IPv6 最優先
            (false, true) => RouteKind::Tunnel, // Tunnel フォールバック
            (false, false) => RouteKind::Relay,  // WebSocket リレー
        };

        DetectionResult {
            ipv6,
            tunnel,
            recommended,
        }
    }

    /// IPv6 到達性をプローブ
    async fn probe_ipv6_route(&self, endpoint: &PeerEndpoint) -> ProbeResult {
        for route in &endpoint.routes {
            if let Route::Direct { addr, .. } = route {
                return self.mesh.probe_ipv6(*addr).await;
            }
        }

        ProbeResult {
            reachable: false,
            rtt_ms: None,
            error: Some("No direct route available".into()),
            addr: None,
        }
    }

    /// Tunnel 可用性をプローブ
    async fn probe_tunnel_route(&self) -> TunnelProbeResult {
        let health = self.tunnel.health_check().await;
        TunnelProbeResult {
            available: health.reachable,
            rtt_ms: health.rtt_ms,
        }
    }

    /// 経路の優先度順に接続を試行
    async fn try_connect_routes(
        &self,
        peer_id: &PeerId,
        routes: &[Route],
    ) -> Result<RouteKind> {
        // 1. IPv6 Direct を試行
        for route in routes {
            if let Route::Direct { addr, fqdn } = route {
                let server_name = fqdn
                    .as_deref()
                    .unwrap_or("synergos");
                match self
                    .quic
                    .connect(peer_id.clone(), std::net::SocketAddr::V6(*addr), server_name)
                    .await
                {
                    Ok(()) => return Ok(RouteKind::Direct),
                    Err(e) => {
                        tracing::debug!("IPv6 direct connection failed: {}", e);
                    }
                }
            }
        }

        // 2. Tunnel を試行
        for route in routes {
            if let Route::Tunnel { hostname, .. } = route {
                // Tunnel 経由の場合、hostname をアドレスとして接続
                // （実際の接続は cloudflared が中継する）
                tracing::debug!("Attempting tunnel connection via {}", hostname);
                // Tunnel が利用可能なら接続成功として扱う
                if self.tunnel.is_available().await {
                    return Ok(RouteKind::Tunnel);
                }
            }
        }

        // 3. Relay を試行
        for route in routes {
            if let Route::Relay { server_url, .. } = route {
                tracing::debug!("Attempting relay connection via {}", server_url);
                // WebSocket Relay は今後実装
                return Ok(RouteKind::Relay);
            }
        }

        Err(SynergosNetError::Quic(
            "All connection routes failed".into(),
        ))
    }
}

/// 経路の優先度（大きいほど高優先）
fn route_priority(kind: RouteKind) -> u8 {
    match kind {
        RouteKind::Direct => 3,
        RouteKind::Tunnel => 2,
        RouteKind::Relay => 1,
    }
}

/// ピア接続状態（外部公開用）
#[derive(Debug, Clone)]
pub struct PeerConnectionStatus {
    pub peer_id: PeerId,
    pub display_name: String,
    pub route: RouteKind,
    pub rtt_ms: u32,
    pub bandwidth_bps: u64,
    pub last_seen: Instant,
}
