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
    /// peer-info HTTP servlet (URL) 経由で bootstrap 情報を取得 → QUIC 直結する。
    /// `url` は `https://host[:port]` 形式 (path は自動で `/peer-info` を付与)。
    /// invite token を必要としないクロスマシン peer 追加経路。
    PeerAddByUrl { project_id: String, url: String },

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

    // ── コンフリクト管理 ──
    /// プロジェクトのアクティブなコンフリクト一覧
    ConflictList { project_id: Option<String> },
    /// コンフリクトを解決する
    ConflictResolve {
        file_id: String,
        /// "keep_local" | "accept_remote" | "manual_merge"
        resolution: String,
    },

    // ── 設定変更 ──
    /// NetConfig の部分更新 (受け取れる主要フィールドだけ)
    ConfigUpdate {
        /// gossipsub.mesh_n
        mesh_n: Option<u16>,
        /// quic.max_concurrent_streams
        max_concurrent_streams: Option<u32>,
        /// tunnel.hostname
        tunnel_hostname: Option<String>,
    },

    // ── モニタリング ──
    /// ネットワーク状態取得
    NetworkStatus,
    /// イベント購読開始
    Subscribe { events: Vec<EventFilter> },
    /// イベント購読解除
    Unsubscribe { subscription_id: String },
}

impl IpcCommand {
    /// 入力バリデーション。dispatcher が処理に入る前に呼ばれ、Err なら
    /// 不正コマンドとして即座に Error 応答を返す。
    ///
    /// 主なチェック:
    /// - 識別子の空文字 (project_id / peer_id / file_id 等)
    /// - 過大な長さ (DoS 防止: 1 KiB を超える文字列はカット)
    /// - PublishUpdate.file_paths は空でないこと、上限 1024 件
    /// - パスに `..` 等のディレクトリ脱出 component が含まれないこと (CWE-22)
    pub fn validate(&self) -> Result<(), String> {
        const MAX_ID_LEN: usize = 1024;
        const MAX_PATHS_PER_PUBLISH: usize = 1024;
        const MAX_PATH_LEN: usize = 4096;

        let check_id = |label: &str, s: &str| -> Result<(), String> {
            if s.trim().is_empty() {
                Err(format!("{label} must not be empty"))
            } else if s.len() > MAX_ID_LEN {
                Err(format!("{label} too long ({} > {MAX_ID_LEN})", s.len()))
            } else {
                Ok(())
            }
        };

        // PathBuf の component をスキャンし、`..` (ParentDir) を拒否する。
        // 絶対パス (root_path 等) はそのまま許容、相対パス (publish file_paths) も
        // OK だが、いずれにせよ親階層への脱出は許さない。
        let check_path = |label: &str, path: &PathBuf| -> Result<(), String> {
            if path.as_os_str().is_empty() {
                return Err(format!("{label} must not be empty"));
            }
            let s = path.to_string_lossy();
            if s.len() > MAX_PATH_LEN {
                return Err(format!("{label} too long ({} > {MAX_PATH_LEN})", s.len()));
            }
            for component in path.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(format!(
                        "{label} must not contain '..' component: {}",
                        path.display()
                    ));
                }
            }
            // 文字列上での `..` 単独 component / `\0` 含みも弾く (Windows / Unix 両方)
            if s.contains('\0') {
                return Err(format!("{label} must not contain NUL byte"));
            }
            Ok(())
        };

        match self {
            Self::Ping
            | Self::Shutdown
            | Self::Status
            | Self::NetworkStatus
            | Self::ProjectList => Ok(()),

            Self::ProjectOpen {
                project_id,
                root_path,
                ..
            } => {
                check_id("project_id", project_id)?;
                check_path("root_path", root_path)
            }
            Self::ProjectClose { project_id }
            | Self::ProjectGet { project_id }
            | Self::ProjectCreateInvite { project_id, .. }
            | Self::PeerList { project_id }
            | Self::ProjectUpdate { project_id, .. } => check_id("project_id", project_id),

            Self::ProjectJoin {
                invite_token,
                root_path,
                ..
            } => {
                check_id("invite_token", invite_token)?;
                check_path("root_path", root_path)
            }

            Self::PeerConnect {
                project_id,
                peer_id,
            } => {
                check_id("project_id", project_id)?;
                check_id("peer_id", peer_id)
            }
            Self::PeerDisconnect { peer_id } => check_id("peer_id", peer_id),
            Self::PeerAddByUrl { project_id, url } => {
                check_id("project_id", project_id)?;
                check_id("url", url)?;
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err("url must start with http:// or https://".into());
                }
                Ok(())
            }

            Self::TransferRequest {
                project_id,
                file_id,
                peer_id,
            } => {
                check_id("project_id", project_id)?;
                check_id("file_id", file_id)?;
                check_id("peer_id", peer_id)
            }
            Self::TransferList { project_id } => {
                if let Some(p) = project_id {
                    check_id("project_id", p)?;
                }
                Ok(())
            }
            Self::TransferCancel { transfer_id } => check_id("transfer_id", transfer_id),

            Self::PublishUpdate {
                project_id,
                file_paths,
            } => {
                check_id("project_id", project_id)?;
                if file_paths.is_empty() {
                    return Err("file_paths must not be empty".into());
                }
                if file_paths.len() > MAX_PATHS_PER_PUBLISH {
                    return Err(format!(
                        "too many file_paths ({} > {MAX_PATHS_PER_PUBLISH})",
                        file_paths.len()
                    ));
                }
                for p in file_paths {
                    check_path("file_paths[*]", p)?;
                }
                Ok(())
            }

            Self::Subscribe { .. } => Ok(()),
            Self::Unsubscribe { subscription_id } => check_id("subscription_id", subscription_id),

            Self::ConflictList { project_id } => {
                if let Some(p) = project_id {
                    check_id("project_id", p)?;
                }
                Ok(())
            }
            Self::ConflictResolve {
                file_id,
                resolution,
            } => {
                check_id("file_id", file_id)?;
                match resolution.as_str() {
                    "keep_local" | "accept_remote" | "manual_merge" => Ok(()),
                    other => Err(format!("invalid resolution: {other}")),
                }
            }
            Self::ConfigUpdate { .. } => Ok(()),
        }
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn empty_project_id_rejected() {
        let cmd = IpcCommand::ProjectClose {
            project_id: "".into(),
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn whitespace_only_id_rejected() {
        let cmd = IpcCommand::PeerDisconnect {
            peer_id: "   ".into(),
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn long_id_rejected() {
        let cmd = IpcCommand::ProjectClose {
            project_id: "x".repeat(2048),
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn empty_publish_paths_rejected() {
        let cmd = IpcCommand::PublishUpdate {
            project_id: "p".into(),
            file_paths: vec![],
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn happy_path_passes() {
        let cmd = IpcCommand::TransferRequest {
            project_id: "p".into(),
            file_id: "f".into(),
            peer_id: "x".into(),
        };
        cmd.validate().unwrap();
    }
}
