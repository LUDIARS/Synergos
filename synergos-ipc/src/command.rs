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
    },
    /// プロジェクトを閉じる（ネットワーク離脱）
    ProjectClose {
        project_id: String,
    },
    /// 管理中のプロジェクト一覧
    ProjectList,

    // ── ピア管理 ──

    /// 接続中のピア一覧
    PeerList {
        project_id: String,
    },
    /// 指定ピアに接続
    PeerConnect {
        project_id: String,
        peer_id: String,
    },
    /// 指定ピアを切断
    PeerDisconnect {
        peer_id: String,
    },

    // ── ファイル転送 ──

    /// ファイル転送リクエスト
    TransferRequest {
        project_id: String,
        file_id: String,
        peer_id: String,
    },
    /// アクティブ転送一覧
    TransferList {
        project_id: Option<String>,
    },
    /// 転送をキャンセル
    TransferCancel {
        transfer_id: String,
    },
    /// ファイル更新を公開
    PublishUpdate {
        project_id: String,
        file_paths: Vec<PathBuf>,
    },

    // ── モニタリング ──

    /// ネットワーク状態取得
    NetworkStatus,
    /// イベント購読開始
    Subscribe {
        events: Vec<EventFilter>,
    },
    /// イベント購読解除
    Unsubscribe {
        subscription_id: String,
    },
}
