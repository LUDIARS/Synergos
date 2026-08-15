//! プロジェクトマニフェスト — プロジェクトルート直下 `.synergos/manifest.json` に
//! 「どのファイルが今どのバージョン (内容) か」を永続化する。
//!
//! 役割:
//! - publish 側: ファイルごとの **単調増加バージョン** を発番する (再起動しても続きから)。
//!   同じ内容 (CRC 一致) を再 publish してもバージョンは進めない (冪等)。
//! - 受信側: 受け取ったバージョンを記録し、再起動後も「もう持っている」を判定できる。
//! - 両側: daemon 起動時に `Exchange` の shared_files を復元する材料になる
//!   (publisher が再起動しても FileWant に応答できる)。
//!
//! バージョン管理との関係 (docs/versioning-design.md): このファイルが
//! 「Synergos が同期している資産集合のロックファイル」。git にコミットすれば
//! そのコミット時点の資産集合 (path → version/size/crc) が固定される。
//! Synergos 自体は履歴 (旧バージョンの内容) を持たない。
//!
//! 書き込みは同一ディレクトリの一時ファイルへ書いて rename する (原子的、
//! 別ボリューム rename 不可問題を避けるため tmp は必ず同じディレクトリに置く)。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// プロジェクトルート配下の Synergos メタデータディレクトリ名。
pub const META_DIR: &str = ".synergos";
/// マニフェストのファイル名 (META_DIR 配下)。
pub const MANIFEST_FILE: &str = "manifest.json";
/// 受信中ファイルの一時ディレクトリ名 (META_DIR 配下)。
pub const INCOMING_DIR: &str = "incoming";

/// 1 ファイル分のエントリ。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    /// 単調増加バージョン。publish ごとに +1 (内容が変わったときだけ)。
    pub version: u64,
    pub size: u64,
    /// 内容の CRC32 (publish 時 / 受信後に計算)。
    pub crc: u32,
    /// 最終更新 (epoch ms)。
    pub updated_at: u64,
    /// このバージョンを publish したピア (受信側では送信元)。
    #[serde(default)]
    pub publisher: String,
}

/// マニフェスト本体。キーはプロジェクトルート相対パス (区切りは常に `/`)。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectManifest {
    #[serde(default = "default_format")]
    pub format: u32,
    pub project_id: String,
    #[serde(default)]
    pub files: BTreeMap<String, ManifestEntry>,
}

fn default_format() -> u32 {
    1
}

/// bump の結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpOutcome {
    /// 初回登録 or 内容が変わった → 新バージョン
    Bumped(u64),
    /// 内容 (CRC+size) が前回と同じ → バージョン据え置き
    Unchanged(u64),
}

impl BumpOutcome {
    pub fn version(self) -> u64 {
        match self {
            BumpOutcome::Bumped(v) | BumpOutcome::Unchanged(v) => v,
        }
    }
}

impl ProjectManifest {
    pub fn new(project_id: &str) -> Self {
        Self {
            format: 1,
            project_id: project_id.to_string(),
            files: BTreeMap::new(),
        }
    }

