//! 「一度しか返らない秘密」をまとめて見せるパネル。
//!
//! connector_token / node_key はサーバーに保存されないため、
//! 閉じる前にコピーするよう明示する。

use dioxus::prelude::*;

use super::CopyField;

/// (ラベル, 値) の並び。
#[component]
pub fn SecretPanel(title: String, secrets: Vec<(String, String)>, footer: String) -> Element {
    rsx! {
        section { class: "secret-panel",
            h3 { "{title}" }
            p { class: "secret-warning",
                "これらの値はサーバーに保存されません。この画面を閉じる前にコピーしてください \
                 (紛失した場合は再発行できます)。"
            }
            for (label, value) in secrets {
                CopyField { key: "{label}", label: label.clone(), value: value.clone() }
            }
            if !footer.is_empty() {
                p { class: "secret-footer", "{footer}" }
            }
        }
    }
}
