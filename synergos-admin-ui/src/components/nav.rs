//! 画面上部のナビゲーション。

use dioxus::prelude::*;

use crate::screen::{nav_tabs, Screen};

#[component]
pub fn NavBar(
    current: Screen,
    on_navigate: EventHandler<Screen>,
    on_sign_out: EventHandler<()>,
) -> Element {
    rsx! {
        header { class: "nav",
            div { class: "nav-brand", "Synergos 管理コンソール" }
            nav { class: "nav-tabs",
                for tab in nav_tabs() {
                    {
                        let is_current = current.same_tab(&tab);
                        let label = tab.label();
                        let target = tab.clone();
                        rsx! {
                            button {
                                key: "{label}",
                                class: if is_current { "nav-tab nav-tab-active" } else { "nav-tab" },
                                r#type: "button",
                                onclick: move |_| on_navigate.call(target.clone()),
                                "{label}"
                            }
                        }
                    }
                }
            }
            button {
                class: "btn btn-ghost",
                r#type: "button",
                onclick: move |_| on_sign_out.call(()),
                "トークンを破棄"
            }
        }
    }
}
