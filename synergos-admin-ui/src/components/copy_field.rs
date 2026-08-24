//! コピー可能な値・コマンドの表示部品。

use dioxus::prelude::*;

use crate::clipboard::copy_to_clipboard;

/// 1 行の値 (トークン等) をコピーボタン付きで表示する。
#[component]
pub fn CopyField(label: String, value: String) -> Element {
    let mut feedback = use_signal(String::new);
    let mut copy_succeeded = use_signal(|| false);
    let to_copy = value.clone();

    rsx! {
        div { class: "copy-field",
            div { class: "copy-field-head",
                span { class: "copy-field-label", "{label}" }
                button {
                    class: "btn btn-small",
                    r#type: "button",
                    onclick: move |_| {
                        let value = to_copy.clone();
                        spawn(async move {
                            match copy_to_clipboard(&value).await {
                                Ok(()) => {
                                    copy_succeeded.set(true);
                                    feedback.set("コピーしました".to_string());
                                }
                                Err(err) => {
                                    copy_succeeded.set(false);
                                    feedback.set(err);
                                }
                            }
                        });
                    },
                    "コピー"
                }
                span {
                    class: if copy_succeeded() {
                        "copy-feedback"
                    } else {
                        "copy-feedback copy-feedback-error"
                    },
                    "{feedback}"
                }
            }
            code { class: "copy-field-value", "{value}" }
        }
    }
}

/// 複数行のコマンドブロック。ガイドと Mesh 自動設定で使う。
#[component]
pub fn CommandBlock(caption: String, body: String) -> Element {
    let mut feedback = use_signal(String::new);
    let mut copy_succeeded = use_signal(|| false);
    let to_copy = body.clone();

    rsx! {
        div { class: "command-block",
            div { class: "command-block-head",
                span { class: "command-caption", "{caption}" }
                button {
                    class: "btn btn-small",
                    r#type: "button",
                    onclick: move |_| {
                        let value = to_copy.clone();
                        spawn(async move {
                            match copy_to_clipboard(&value).await {
                                Ok(()) => {
                                    copy_succeeded.set(true);
                                    feedback.set("コピーしました".to_string());
                                }
                                Err(err) => {
                                    copy_succeeded.set(false);
                                    feedback.set(err);
                                }
                            }
                        });
                    },
                    "コピー"
                }
                span {
                    class: if copy_succeeded() {
                        "copy-feedback"
                    } else {
                        "copy-feedback copy-feedback-error"
                    },
                    "{feedback}"
                }
            }
            pre { class: "command-body", "{body}" }
        }
    }
}
