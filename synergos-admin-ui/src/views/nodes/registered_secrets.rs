//! 登録直後にだけ返る秘密と、次の一手への導線。

use dioxus::prelude::*;

use crate::api::RegisterNodeResponse;
use crate::components::SecretPanel;
use crate::guide::anchors;
use crate::screen::Screen;

/// 登録直後にだけ返る秘密と、次の一手への導線。
#[component]
pub(super) fn RegisteredSecrets(
    response: RegisterNodeResponse,
    on_navigate: EventHandler<Screen>,
) -> Element {
    let mut secrets = vec![("node_key".to_string(), response.node_key.clone())];
    if let Some(token) = response.connector_token.clone() {
        secrets.insert(0, ("connector_token".to_string(), token));
    }
    let hint = response.enroll_hint.clone().unwrap_or_default();

    rsx! {
        SecretPanel {
            title: format!("{} を登録しました", response.node.display_name),
            secrets: secrets,
            footer: hint,
        }
        div { class: "next-step",
            span { "次にすること: ノード側でエンロールし、ファイアウォールを開けます。" }
            button {
                class: "btn btn-small",
                r#type: "button",
                onclick: move |_| on_navigate.call(Screen::guide(anchors::NODE_ENROLL)),
                "ガイド「5. ノードをエンロールする」を開く"
            }
        }
    }
}
