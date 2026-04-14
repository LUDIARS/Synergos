//! IPC コマンド定義
//!
//! クライアント → synergos-core デーモンへのリクエスト。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::event::EventFilter;

/// クライアントから Core デーモンへのコマンド
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    // ── デーモン制御 ──
    /// 疎通確認
    Ping,
    /// デーモン停止
    Shutdown,
    /// デーモン状態取得
    Status,

    // ── プロジェクト管理 ──
    /// プロジェクトを開く（ネットワーク参加）
    ProjectOpen {
        project_id: String,
        root_path: PathBuf,
        /// プロジェクト表示名（省略時は project_id を使用）
        display_name: Option<String>,
    },
    /// プロジェクトを閉じる（ネットワーク離脱）
    ProjectClose { project_id: String },
    /// 管理中のプロジェクト一覧
    ProjectList,
    /// プロジェクトの詳細情報を取得
    ProjectGet { project_id: String },
    /// プロジェクト設定を更新
    ProjectUpdate {
        project_id: String,
        /// 更新する設定フィールド（None のフィールドは変更しない）
        display_name: Option<String>,
        description: Option<String>,
        sync_mode: Option<String>,
        max_peers: Option<u16>,
    },
    /// プロジェクトの招待トークンを生成
    ProjectCreateInvite {
        project_id: String,
        /// トークンの有効期限（秒）。None の場合は無期限
        expires_in_secs: Option<u64>,
    },
    /// 招待トークンでプロジェクトに参加
    ProjectJoin {
        invite_token: String,
        root_path: PathBuf,
    },

    // ── ピア管理 ──
    /// 接続中のピア一覧
    PeerList { project_id: String },
    /// 指定ピアに接続
    PeerConnect { project_id: String, peer_id: String },
    /// 指定ピアを切断
    PeerDisconnect { peer_id: String },

    // ── ファイル転送 ──
    /// ファイル転送リクエスト
    TransferRequest {
        project_id: String,
        file_id: String,
        peer_id: String,
    },
    /// アクティブ転送一覧
    TransferList { project_id: Option<String> },
    /// 転送をキャンセル
    TransferCancel { transfer_id: String },
    /// ファイル更新を公開
    PublishUpdate {
        project_id: String,
        file_paths: Vec<PathBuf>,
    },

    // ── モニタリング ──
    /// ネットワーク状態取得
    NetworkStatus,
    /// イベント購読開始
    Subscribe { events: Vec<EventFilter> },
    /// イベント購読解除
    Unsubscribe { subscription_id: String },
}
