//! 作業ツリーと `.synergos/manifest.json` の突合 (`project status` / `publish --all`)。
//!
//! - 走査対象はプロジェクトルート配下の通常ファイル。`.synergos/` `.git/` と
//!   `.synergosignore` 自体は常に除外し、同ファイルの glob (1 行 1 パターン、
//!   `#` はコメント) に一致するパスも除外する。シンボリックリンクは辿らない。
//! - 内容の比較は manifest の size + CRC32。size が違えば CRC は計算しない。
//!
//! 欠落・破損したファイルの再取得は `project checkout` が担う (manifest どおりに
//! 作業ツリーを揃える) ので、ここは「何が違うか」を数えるだけにする。

use std::io;
use std::path::Path;

use synergos_net::config::glob_match;

use crate::manifest::{crc32_of_file, normalize_rel_path, ProjectManifest, META_DIR};

/// 除外パターンを書くファイル名 (プロジェクトルート直下)。
pub const IGNORE_FILE: &str = ".synergosignore";

/// 常に除外するトップレベルの項目。
/// `.synergosignore` はマシン固有の除外設定を他ピアへ漏らさないため同期しない。
const ALWAYS_IGNORED: &[&str] = &[META_DIR, ".git", IGNORE_FILE];

/// 作業ツリーと manifest の差分。パスはすべてルート相対 (`/` 区切り) で昇順。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    /// manifest にあり、内容が違う
    pub modified: Vec<String>,
    /// manifest に無い (未 publish / 未取得の新規)
    pub untracked: Vec<String>,
    /// manifest にあるが作業ツリーに無い
    pub missing: Vec<String>,
    /// manifest と一致している件数
    pub unchanged: usize,
}

impl WorktreeStatus {
    /// `publish --all` で送るべきもの (内容が変わった / 新規)。
    pub fn publishable(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .modified
            .iter()
            .chain(self.untracked.iter())
            .cloned()
            .collect();
        files.sort();
        files
    }

    pub fn is_clean(&self) -> bool {
        self.modified.is_empty() && self.untracked.is_empty() && self.missing.is_empty()
    }
}

/// `.synergosignore` の中身を除外パターン列にする。
pub fn parse_ignore_patterns(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.trim_start_matches('/').to_string())
        .collect()
}

/// ルート相対パスが除外対象か。パターンは `glob_match` (`*` は `/` を跨がない、
/// `**` は跨ぐ) で、パス全体に加えて各上位ディレクトリにも当てる
/// (`build` と書けば `build/a/b.bin` も除外される)。
pub fn is_ignored(rel: &str, patterns: &[String]) -> bool {
    let first = rel.split('/').next().unwrap_or(rel);
    if ALWAYS_IGNORED.contains(&first) {
        return true;
    }
    rel.match_indices('/')
        .map(|(i, _)| i)
        .chain([rel.len()])
        .any(|end| patterns.iter().any(|p| glob_match(p, &rel[..end])))
}

/// ルート配下の対象ファイルをルート相対パスで列挙する (昇順)。
pub async fn scan_worktree(root: &Path) -> io::Result<Vec<String>> {
    let patterns = load_ignore_patterns(root).await?;
    scan_worktree_with_patterns(root, &patterns).await
}

