use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};

use super::AppState;

/// 管理 Web UI (synergos-admin-ui / Dioxus) の静的配信。
///
/// 配信元は設定 `[ui] dist_path`。未設定なら 503 を返し、ビルド手順を案内する
/// (UI を置かない運用でも API サーバーとしては動く)。
/// 依存を増やさないため ServeDir は使わず、パス正規化を自前で行う。
pub async fn redirect_to_ui() -> Redirect {
    Redirect::permanent("/ui/")
}

pub async fn serve_index(State(state): State<Arc<AppState>>) -> Response {
    serve_relative(&state, "index.html")
}

pub async fn serve_asset(
    State(state): State<Arc<AppState>>,
    UrlPath(request_path): UrlPath<String>,
) -> Response {
    serve_relative(&state, &request_path)
}

fn serve_relative(state: &AppState, request_path: &str) -> Response {
    let Some(dist) = state.ui_dist.as_deref() else {
        return ui_not_configured();
    };
    match read_contained_file(dist, request_path) {
        Ok((target, bytes)) => file_response(&target, bytes),
        Err(AssetReadError::Unsafe) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(AssetReadError::Missing) => {
            // SPA フォールバック: 拡張子の無いパスは index.html を返す。
            // 拡張子付き (= 実アセット要求) の取りこぼしは 404 のままにする。
            if Path::new(request_path).extension().is_some() {
                return (StatusCode::NOT_FOUND, "not found").into_response();
            }
            match read_contained_file(dist, "index.html") {
                Ok((index, bytes)) => file_response(&index, bytes),
                Err(_) => ui_not_built(),
            }
        }
    }
}

/// dist と対象を実パスへ解決し、対象が dist 内に残る場合だけ読み込む。
///
/// 字句的な `..` 検査だけでは dist 内の symlink が外部を指すケースを防げない。
/// canonicalize 後のパスをそのまま読むことで、symlink を介した任意ファイル配信を拒否する。
fn read_contained_file(
    dist: &Path,
    request_path: &str,
) -> Result<(PathBuf, Vec<u8>), AssetReadError> {
    let target = safe_join(dist, request_path).ok_or(AssetReadError::Unsafe)?;
    let canonical_dist = dist.canonicalize().map_err(|_| AssetReadError::Missing)?;
    let canonical_target = target
        .canonicalize()
        .map_err(|_| AssetReadError::Missing)?;
    if !canonical_target.starts_with(&canonical_dist) {
        return Err(AssetReadError::Unsafe);
    }
    if !canonical_target.is_file() {
        return Err(AssetReadError::Missing);
    }
    let bytes = std::fs::read(&canonical_target).map_err(|_| AssetReadError::Missing)?;
    Ok((canonical_target, bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetReadError {
    Missing,
    Unsafe,
}

fn file_response(path: &Path, bytes: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime_for(path)),
    );
    response
}

/// URL パスを dist 配下の実パスへ安全に解決する。
/// `..` や絶対パス・ルート指定を含む要求は解決せずに拒否する (traversal 対策)。
fn safe_join(dist: &Path, request_path: &str) -> Option<PathBuf> {
    // 先頭 `/` は絶対パス指定なので拒否する。axum の `*ui_path` は先頭 `/` を
    // 含まないため、これを弾いても通常の配信要求には影響しない。
    if request_path.starts_with('/') {
        return None;
    }
    let mut resolved = dist.to_path_buf();
    for segment in request_path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        let mut components = Path::new(segment).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(name)), None) => resolved.push(name),
            _ => return None,
        }
    }
    if resolved == dist {
        resolved.push("index.html");
    }
    Some(resolved)
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn ui_not_configured() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "管理 UI が配信設定されていません。control.toml に [ui] dist_path を設定してください \
         (ビルド手順: docs/admin-ui.md)。",
    )
        .into_response()
}

fn ui_not_built() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "管理 UI の配信先に index.html がありません。\
         `dx build --release --platform web` で synergos-admin-ui をビルドしてください \
         (docs/admin-ui.md)。",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_attempts_are_rejected() {
        let dist = Path::new("/srv/ui");
        assert!(safe_join(dist, "../secrets.toml").is_none());
        assert!(safe_join(dist, "assets/../../etc/passwd").is_none());
        assert!(safe_join(dist, "/etc/passwd").is_none());
    }

    #[test]
    fn normal_paths_resolve_under_dist() {
        let dist = Path::new("/srv/ui");
        assert_eq!(
            safe_join(dist, "assets/app.wasm").unwrap(),
            dist.join("assets").join("app.wasm")
        );
        assert_eq!(safe_join(dist, "").unwrap(), dist.join("index.html"));
        // ルート指定 `/` は絶対パス扱いで拒否する。`/ui/` は専用ルート
        // (serve_index) が index.html を返すため、配信には影響しない。
        assert!(safe_join(dist, "/").is_none());
    }

    #[test]
    fn wasm_is_served_with_its_own_mime_type() {
        assert_eq!(mime_for(Path::new("app.wasm")), "application/wasm");
        assert_eq!(
            mime_for(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(mime_for(Path::new("noext")), "application/octet-stream");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_outside_dist_are_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let dist = root.path().join("dist");
        std::fs::create_dir(&dist).unwrap();
        let secret = root.path().join("secret.txt");
        std::fs::write(&secret, "not public").unwrap();
        symlink(&secret, dist.join("leak.txt")).unwrap();

        assert_eq!(
            read_contained_file(&dist, "leak.txt").unwrap_err(),
            AssetReadError::Unsafe
        );
    }
}
