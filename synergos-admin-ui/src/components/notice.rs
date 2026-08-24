//! 状態表示 (エラー / 情報 / 読み込み中)。

use dioxus::prelude::*;

#[component]
pub fn ErrorNotice(message: String) -> Element {
    rsx! {
        div { class: "notice notice-error", role: "alert", "{message}" }
    }
}

#[component]
pub fn InfoNotice(message: String) -> Element {
    rsx! {
        div { class: "notice notice-info", "{message}" }
    }
}

#[component]
pub fn Spinner(label: String) -> Element {
    rsx! {
        div { class: "spinner", "{label}" }
    }
}