async fn load_ignore_patterns(root: &Path) -> io::Result<Vec<String>> {
    let patterns = match tokio::fs::read_to_string(root.join(IGNORE_FILE)).await {
        Ok(text) => parse_ignore_patterns(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    Ok(patterns)
}

async fn scan_worktree_with_patterns(root: &Path, patterns: &[String]) -> io::Result<Vec<String>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel = normalize_rel_path(rel);
            if is_ignored(&rel, patterns) {
                continue;
            }
            // file_type はリンクを辿らない
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() {
                files.push(rel);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// 作業ツリーの `files` (ルート相対) と manifest を突合する。
pub async fn compare_with_manifest(
    root: &Path,
    manifest: &ProjectManifest,
    files: &[String],
) -> io::Result<WorktreeStatus> {
    let mut status = WorktreeStatus::default();
    for rel in files {
        let Some(entry) = manifest.get(rel) else {
            status.untracked.push(rel.clone());
            continue;
        };
        let path = root.join(rel);
        let size = tokio::fs::metadata(&path).await?.len();
        let same = size == entry.size && crc32_of_file(&path).await?.0 == entry.crc;
        if same {
            status.unchanged += 1;
        } else {
            status.modified.push(rel.clone());
        }
    }
    let present: std::collections::BTreeSet<&str> = files.iter().map(String::as_str).collect();
    status.missing = manifest
        .files
        .keys()
        .filter(|rel| !present.contains(rel.as_str()))
        .cloned()
        .collect();
    Ok(status)
}

/// ルートを走査して manifest と突合する (`project status`)。
pub async fn worktree_status(root: &Path, project_id: &str) -> io::Result<WorktreeStatus> {
    let manifest = ProjectManifest::load(root, project_id).await?;
    let patterns = load_ignore_patterns(root).await?;
    let files = scan_worktree_with_patterns(root, &patterns).await?;
    let mut status = compare_with_manifest(root, &manifest, &files).await?;
    // 既に manifest に載っているファイルを後から除外しても Missing として
    // checkout を促さない。除外指定は status / publish --all の両方に適用する。
    status.missing.retain(|rel| !is_ignored(rel, &patterns));
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestEntry;

    fn entry(size: u64, crc: u32) -> ManifestEntry {
        ManifestEntry {
            version: 1,
            size,
            crc,
            updated_at: 0,
            publisher: String::new(),
        }
    }

    #[test]
    fn ignore_file_parsing_skips_comments_and_blank_lines() {
        let p = parse_ignore_patterns("# c\n\n/build\n*.tmp \n");
        assert_eq!(p, vec!["build".to_string(), "*.tmp".to_string()]);
    }

    #[test]
    fn metadata_and_git_dirs_are_always_ignored() {
        assert!(is_ignored(".synergos/manifest.json", &[]));
        assert!(is_ignored(".git/HEAD", &[]));
        assert!(is_ignored(IGNORE_FILE, &[]));
        assert!(!is_ignored("src/.gitkeep", &[]));
    }

    #[test]
    fn directory_pattern_excludes_everything_below() {
        let p = vec!["build".to_string()];
        assert!(is_ignored("build/a/b.bin", &p));
        assert!(!is_ignored("src/build.rs", &p));
    }

    #[test]
    fn star_pattern_matches_within_one_segment() {
        let p = vec!["*.tmp".to_string(), "cache/**".to_string()];
        assert!(is_ignored("a.tmp", &p));
        assert!(!is_ignored("sub/a.tmp", &p));
        assert!(is_ignored("cache/x/y.bin", &p));
    }

    #[test]
    fn publishable_is_sorted_union_of_modified_and_untracked() {
        let s = WorktreeStatus {
            modified: vec!["b".into()],
            untracked: vec!["a".into(), "c".into()],
            ..Default::default()
        };
        assert_eq!(s.publishable(), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn status_classifies_modified_untracked_missing_and_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        tokio::fs::create_dir_all(root.join("sub")).await.unwrap();
        tokio::fs::create_dir_all(root.join(".synergos"))
            .await
            .unwrap();
        tokio::fs::write(root.join("same.txt"), b"same")
            .await
            .unwrap();
        tokio::fs::write(root.join("sub/changed.txt"), b"new")
            .await
            .unwrap();
        tokio::fs::write(root.join("fresh.txt"), b"fresh")
            .await
            .unwrap();
        tokio::fs::write(root.join("skip.tmp"), b"x").await.unwrap();
        tokio::fs::write(root.join(IGNORE_FILE), b"*.tmp\n")
            .await
            .unwrap();

        let (same_crc, same_size) = crc32_of_file(&root.join("same.txt")).await.unwrap();
        let mut manifest = ProjectManifest::new("p");
        manifest
            .files
            .insert("same.txt".into(), entry(same_size, same_crc));
        manifest
            .files
            .insert("sub/changed.txt".into(), entry(3, 0xdead_beef));
        manifest.files.insert("gone.txt".into(), entry(1, 1));

        let files = scan_worktree(root).await.unwrap();
        assert_eq!(files, vec!["fresh.txt", "same.txt", "sub/changed.txt"]);
        let status = compare_with_manifest(root, &manifest, &files)
            .await
            .unwrap();
        assert_eq!(status.unchanged, 1);
        assert_eq!(status.modified, vec!["sub/changed.txt"]);
        assert_eq!(status.untracked, vec!["fresh.txt"]);
        assert_eq!(status.missing, vec!["gone.txt"]);
    }

    #[tokio::test]
    async fn ignored_manifest_file_is_not_reported_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        tokio::fs::create_dir_all(root.join(".synergos"))
            .await
            .unwrap();
        tokio::fs::write(root.join(IGNORE_FILE), b"private/**\n")
            .await
            .unwrap();

        let mut manifest = ProjectManifest::new("p");
        manifest
            .files
            .insert("private/secret.txt".into(), entry(1, 1));
        manifest.save(root).await.unwrap();

        let status = worktree_status(root, "p").await.unwrap();
        assert!(status.is_clean());
    }
}
