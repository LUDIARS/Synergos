//! ノード登録フォーム。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError, NodeKind, RegisterNodeRequest, RegisterNodeResponse};

/// ノード登録フォーム。
#[component]
pub(super) fn RegisterForm(
    client: ApiClient,
    org_id: String,
    on_registered: EventHandler<RegisterNodeResponse>,
    on_failed: EventHandler<String>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let mut display_name = use_signal(String::new);
    let mut owner_email = use_signal(String::new);
    let mut kind = use_signal(|| NodeKind::MeshNode);
    let mut peer_id = use_signal(String::new);
    let mut submitting = use_signal(|| false);

    let submit = move |event: FormEvent| {
        event.prevent_default();
        let name = display_name().trim().to_string();
        let email = owner_email().trim().to_string();
        if name.is_empty() || email.is_empty() {
            on_failed.call("表示名と所有者メールを入力してください".to_string());
            return;
        }
        let peer = peer_id().trim().to_string();
        let request = RegisterNodeRequest {
            display_name: name,
            owner_email: email,
            kind: kind(),
            synergos_peer_id: if peer.is_empty() { None } else { Some(peer) },
        };
        let client = client.clone();
        let org_id = org_id.clone();
        submitting.set(true);
        spawn(async move {
            let result = client.register_node(&org_id, &request).await;
            submitting.set(false);
            match result {
                Ok(response) => {
                    display_name.set(String::new());
                    owner_email.set(String::new());
                    peer_id.set(String::new());
                    on_registered.call(response);
                }
                Err(ApiError::Unauthorized) => on_unauthorized.call(()),
                Err(err) => on_failed.call(err.message()),
            }
        });
    };

    rsx! {
        form { class: "register-form", onsubmit: submit,
            div { class: "field",
                label { r#for: "node-name", "表示名" }
                input {
                    id: "node-name",
                    value: "{display_name}",
                    placeholder: "build-server-1",
                    oninput: move |event| display_name.set(event.value()),
                }
            }
            div { class: "field",
                label { r#for: "node-owner", "所有者メール" }
                input {
                    id: "node-owner",
                    r#type: "email",
                    value: "{owner_email}",
                    placeholder: "alice@example.test",
                    oninput: move |event| owner_email.set(event.value()),
                }
            }
            div { class: "field",
                label { r#for: "node-kind", "種別" }
                select {
                    id: "node-kind",
                    value: "{kind().wire_value()}",
                    onchange: move |event| {
                        if let Some(value) = NodeKind::from_wire(&event.value()) {
                            kind.set(value);
                        }
                    },
                    option { value: "{NodeKind::MeshNode.wire_value()}", "{NodeKind::MeshNode.label()}" }
                    option { value: "{NodeKind::ClientDevice.wire_value()}", "{NodeKind::ClientDevice.label()}" }
                }
            }
            div { class: "field",
                label { r#for: "node-peer", "synergos peer_id (任意)" }
                input {
                    id: "node-peer",
                    value: "{peer_id}",
                    placeholder: "分かっている場合のみ",
                    oninput: move |event| peer_id.set(event.value()),
                }
            }
            button {
                class: "btn btn-primary",
                r#type: "submit",
                disabled: submitting(),
                if submitting() { "登録中..." } else { "ノードを登録" }
            }
        }
    }
}
