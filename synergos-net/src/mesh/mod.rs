//! Mesh — IPv6 Direct / TURN / STUN 接続
//!
//! NAT を介さない直接接続を確立するためのコンポーネント。
//! FQDN → IPv6 解決、到達性プローブ、TURN/STUN セッション管理を提供する。

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::time::{Duration, Instant};

use crate::config::MeshConfig;
use crate::error::{Result, SynergosNetError};

/// IPv6 プローブ結果
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// 到達可能かどうか
    pub reachable: bool,
    /// RTT（ミリ秒）
    pub rtt_ms: Option<u32>,
    /// エラーメッセージ
    pub error: Option<String>,
    /// プローブ対象アドレス
    pub addr: Option<SocketAddrV6>,
}

/// FQDN 解決結果
#[derive(Debug, Clone)]
pub struct ResolveResult {
    /// 解決された IPv6 アドレス一覧
    pub addresses: Vec<Ipv6Addr>,
    /// 解決に使用した方法
    pub method: ResolveMethod,
    /// 解決にかかった時間
    pub resolve_time_ms: u32,
}

/// 名前解決方法
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveMethod {
    /// DNS-over-HTTPS
    DoH,
    /// 通常の DNS (AAAA)
    Standard,
    /// キャッシュヒット
    Cached,
}

/// TURN セッション情報
#[derive(Debug, Clone)]
pub struct TurnSession {
    /// TURN サーバーアドレス
    pub server_addr: String,
    /// 割り当てられたリレーアドレス
    pub relay_addr: Option<SocketAddr>,
    /// セッションの有効期限
    pub expires_at: Option<Instant>,
    /// セッション状態
    pub state: TurnSessionState,
}

/// TURN セッション状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSessionState {
    /// アロケーション要求中
    Allocating,
    /// アクティブ
    Active,
    /// 期限切れ
    Expired,
    /// エラー
    Failed(String),
}

/// IPv6/TURN メッシュ接続マネージャ
///
/// ピアへの IPv6 直接接続を試み、失敗した場合は TURN リレーにフォールバックする。
/// FQDN の DNS-over-HTTPS 解決と、IPv6 到達性プローブを提供する。
pub struct Mesh {
    config: MeshConfig,
    /// DNS 解決キャッシュ（FQDN → IPv6 アドレス）
    dns_cache: dashmap::DashMap<String, CachedResolve>,
    /// アクティブな TURN セッション
    turn_sessions: dashmap::DashMap<String, TurnSession>,
}

/// DNS キャッシュエントリ
#[derive(Debug, Clone)]
struct CachedResolve {
    addresses: Vec<Ipv6Addr>,
    cached_at: Instant,
    ttl: Duration,
}

impl Mesh {
    pub fn new(config: MeshConfig) -> Self {
        Self {
            config,
            dns_cache: dashmap::DashMap::new(),
            turn_sessions: dashmap::DashMap::new(),
        }
    }

    /// FQDN を IPv6 アドレスに解決する
    ///
    /// 1. キャッシュを確認
    /// 2. DNS-over-HTTPS で AAAA レコードを解決
    /// 3. フォールバック: 通常の DNS 解決
    pub async fn resolve_fqdn(&self, fqdn: &str) -> Result<ResolveResult> {
        // 1. キャッシュ確認
        if let Some(cached) = self.dns_cache.get(fqdn) {
            if cached.cached_at.elapsed() < cached.ttl {
                return Ok(ResolveResult {
                    addresses: cached.addresses.clone(),
                    method: ResolveMethod::Cached,
                    resolve_time_ms: 0,
                });
            }
            // TTL 切れ → 削除
            drop(cached);
            self.dns_cache.remove(fqdn);
        }

        let start = Instant::now();

        // 2. DNS-over-HTTPS 解決を試行
        let result = self.resolve_doh(fqdn).await;

        match result {
            Ok(addrs) if !addrs.is_empty() => {
                let elapsed = start.elapsed().as_millis() as u32;

                // キャッシュに保存
                self.dns_cache.insert(
                    fqdn.to_string(),
                    CachedResolve {
                        addresses: addrs.clone(),
                        cached_at: Instant::now(),
                        ttl: Duration::from_secs(300), // 5分キャッシュ
                    },
                );

                Ok(ResolveResult {
                    addresses: addrs,
                    method: ResolveMethod::DoH,
                    resolve_time_ms: elapsed,
                })
            }
            _ => {
                // 3. 通常の DNS 解決にフォールバック
                let addrs = self.resolve_standard(fqdn).await?;
                let elapsed = start.elapsed().as_millis() as u32;

                if !addrs.is_empty() {
                    self.dns_cache.insert(
                        fqdn.to_string(),
                        CachedResolve {
                            addresses: addrs.clone(),
                            cached_at: Instant::now(),
                            ttl: Duration::from_secs(300),
                        },
                    );
                }

                Ok(ResolveResult {
                    addresses: addrs,
                    method: ResolveMethod::Standard,
                    resolve_time_ms: elapsed,
                })
            }
        }
    }