    /// 受信一時ディレクトリ `<root>/.synergos/incoming` (無ければ作る)。
    pub async fn incoming_dir(root: &Path) -> std::io::Result<PathBuf> {
        let metadata_dir = prepare_metadata_dir(root).await?;
        let dir = metadata_dir.join(INCOMING_DIR);
        match tokio::fs::symlink_metadata(&dir).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    ".synergos/incoming must be a real directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        tokio::fs::create_dir_all(&dir).await?;
        let canonical = tokio::fs::canonicalize(&dir).await?;
        if !canonical.starts_with(&metadata_dir) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                ".synergos/incoming resolves outside the metadata directory",
            ));
        }
        Ok(canonical)
    }

    /// 読み込み。無ければ空のマニフェスト。壊れていれば Err。
    pub async fn load(root: &Path, project_id: &str) -> std::io::Result<Self> {
        let path = safe_metadata_path(root, MANIFEST_FILE)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let mut m: ProjectManifest = serde_json::from_slice(&bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                if m.project_id.is_empty() {
                    m.project_id = project_id.to_string();
                }
                if m.format != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unsupported manifest format: {}", m.format),
                    ));
                }
                if m.project_id != project_id {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "manifest project id does not match the open project",
                    ));
                }
                if m.files
                    .values()
                    .any(|entry| entry.version == 0 || entry.version == u64::MAX)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "manifest contains an invalid file version",
                    ));
                }
                Ok(m)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new(project_id)),
            Err(e) => Err(e),
        }
    }

    /// 原子的に保存 (同一ディレクトリ tmp → rename)。
    pub async fn save(&self, root: &Path) -> std::io::Result<()> {
        let metadata_dir = prepare_metadata_dir(root).await?;
        let path = metadata_dir.join(MANIFEST_FILE);
        let tmp = metadata_dir.join(format!("manifest-{}.tmp", uuid::Uuid::new_v4()));
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::fs::write(&tmp, json).await?;
        if let Err(error) = replace_file_atomically(&tmp, &path).await {
            // The old manifest is preserved by replace_file_atomically. The unique
            // temporary file is no longer useful after the failed save.
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
        Ok(())
    }

    /// publish 時のバージョン発番。内容が同じなら据え置き。
    pub fn bump(
        &mut self,
        rel: &str,
        size: u64,
        crc: u32,
        publisher: &str,
        now_ms: u64,
    ) -> BumpOutcome {
        match self.files.get_mut(rel) {
            Some(e) if e.size == size && e.crc == crc => BumpOutcome::Unchanged(e.version),
            Some(e) => {
                e.version += 1;
                e.size = size;
                e.crc = crc;
                e.updated_at = now_ms;
                e.publisher = publisher.to_string();
                BumpOutcome::Bumped(e.version)
            }
            None => {
                self.files.insert(
                    rel.to_string(),
                    ManifestEntry {
                        version: 1,
                        size,
                        crc,
                        updated_at: now_ms,
                        publisher: publisher.to_string(),
                    },
                );
                BumpOutcome::Bumped(1)
            }
        }
    }

    /// 受信完了時の記録。手元より新しい (or 未知の) バージョンだけ上書きする。
    /// 上書きしたら true。
    pub fn record_received(
        &mut self,
        rel: &str,
        version: u64,
        size: u64,
        crc: u32,
        publisher: &str,
        now_ms: u64,
    ) -> bool {
        match self.files.get(rel) {
            Some(e) if e.version >= version => false,
            _ => {
                self.files.insert(
                    rel.to_string(),
                    ManifestEntry {
                        version,
                        size,
                        crc,
                        updated_at: now_ms,
                        publisher: publisher.to_string(),
                    },
                );
                true
            }
        }
    }

    pub fn get(&self, rel: &str) -> Option<&ManifestEntry> {
        self.files.get(rel)
    }
}

/// パス区切りを `/` に正規化する。FileId / マニフェストキーは OS を跨いで
/// 同じ文字列でなければならない (Windows publisher → Linux receiver で
/// `sub\file.txt` という名前のファイルができる事故の防止)。
pub fn normalize_rel_path(rel: &Path) -> String {
    let s = rel.to_string_lossy();
    let s = s.replace('\\', "/");
    s.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// ネットワーク由来の FileId を **プロジェクトルート内に閉じた** 相対パスへ戻す。
/// 以下は None (拒否):
/// - `..` セグメント / 空 / 絶対パス (`/x`, `C:\x`) / `:` 入り
/// - `.synergos/` 配下 (マニフェスト等のメタデータ上書き防止)
/// - セグメントに `\` や制御文字を含む (Windows で別階層に化けるのを防ぐ)
pub fn safe_rel_to_local(rel: &str) -> Option<PathBuf> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.chars().any(char::is_control)
    {
        return None;
    }
    let mut p = PathBuf::new();
    let mut first = true;
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return None;
        }
        if seg.contains(':') || (first && seg == META_DIR) {
            return None;
        }
        first = false;
        p.push(seg);
    }
    Some(p)
}

/// Resolve a network-originated relative path below `root` without traversing an
/// existing symlink or junction. Missing components are allowed so the caller can
/// create ordinary directories, but the caller must resolve again after creation
/// and immediately before replacing the destination.
pub fn safe_join_under_root(root: &Path, rel: &str) -> Option<PathBuf> {
    let local = safe_rel_to_local(rel)?;
    let canonical_root = std::fs::canonicalize(root).ok()?;
    let mut candidate = canonical_root.clone();

    for component in local.components() {
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return None;
                }
                let canonical = std::fs::canonicalize(&candidate).ok()?;
                if !canonical.starts_with(&canonical_root) {
                    return None;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }

    Some(canonical_root.join(local))
}

/// Atomically replace `destination` with `source` while preserving the old file
/// whenever replacement fails. Both paths must be on the same volume.
pub async fn replace_file_atomically(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        tokio::fs::rename(source, destination).await
    }

    #[cfg(windows)]
    {
        let source = source.to_path_buf();
        let destination = destination.to_path_buf();
        tokio::task::spawn_blocking(move || replace_file_windows(&source, &destination))
            .await
            .map_err(|error| io::Error::other(format!("replace task failed: {error}")))?
    }
}

fn safe_metadata_path(root: &Path, file_name: &str) -> io::Result<PathBuf> {
    let canonical_root = std::fs::canonicalize(root)?;
    let metadata_dir = canonical_root.join(META_DIR);
    match std::fs::symlink_metadata(&metadata_dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    ".synergos must be a real directory inside the project root",
                ));
            }
            if !std::fs::canonicalize(&metadata_dir)?.starts_with(&canonical_root) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    ".synergos resolves outside the project root",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(metadata_dir.join(file_name))
}

