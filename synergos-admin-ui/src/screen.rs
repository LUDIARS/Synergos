//! 画面の識別子。
//!
//! ルーターは使わず、単一の `Signal<Screen>` で切り替える。
//! `/ui/` 配下という部分パスで配信されるため、history API の base path 設定に
//! 依存しない方が壊れにくい。

/// 表示中の画面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// 組織・ノードの概況。
    Dashboard,
    /// 組織/ノード管理。
    Nodes,
    /// Cloudflare API トークンを渡す Mesh 自動設定。
    MeshSetup,
    /// セットアップガイド。`section` が指定されていればその節を開く。
    Guide { section: Option<String> },
}

impl Screen {
    /// ナビゲーションの見出し。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Dashboard => "ダッシュボード",
            Self::Nodes => "組織 / ノード管理",
            Self::MeshSetup => "Mesh 自動設定",
            Self::Guide { .. } => "セットアップガイド",
        }
    }

    /// ナビゲーションのタブとして同一視するか (ガイドは節が違っても同じタブ)。
    pub fn same_tab(&self, other: &Screen) -> bool {
        matches!(
            (self, other),
            (Self::Dashboard, Self::Dashboard)
                | (Self::Nodes, Self::Nodes)
                | (Self::MeshSetup, Self::MeshSetup)
                | (Self::Guide { .. }, Self::Guide { .. })
        )
    }

    /// ガイドの特定節へ飛ぶ画面。
    pub fn guide(section: &str) -> Self {
        Self::Guide {
            section: Some(section.to_string()),
        }
    }
}

/// ナビゲーションに並べるタブ。
pub fn nav_tabs() -> Vec<Screen> {
    vec![
        Screen::Dashboard,
        Screen::Nodes,
        Screen::MeshSetup,
        Screen::Guide { section: None },
    ]
}
