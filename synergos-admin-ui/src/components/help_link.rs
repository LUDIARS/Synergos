//! 各画面の主要操作に添える「?」ヘルプ。該当ガイド節へ飛ばす。

use dioxus::prelude::*;

use crate::guide::section_by_id;
use crate::screen::Screen;

#[component]
pub fn HelpLink(section: String, on_navigate: EventHandler<Screen>) -> Element {
    let title = section_by_id(&section)
        .map(|s| format!("ガイド: {}", s.title))
        .unwrap_or_else(|| "セットアップガイドを開く".to_string());
    let target = section.clone();

    rsx! {
        button {
            class: "help-link",
            r#type: "button",
            title: "{title}",
            "aria-label": "{title}",
            onclick: move |_| on_navigate.call(Screen::guide(&target)),
            "?"
        }
    }
}
