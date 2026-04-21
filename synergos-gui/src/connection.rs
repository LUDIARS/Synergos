//! synergos-core への IPC 接続管理
//!
//! tokio ランタイムを使って非同期 IPC 通信を行い、
//! egui の即時モード描画から利用できるようキャッシュを提供する。

use std::sync::{Arc, Mutex};

use synergos_ipc::command::IpcCommand;
use synergos_ipc::response::{
    ConflictInfoDto, DaemonStatus, NetworkStatusInfo, PeerInfo, ProjectInfo, TransferInfo,
};

/// Core デーモンへの接続状態
pub struct CoreConnection {
    connected: bool,
    /// tokio ランタイム（GUI スレッドとは別にバックグラウンドで動作）
    runtime: tokio::runtime::Runtime,
    /// キャッシュされた状態
    pub cache: Arc<Mutex<ConnectionCache>>,
}

/// IPC 経由で取得したデータのキャッシュ
#[derive(Debug, Clone, Default)]
pub struct ConnectionCache {
    pub status: Option<DaemonStatus>,
    pub projects: Vec<ProjectInfo>,
    pub peers: Vec<PeerInfo>,
    pub transfers: Vec<TransferInfo>,
    pub network: Option<NetworkStatusInfo>,
    pub conflicts: Vec<ConflictInfoDto>,
    /// 最後にリフレッシュした時刻
    pub last_refresh: Option<std::time::Instant>,
}

impl CoreConnection {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        let cache = Arc::new(Mutex::new(ConnectionCache::default()));

        let mut conn = Self {
            connected: false,
            runtime,
            cache,
        };

        // 初回接続を試行
        conn.try_connect();
        conn
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// デーモンへの接続を試行し、状態を更新
    pub fn try_connect(&mut self) {
        let cache = self.cache.clone();
        let connected = self.runtime.block_on(async {
            if !synergos_ipc::IpcClient::is_daemon_running().await {
                return false;
            }
            // 接続してステータスを取得
            match synergos_ipc::IpcClient::connect().await {
                Ok(mut client) => {
                    let status = client.send(IpcCommand::Status).await.ok();
                    let projects = client.send(IpcCommand::ProjectList).await.ok();
                    let network = client.send(IpcCommand::NetworkStatus).await.ok();

                    let mut c = cache.lock().unwrap();
                    if let Some(synergos_ipc::IpcResponse::Status(s)) = status {
                        c.status = Some(s);
                    }
                    if let Some(synergos_ipc::IpcResponse::ProjectList(p)) = projects {
                        c.projects = p;
                    }
                    if let Some(synergos_ipc::IpcResponse::NetworkStatus(n)) = network {
                        c.network = Some(n);
                    }
                    c.last_refresh = Some(std::time::Instant::now());
                    true
                }
                Err(_) => false,
            }
        });
        self.connected = connected;
    }

    /// キャッシュをリフレッシュ（一定間隔で呼ぶ）
    pub fn refresh_if_needed(&mut self) {
        let should_refresh = {
            let cache = self.cache.lock().unwrap();
            match cache.last_refresh {
                Some(t) => t.elapsed() > std::time::Duration::from_secs(2),
                None => true,
            }
        };
        if should_refresh {
            self.try_connect();
        }
    }

    /// IPC コマンドを送信してレスポンスを取得
    pub fn send_command(&self, command: IpcCommand) -> Option<synergos_ipc::IpcResponse> {
        self.runtime.block_on(async {
            match synergos_ipc::IpcClient::connect().await {
                Ok(mut client) => client.send(command).await.ok(),
                Err(_) => None,
            }
        })
    }

    /// ピア一覧をリフレッシュ
    pub fn refresh_peers(&self, project_id: &str) {
        let resp = self.send_command(IpcCommand::PeerList {
            project_id: project_id.to_string(),
        });
        if let Some(synergos_ipc::IpcResponse::PeerList(peers)) = resp {
            let mut cache = self.cache.lock().unwrap();
            cache.peers = peers;
        }
    }

