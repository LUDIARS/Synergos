//! ノード一覧の 1 行と、その行からの操作 (再発行 / 削除)。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError, NodeKind, NodeView};

/// ノード 1 行。削除は誤操作を避けるため 2 段階にする。
#[component]
pub(super) fn NodeRow(
    client: ApiClient,
    node: NodeView,
    on_token: EventHandler<(String, String)>,
    on_removed: EventHandler<String>,
    on_failed: EventHandler<String>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let mut confirming = use_signal(|| false);
    let mut busy = use_signal(|| false);

    let reported = node
        .reported_mesh_ip
        .clone()
        .unwrap_or_else(|| "-".to_string());
    let expected = node.mesh_ip.clone().unwrap_or_else(|| "-".to_string());
    let heartbeat = node
        .last_heartbeat_ms
        .map(|_| "受信済み".to_string())
        .unwrap_or_else(|| "未着".to_string());
    let is_mesh_node = node.kind == NodeKind::MeshNode;

    let reissue = {
        let client = client.clone();
        let node = node.clone();
        move |_| {
            let client = client.clone();
            let node = node.clone();
            busy.set(true);
            spawn(async move {
                let result = client.reissue_connector_token(&node.org_id, &node.id).await;
                busy.set(false);
                match result {
                    Ok(response) => {
                        on_token.call((node.display_name.clone(), response.connector_token))
                    }
                    Err(ApiError::Unauthorized) => on_unauthorized.call(()),
                    Err(err) => on_failed.call(err.message()),
                }
            });
        }
    };

    let remove = {
        let client = client.clone();
        let node = node.clone();
        move |_| {
            let client = client.clone();
            let node = node.clone();
            busy.set(true);
            spawn(async move {
                let result = client.remove_node(&node.org_id, &node.id).await;
                busy.set(false);
                confirming.set(false);
                match result {
                    Ok(_) => on_removed.call(node.display_name.clone()),
                    Err(ApiError::Unauthorized) => on_unauthorized.call(()),
                    Err(err) => on_failed.call(err.message()),
                }
            });
        }
    };

    rsx! {
        tr {
            td {
                div { "{node.display_name}" }
                div { class: "row-sub", "{node.id}" }
            }
            td { "{node.kind.label()}" }
            td { "{node.owner_email}" }
            td { "{reported} / {expected}" }
            td { "{heartbeat}" }
            td { class: "row-actions",
                if is_mesh_node {
                    button {
                        class: "btn btn-small",
                        r#type: "button",
                        disabled: busy(),
                        onclick: reissue,
                        "トークン再発行"
                    }
                }
                if confirming() {
                    button {
                        class: "btn btn-small btn-danger",
                        r#type: "button",
                        disabled: busy(),
                        onclick: remove,
                        "削除を確定"
                    }
                    button {
                        class: "btn btn-small btn-ghost",
                        r#type: "button",
                        onclick: move |_| confirming.set(false),
                        "やめる"
                    }
                } else {
                    button {
                        class: "btn btn-small btn-ghost",
                        r#type: "button",
                        disabled: busy(),
                        onclick: move |_| confirming.set(true),
                        "削除"
                    }
                }
            }
        }
    }
}