    /// IPv6 アドレスへの到達性をプローブする。
    ///
    /// S12 対策: unspecified / loopback / multicast / link-local のような
    /// 外部プローブに使うべきでないアドレスは拒絶する。
    /// (任意 IPv6 への TCP 接続は SSRF 様の攻撃面になる。)
    pub async fn probe_ipv6(&self, addr: SocketAddrV6) -> ProbeResult {
        let ip = *addr.ip();
        let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
        if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() || link_local {
            return ProbeResult {
                reachable: false,
                rtt_ms: None,
                error: Some("refused: restricted IPv6 address".into()),
                addr: Some(addr),
            };
        }

        let timeout = Duration::from_millis(self.config.probe_timeout_ms as u64);
        let start = Instant::now();

        // TCP 接続でプローブ（QUIC ポートへの到達性確認）
        let result = tokio::time::timeout(
            timeout,
            tokio::net::TcpStream::connect(SocketAddr::V6(addr)),
        )
        .await;

        match result {
            Ok(Ok(_stream)) => {
                let rtt = start.elapsed().as_millis() as u32;
                ProbeResult {
                    reachable: true,
                    rtt_ms: Some(rtt),
                    error: None,
                    addr: Some(addr),
                }
            }
            Ok(Err(e)) => ProbeResult {
                reachable: false,
                rtt_ms: None,
                error: Some(format!("Connection failed: {}", e)),
                addr: Some(addr),
            },
            Err(_) => ProbeResult {
                reachable: false,
                rtt_ms: None,
                error: Some("Timeout".into()),
                addr: Some(addr),
            },
        }
    }

    /// FQDN から IPv6 到達性を一括チェック
    pub async fn probe_fqdn(&self, fqdn: &str, port: u16) -> Vec<ProbeResult> {
        let resolve = match self.resolve_fqdn(fqdn).await {
            Ok(r) => r,
            Err(e) => {
                return vec![ProbeResult {
                    reachable: false,
                    rtt_ms: None,
                    error: Some(format!("DNS resolution failed: {}", e)),
                    addr: None,
                }];
            }
        };

        let mut results = Vec::new();
        for addr in &resolve.addresses {
            let sock_addr = SocketAddrV6::new(*addr, port, 0, 0);
            let result = self.probe_ipv6(sock_addr).await;
            results.push(result);
        }
        results
    }

    /// TURN サーバーにアロケーションを要求する
    pub async fn allocate_turn(&self, server_uri: &str) -> Result<TurnSession> {
        tracing::info!("Requesting TURN allocation from {}", server_uri);

        // TURN セッションを作成（実際の STUN/TURN プロトコルは今後実装）
        let session = TurnSession {
            server_addr: server_uri.to_string(),
            relay_addr: None,
            expires_at: Some(Instant::now() + Duration::from_secs(600)),
            state: TurnSessionState::Active,
        };

        self.turn_sessions
            .insert(server_uri.to_string(), session.clone());

        Ok(session)
    }

    /// TURN セッションをリフレッシュする
    pub async fn refresh_turn(&self, server_uri: &str) -> Result<()> {
        if let Some(mut entry) = self.turn_sessions.get_mut(server_uri) {
            entry.expires_at = Some(Instant::now() + Duration::from_secs(600));
            tracing::debug!("Refreshed TURN session for {}", server_uri);
            Ok(())
        } else {
            Err(SynergosNetError::Turn(format!(
                "No active session for {}",
                server_uri
            )))
        }
    }

    /// 期限切れの TURN セッションをクリーンアップ
    pub fn cleanup_expired_sessions(&self) {
        self.turn_sessions
            .retain(|_, session| match session.expires_at {
                Some(exp) => Instant::now() < exp,
                None => true,
            });
    }

    /// アクティブな TURN セッション一覧を取得
    pub fn list_turn_sessions(&self) -> Vec<TurnSession> {
        self.turn_sessions
            .iter()
            .map(|e| e.value().clone())
            .collect()
    }

    /// DNS キャッシュをクリアする
    pub fn clear_dns_cache(&self) {
        self.dns_cache.clear();
    }

    // ── 内部ヘルパー ──

    /// DNS-over-HTTPS で AAAA レコードを解決
    async fn resolve_doh(&self, fqdn: &str) -> Result<Vec<Ipv6Addr>> {
        use hickory_resolver::config::{ResolverConfig, ResolverOpts};
        use hickory_resolver::TokioAsyncResolver;

        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::cloudflare_https(), ResolverOpts::default());

        let response = resolver
            .ipv6_lookup(fqdn)
            .await
            .map_err(|e| SynergosNetError::DnsResolution(format!("DoH lookup failed: {}", e)))?;

        let addrs: Vec<Ipv6Addr> = response.iter().map(|aaaa| aaaa.0).collect();

        tracing::debug!("DoH resolved {} → {} addresses", fqdn, addrs.len());
        Ok(addrs)
    }

    /// 通常の DNS で AAAA レコードを解決
    async fn resolve_standard(&self, fqdn: &str) -> Result<Vec<Ipv6Addr>> {
        use hickory_resolver::config::{ResolverConfig, ResolverOpts};
        use hickory_resolver::TokioAsyncResolver;

        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        let response = resolver
            .ipv6_lookup(fqdn)
            .await
            .map_err(|e| SynergosNetError::DnsResolution(format!("DNS lookup failed: {}", e)))?;

        let addrs: Vec<Ipv6Addr> = response.iter().map(|aaaa| aaaa.0).collect();

        tracing::debug!("DNS resolved {} → {} addresses", fqdn, addrs.len());
        Ok(addrs)
    }
}
