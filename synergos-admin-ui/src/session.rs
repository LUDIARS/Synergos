//! 管理トークンのセッション保持。
//!
//! トークンは `sessionStorage` にのみ置く (タブを閉じれば消える)。
//! localStorage は使わない — 共用端末で残り続けるのを避ける。

const TOKEN_KEY: &str = "synergos-admin-token";

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok()?
}

/// 保存済みの管理トークンを読む。
pub fn load_token() -> Option<String> {
    let value = session_storage()?.get_item(TOKEN_KEY).ok()??;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// 管理トークンを保存する。保存できなくても UI は続行できる
/// (その場合はリロードで再入力になるだけ)。
pub fn store_token(token: &str) {
    if let Some(storage) = session_storage() {
        let _ = storage.set_item(TOKEN_KEY, token);
    }
}

/// 保存済みトークンを消す (ログアウト / 401 検出時)。
pub fn clear_token() {
    if let Some(storage) = session_storage() {
        let _ = storage.remove_item(TOKEN_KEY);
    }
}
