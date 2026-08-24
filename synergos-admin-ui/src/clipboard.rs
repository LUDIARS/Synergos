//! クリップボードコピー。
//!
//! `web-sys` の `Clipboard` は unstable API フラグ配下にあるため、
//! `js_sys::Reflect` で `navigator.clipboard.writeText` を動的に呼ぶ。

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// テキストをクリップボードへ書き込む。
/// 非対応環境 (http 経由など) では `Err` を返し、呼び出し側が手動コピーを促す。
pub async fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("ブラウザ環境ではありません")?;
    let navigator = js_sys::Reflect::get(&window, &JsValue::from_str("navigator"))
        .map_err(|_| "navigator を取得できません".to_string())?;
    let clipboard = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard"))
        .map_err(|_| "clipboard API を取得できません".to_string())?;
    if clipboard.is_undefined() || clipboard.is_null() {
        return Err("このブラウザ/接続ではクリップボード API を使えません".to_string());
    }
    let write_text = js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText"))
        .map_err(|_| "writeText を取得できません".to_string())?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "writeText が関数ではありません".to_string())?;

    let promise = write_text
        .call1(&clipboard, &JsValue::from_str(text))
        .map_err(|_| "クリップボードへの書き込みに失敗しました".to_string())?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "クリップボード API が不正な結果を返しました".to_string())?;
    JsFuture::from(promise)
        .await
        .map_err(|_| "クリップボードへの書き込みが拒否されました".to_string())?;
    Ok(())
}