    /// 転送一覧をリフレッシュ
    pub fn refresh_transfers(&self) {
        let resp = self.send_command(IpcCommand::TransferList { project_id: None });
        if let Some(synergos_ipc::IpcResponse::TransferList(transfers)) = resp {
            let mut cache = self.cache.lock().unwrap();
            cache.transfers = transfers;
        }
    }

    // ── ユーザ操作ヘルパ (#13: GUI 操作 UI) ──

    /// プロジェクトを新規に開く (ローカル root を指定)。成功時 true。
    pub fn open_project(&self, project_id: &str, root: &std::path::Path) -> bool {
        let resp = self.send_command(IpcCommand::ProjectOpen {
            project_id: project_id.into(),
            root_path: root.to_path_buf(),
            display_name: None,
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    /// プロジェクトを閉じる。成功時 true。
    pub fn close_project(&self, project_id: &str) -> bool {
        let resp = self.send_command(IpcCommand::ProjectClose {
            project_id: project_id.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    /// 招待トークンを生成。成功時 Some((token, expires_at))。
    pub fn create_invite(
        &self,
        project_id: &str,
        expires_in_secs: Option<u64>,
    ) -> Option<(String, Option<u64>)> {
        let resp = self.send_command(IpcCommand::ProjectCreateInvite {
            project_id: project_id.into(),
            expires_in_secs,
        });
        if let Some(synergos_ipc::IpcResponse::InviteToken { token, expires_at }) = resp {
            Some((token, expires_at))
        } else {
            None
        }
    }

    /// ピアに接続する。成功時 true。
    pub fn peer_connect(&self, project_id: &str, peer_id: &str) -> bool {
        let resp = self.send_command(IpcCommand::PeerConnect {
            project_id: project_id.into(),
            peer_id: peer_id.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    /// ピアを切断する。成功時 true。
    pub fn peer_disconnect(&self, peer_id: &str) -> bool {
        let resp = self.send_command(IpcCommand::PeerDisconnect {
            peer_id: peer_id.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    /// 転送をキャンセルする。成功時 true。
    pub fn cancel_transfer(&self, transfer_id: &str) -> bool {
        let resp = self.send_command(IpcCommand::TransferCancel {
            transfer_id: transfer_id.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    /// 転送を発行する (ピアから file_id を取得)。成功時 true。
    pub fn request_transfer(&self, project_id: &str, file_id: &str, peer_id: &str) -> bool {
        let resp = self.send_command(IpcCommand::TransferRequest {
            project_id: project_id.into(),
            file_id: file_id.into(),
            peer_id: peer_id.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    // ── コンフリクト (#13) ──

    /// コンフリクト一覧をリフレッシュ
    pub fn refresh_conflicts(&self, project_id: Option<&str>) {
        let resp = self.send_command(IpcCommand::ConflictList {
            project_id: project_id.map(String::from),
        });
        if let Some(synergos_ipc::IpcResponse::ConflictList(items)) = resp {
            let mut cache = self.cache.lock().unwrap();
            cache.conflicts = items;
        }
    }

    /// コンフリクトを解決する。
    /// `resolution` は "keep_local" / "accept_remote" / "manual_merge"。
    pub fn resolve_conflict(&self, file_id: &str, resolution: &str) -> bool {
        let resp = self.send_command(IpcCommand::ConflictResolve {
            file_id: file_id.into(),
            resolution: resolution.into(),
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }

    // ── 設定変更 (#13) ──

    /// ネットワーク設定を部分更新する。成功時 true。
    pub fn update_config(
        &self,
        mesh_n: Option<u16>,
        max_concurrent_streams: Option<u32>,
        tunnel_hostname: Option<String>,
    ) -> bool {
        let resp = self.send_command(IpcCommand::ConfigUpdate {
            mesh_n,
            max_concurrent_streams,
            tunnel_hostname,
        });
        matches!(resp, Some(synergos_ipc::IpcResponse::Ok))
    }
}