async fn prepare_metadata_dir(root: &Path) -> io::Result<PathBuf> {
    let path = safe_metadata_path(root, MANIFEST_FILE)?;
    let metadata_dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "manifest has no parent"))?;
    tokio::fs::create_dir_all(metadata_dir).await?;
    let checked = safe_metadata_path(root, MANIFEST_FILE)?;
    checked
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "manifest has no parent"))
}

#[cfg(windows)]
fn replace_file_windows(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// ファイル全体を RAM に載せずに CRC32 を計算する。返り値は (crc, size)。
pub async fn crc32_of_file(path: &Path) -> std::io::Result<(u32, u64)> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hasher.finalize(), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_backslashes_and_leading_dot() {
        assert_eq!(
            normalize_rel_path(Path::new("sub\\dir\\a.bin")),
            "sub/dir/a.bin"
        );
        assert_eq!(normalize_rel_path(Path::new("./a.txt")), "a.txt");
    }

    #[test]
    fn safe_rel_rejects_escapes_and_meta() {
        assert!(safe_rel_to_local("../x").is_none());
        assert!(safe_rel_to_local("a/../x").is_none());
        assert!(safe_rel_to_local("/abs").is_none());
        assert!(safe_rel_to_local("C:/abs").is_none());
        assert!(safe_rel_to_local(".synergos/manifest.json").is_none());
        assert!(safe_rel_to_local("a\\b").is_none());
        assert!(safe_rel_to_local("a/b:stream").is_none());
        assert!(safe_rel_to_local("a/line\nfeed").is_none());
        assert!(safe_rel_to_local("").is_none());
        assert_eq!(
            safe_rel_to_local("a/b.bin"),
            Some(Path::new("a").join("b.bin"))
        );
        assert_eq!(
            safe_rel_to_local(".hidden/x"),
            Some(Path::new(".hidden").join("x"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn safe_join_rejects_symlinked_components() {
        let root =
            std::env::temp_dir().join(format!("synergos-safe-root-{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("synergos-safe-outside-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

        assert!(safe_join_under_root(&root, "linked/payload.bin").is_none());
        assert_eq!(
            safe_join_under_root(&root, "ordinary/payload.bin"),
            Some(
                std::fs::canonicalize(&root)
                    .unwrap()
                    .join("ordinary/payload.bin")
            )
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn bump_is_idempotent_for_same_content() {
        let mut m = ProjectManifest::new("p");
        assert_eq!(
            m.bump("a.bin", 10, 0xAA, "peer-a", 1),
            BumpOutcome::Bumped(1)
        );
        assert_eq!(
            m.bump("a.bin", 10, 0xAA, "peer-a", 2),
            BumpOutcome::Unchanged(1)
        );
        assert_eq!(
            m.bump("a.bin", 11, 0xAB, "peer-a", 3),
            BumpOutcome::Bumped(2)
        );
        assert_eq!(m.get("a.bin").unwrap().version, 2);
    }

    #[test]
    fn record_received_only_moves_forward() {
        let mut m = ProjectManifest::new("p");
        assert!(m.record_received("a.bin", 3, 1, 1, "peer-b", 1));
        assert!(!m.record_received("a.bin", 2, 1, 1, "peer-b", 2));
        assert!(!m.record_received("a.bin", 3, 1, 1, "peer-b", 2));
        assert!(m.record_received("a.bin", 4, 1, 1, "peer-b", 2));
    }

    #[tokio::test]
    async fn save_load_roundtrip_and_crc() {
        let dir = std::env::temp_dir().join(format!("synergos-manifest-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file = dir.join("x.bin");
        tokio::fs::write(&file, b"hello world").await.unwrap();
        let (crc, size) = crc32_of_file(&file).await.unwrap();
        assert_eq!(size, 11);
        assert_eq!(crc, crc32fast::hash(b"hello world"));

        let mut m = ProjectManifest::new("proj");
        m.bump("x.bin", size, crc, "peer-a", 5);
        m.save(&dir).await.unwrap();
        let loaded = ProjectManifest::load(&dir, "proj").await.unwrap();
        assert_eq!(loaded, m);
        // 2 回目の save (既存ファイル上書き) も通る
        m.save(&dir).await.unwrap();
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn failed_replace_preserves_existing_destination() {
        let dir = std::env::temp_dir().join(format!("synergos-replace-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let destination = dir.join("asset.bin");
        tokio::fs::write(&destination, b"last-valid-version")
            .await
            .unwrap();

        let result = replace_file_atomically(&dir.join("missing.part"), &destination).await;
        assert!(result.is_err());
        assert_eq!(
            tokio::fs::read(&destination).await.unwrap(),
            b"last-valid-version"
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
