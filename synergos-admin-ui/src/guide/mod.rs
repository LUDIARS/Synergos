//! アプリ内セットアップガイド。
//!
//! 文面の正本は `docs/getting-started.md` / `docs/two-node-operations.md` /
//! `docs/mesh-operations.md`。ここはそれを操作順に並べ直した UI 用のデータ。

mod content;

pub use content::{anchors, sections};

/// コマンドブロックが対象とする OS。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Mac,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Windows, Os::Linux, Os::Mac];

    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Mac => "macOS",
        }
    }
}

/// コピー可能なコマンドブロック。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// None なら OS を問わず表示する。
    pub os: Option<Os>,
    pub caption: &'static str,
    pub body: &'static str,
}

/// ガイドの 1 ステップ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub title: &'static str,
    pub body: &'static str,
    pub commands: &'static [Command],
}

/// ガイドの 1 節。`id` は各画面の「?」ヘルプからのリンク先になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub steps: &'static [Step],
}

/// 指定 id の節。見つからなければ None。
pub fn section_by_id(id: &str) -> Option<&'static Section> {
    sections().iter().find(|s| s.id == id)
}

/// その OS タブで表示すべきコマンドだけを返す。
pub fn commands_for(step: &Step, os: Os) -> Vec<&'static Command> {
    step.commands
        .iter()
        .filter(|c| c.os.is_none() || c.os == Some(os))
        .collect()
}
