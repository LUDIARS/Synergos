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
        /// 自己完結型トークンに埋める、この daemon の `/peer-info` URL
        /// (例 `http://100.96.0.5:7780`)。None なら config
        /// (`peer_info_advertised_url`) → 導出 → 従来型 (同一 daemon 内限定) の順。
        #[serde(default)]
        peer_info_url: Option<String>,
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
    /// 作業ツリーを manifest に合わせる (docs/versioning-design.md §3.4)。
    /// `manifest_path` 省略時は `<root>/.synergos/manifest.json` をディスクから
    /// 読み直す (= `git checkout` 後の状態)。手元と違う版は履歴ノード /
    /// publisher へ FileWant(version) を出す (非同期に届く)。
    ProjectCheckout {
        project_id: String,
        manifest_path: Option<PathBuf>,
    },
    /// 1 ファイルを指定版に戻す (manifest も書き戻す)。
    ProjectRestore {
        project_id: String,
        /// プロジェクトルート相対 (`/` 区切り)
        rel_path: String,
        version: u64,
    },

    // ── 履歴ノード ──
    /// 履歴ノード上の保持版一覧 (このノードが履歴ノードでなければ空)
    HistoryList {
        project_id: String,
        rel_path: Option<String>,
    },
    /// 保持ポリシーを適用する。`purge` なら保管庫を全消去する。
    /// `keep_manifests` (例: git の各リリースタグ時点の manifest) が参照する版は削らない。
    HistoryGc {
        project_id: String,
        purge: bool,
        #[serde(default)]
        keep_manifests: Vec<PathBuf>,
    },

    // ── 外部ストレージローテーション (spec: archive-rotation) ──
    /// `[history.rotation]` の保持ポリシーを外部ストレージへ適用する。
    /// `dry_run` なら候補一覧のみで実際には何もしない。
    HistoryRotate {
        project_id: String,
        #[serde(default)]
        dry_run: bool,
        #[serde(default)]
        keep_manifests: Vec<PathBuf>,
    },
    /// 退避済み版の一覧 (path / version / backend / key / size)。
    HistoryOffloaded {
        project_id: String,
        rel_path: Option<String>,
    },
    /// 退避済みの版を明示的に取り戻す。
    HistoryFetch {
        project_id: String,
        rel_path: String,
        version: u64,
    },

    // ── publish / 受信時フック (docs/hooks.md) ──
    /// 有効なフック一覧 (定義元 daemon/project の別と opt-in 状態を表示)
    HooksList { project_id: String },
    /// 手動発火 (デバッグ用)。`event` は `pre-publish` | `post-publish` | `post-receive`。
    HooksRun {
        project_id: String,
        event: String,
        /// プロジェクトルート相対 (`/` 区切り)
        rel_path: String,
    },

    // ── 版タグ (GC / ローテーション保護) ──
    /// タグを作成/上書きする。ピン集合の指定方法は 3 通り (排他):
    /// - `pins` を明示 (`tag add --file <path> --version N`)
    /// - `manifest_path` を渡す (`tag add --manifest <path>`)
    /// - 両方省略: 作業ツリーの現在 manifest (`.synergos/manifest.json`) をピン
    TagAdd {
        project_id: String,
        name: String,
        manifest_path: Option<PathBuf>,
        #[serde(default)]
        pins: Vec<(String, u64)>,
    },
    /// タグ一覧 (name / created_at / pin 数)。
    TagLs { project_id: String },
    /// 1 タグのピン内容を表示する。
    TagShow { project_id: String, name: String },
    /// タグを削除する (実体は消さない。以後 GC 対象に戻るだけ)。
    TagRm { project_id: String, name: String },

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

    // ── 拡張ストリーム (#peer-stream-extension) ──
    /// 任意 magic の uni-directional QUIC stream を 1 本送る。
    /// upper-layer service (例: Susurrus) がリアルタイムイベントを
    /// peer に届けるためのエスケープハッチ。
    ///
    /// 受信側の magic ディスパッチ:
    /// - 既存 magic (HLO1/DHT1/TXFR/GSP1/BSW1) は既存のハンドラへ
    /// - それ以外は `IpcEvent::PeerStreamReceived` として購読クライアントに配信
    PeerSendStream {
        peer_id: String,
        magic: [u8; 4],
        payload: Vec<u8>,
    },
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

        let check_rel_path = |label: &str, path: &str| -> Result<(), String> {
            check_id(label, path)?;
            if path.starts_with('/')
                || path.contains('\\')
                || path == ".synergos"
                || path.starts_with(".synergos/")
                || path.chars().any(char::is_control)
                || path.split('/').any(|segment| {
                    segment.is_empty()
                        || segment == "."
                        || segment == ".."
                        || segment.contains(':')
                })
            {
                return Err(format!(
                    "{label} must be a safe '/'-separated project-relative path"
                ));
            }
            Ok(())
        };

        // タグ名: `[A-Za-z0-9._-]{1,64}` (synergos-core::history::tags::is_valid_tag_name と同じ規則)
        let check_tag_name = |name: &str| -> Result<(), String> {
            let valid = !name.is_empty()
                && name.len() <= 64
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
            if valid {
                Ok(())
            } else {
                Err(format!(
                    "tag name must match [A-Za-z0-9._-]{{1,64}}: {name}"
                ))
            }
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

            Self::ProjectCheckout {
                project_id,
                manifest_path,
            } => {
                check_id("project_id", project_id)?;
                if let Some(p) = manifest_path {
                    check_path("manifest_path", p)?;
                }
                Ok(())
            }
            Self::ProjectRestore {
                project_id,
                rel_path,
                version,
            } => {
                check_id("project_id", project_id)?;
                check_rel_path("rel_path", rel_path)?;
                if *version == 0 || *version == u64::MAX {
                    return Err("version must be a positive file version".into());
                }
                Ok(())
            }
            Self::HistoryList {
                project_id,
                rel_path,
            } => {
                check_id("project_id", project_id)?;
                if let Some(p) = rel_path {
                    check_rel_path("rel_path", p)?;
                }
                Ok(())
            }
            Self::HistoryGc {
                project_id,
                keep_manifests,
                ..
            } => {
                check_id("project_id", project_id)?;
                for p in keep_manifests {
                    check_path("keep_manifests[*]", p)?;
                }
                Ok(())
            }

            Self::HistoryRotate {
                project_id,
                keep_manifests,
                ..
            } => {
                check_id("project_id", project_id)?;
                for p in keep_manifests {
                    check_path("keep_manifests[*]", p)?;
                }
                Ok(())
            }
            Self::HistoryOffloaded {
                project_id,
                rel_path,
            } => {
                check_id("project_id", project_id)?;
                if let Some(p) = rel_path {
                    check_rel_path("rel_path", p)?;
                }
                Ok(())
            }
            Self::HistoryFetch {
                project_id,
                rel_path,
                version,
            } => {
                check_id("project_id", project_id)?;
                check_rel_path("rel_path", rel_path)?;
                if *version == 0 || *version == u64::MAX {
                    return Err("version must be a positive file version".into());
                }
                Ok(())
            }

            Self::HooksList { project_id } => check_id("project_id", project_id),
            Self::HooksRun {
                project_id,
                event,
                rel_path,
            } => {
                check_id("project_id", project_id)?;
                check_id("event", event)?;
                if !matches!(
                    event.as_str(),
                    "pre-publish" | "post-publish" | "post-receive"
                ) {
                    return Err(format!(
                        "event must be one of pre-publish|post-publish|post-receive (got {event})"
                    ));
                }
                check_rel_path("rel_path", rel_path)
            }

            Self::TagAdd {
                project_id,
                name,
                manifest_path,
                pins,
            } => {
                check_id("project_id", project_id)?;
                check_tag_name(name)?;
                if let Some(p) = manifest_path {
                    check_path("manifest_path", p)?;
                }
                if manifest_path.is_some() && !pins.is_empty() {
                    return Err("manifest_path and pins are mutually exclusive".into());
                }
                for (rel, version) in pins {
                    check_rel_path("pins[*].0", rel)?;
                    if *version == 0 || *version == u64::MAX {
                        return Err("pins[*].1 must be a positive file version".into());
                    }
                }
                Ok(())
            }
            Self::TagLs { project_id } => check_id("project_id", project_id),
            Self::TagShow { project_id, name } | Self::TagRm { project_id, name } => {
                check_id("project_id", project_id)?;
                check_tag_name(name)
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

            Self::PeerSendStream {
                peer_id, payload, ..
            } => {
                check_id("peer_id", peer_id)?;
                // payload 上限 1 MiB (DoS 対策、 transport.MAX_MESSAGE_SIZE と整合)
                const MAX_PAYLOAD: usize = 1024 * 1024;
                if payload.len() > MAX_PAYLOAD {
                    return Err(format!(
                        "payload too large ({} > {MAX_PAYLOAD})",
                        payload.len()
                    ));
                }
                Ok(())
            }
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

    #[test]
    fn history_paths_must_stay_project_relative() {
        for rel_path in ["../outside", "/absolute", ".synergos/manifest.json", "a\\b"] {
            let restore = IpcCommand::ProjectRestore {
                project_id: "p".into(),
                rel_path: rel_path.into(),
                version: 1,
            };
            assert!(restore.validate().is_err(), "accepted {rel_path}");

            let list = IpcCommand::HistoryList {
                project_id: "p".into(),
                rel_path: Some(rel_path.into()),
            };
            assert!(list.validate().is_err(), "accepted {rel_path}");
        }
    }

    #[test]
    fn peer_send_stream_oversize_rejected() {
        let cmd = IpcCommand::PeerSendStream {
            peer_id: "p".into(),
            magic: *b"SUM1",
            payload: vec![0u8; 2 * 1024 * 1024], // 2 MiB
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn peer_send_stream_within_limit_passes() {
        let cmd = IpcCommand::PeerSendStream {
            peer_id: "p".into(),
            magic: *b"SUM1",
            payload: vec![0u8; 1024], // 1 KiB
        };
        cmd.validate().unwrap();
    }

    #[test]
    fn tag_names_are_restricted_to_the_allowed_charset() {
        for bad in ["", "../escape", "a/b", "a\\b", " space", &"x".repeat(65)] {
            let add = IpcCommand::TagAdd {
                project_id: "p".into(),
                name: bad.into(),
                manifest_path: None,
                pins: Vec::new(),
            };
            assert!(add.validate().is_err(), "accepted {bad}");

            let show = IpcCommand::TagShow {
                project_id: "p".into(),
                name: bad.into(),
            };
            assert!(show.validate().is_err(), "accepted {bad}");
        }
        let ok = IpcCommand::TagAdd {
            project_id: "p".into(),
            name: "release-1.0".into(),
            manifest_path: None,
            pins: Vec::new(),
        };
        ok.validate().unwrap();
    }

    #[test]
    fn tag_add_rejects_manifest_and_pins_together() {
        let cmd = IpcCommand::TagAdd {
            project_id: "p".into(),
            name: "t".into(),
            manifest_path: Some(PathBuf::from("m.json")),
            pins: vec![("a.bin".into(), 1)],
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn tag_add_rejects_non_positive_pin_versions() {
        let cmd = IpcCommand::TagAdd {
            project_id: "p".into(),
            name: "t".into(),
            manifest_path: None,
            pins: vec![("a.bin".into(), 0)],
        };
        assert!(cmd.validate().is_err());
    }

    #[test]
    fn tag_add_rejects_unsafe_pin_paths() {
        let cmd = IpcCommand::TagAdd {
            project_id: "p".into(),
            name: "t".into(),
            manifest_path: None,
            pins: vec![("../escape".into(), 1)],
        };
        assert!(cmd.validate().is_err());
    }
}
