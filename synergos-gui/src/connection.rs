//! synergos-core への IPC 接続管理
//!
//! tokio ランタイムを使って非同期 IPC 通信を行い、
//! egui の即時モード描画から利用できるようキャッシュを提供する。

use std::sync::{Arc, Mutex};

use synergos_ipc::command::IpcCommand;
use synergos_ipc::response::{
    DaemonStatus, NetworkStatusInfo, PeerInfo, ProjectInfo, TransferInfo,
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
}
