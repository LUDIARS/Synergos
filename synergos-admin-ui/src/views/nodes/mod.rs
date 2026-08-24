//! 組織 / ノード管理: 一覧・登録・トークン再発行・削除。

mod node_row;
mod org_form;
mod register_form;
mod registered_secrets;

use node_row::NodeRow;
use org_form::OrgForm;
use register_form::RegisterForm;
use registered_secrets::RegisteredSecrets;

use dioxus::prelude::*;

use crate::api::{ApiClient, Org, RegisterNodeResponse};
use crate::components::{ErrorNotice, HelpLink, InfoNotice, SecretPanel, Spinner};
use crate::guide::anchors;
use crate::screen::Screen;

#[component]
pub fn Nodes(
    client: ApiClient,
    on_navigate: EventHandler<Screen>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let mut org_refresh = use_signal(|| 0u32);
    let orgs = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let _ = org_refresh();
            async move { client.list_orgs().await }
        }
    });
    let mut selected_org = use_signal(|| Option::<String>::None);

    rsx! {
        div { class: "view",
            h2 {
                "組織 / ノード管理"
                HelpLink { section: anchors::NODE_REGISTER.to_string(), on_navigate: on_navigate }
            }

            OrgForm {
                client: client.clone(),
                on_created: move |org: Org| {
                    selected_org.set(Some(org.id));
                    org_refresh += 1;
                },
                on_unauthorized: on_unauthorized,
            }

            match &*orgs.read_unchecked() {
                None => rsx! { Spinner { label: "組織を読み込んでいます...".to_string() } },
                Some(Err(err)) => rsx! { ErrorNotice { message: err.message() } },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    InfoNotice {
                        message: "組織がありません。先に組織を作成してください \
                                  (ガイドの「3. 組織とメンバーを作る」)。".to_string()
                    }
                },
                Some(Ok(list)) => {
                    let list = list.clone();
                    let current = selected_org().unwrap_or_else(|| list[0].id.clone());
                    let org = list
                        .iter()
                        .find(|o| o.id == current)
                        .cloned()
                        .unwrap_or_else(|| list[0].clone());
                    rsx! {
                        OrgPicker {
                            orgs: list.clone(),
                            current: current.clone(),
                            on_select: move |id: String| selected_org.set(Some(id)),
                        }
                        OrgNodes {
                            client: client.clone(),
                            org: org,
                            on_navigate: on_navigate,
                            on_unauthorized: on_unauthorized,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn OrgPicker(orgs: Vec<Org>, current: String, on_select: EventHandler<String>) -> Element {
    rsx! {
        div { class: "org-picker",
            label { r#for: "org-select", "組織" }
            select {
                id: "org-select",
                value: "{current}",
                onchange: move |event| on_select.call(event.value()),
                for org in orgs {
                    option { key: "{org.id}", value: "{org.id}", "{org.name} ({org.id})" }
                }
            }
        }
    }
}

/// 選択中の組織のノード一覧 + 登録フォーム。
#[component]
fn OrgNodes(
    client: ApiClient,
    org: Org,
    on_navigate: EventHandler<Screen>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let org_id = org.id.clone();
    // 登録・削除・再発行のたびに増やして一覧を引き直す。
    let mut refresh = use_signal(|| 0u32);
    let nodes = use_resource({
        let client = client.clone();
        let org_id = org_id.clone();
        move || {
            let client = client.clone();
            let org_id = org_id.clone();
            // refresh を読むことで依存に入り、値が変わると再取得される。
            let _ = refresh();
            async move { client.list_nodes(&org_id).await }
        }
    });

    let mut registered = use_signal(|| Option::<RegisterNodeResponse>::None);
    let mut action_error = use_signal(|| Option::<String>::None);
    let mut action_notice = use_signal(|| Option::<String>::None);
    let mut reissued_token = use_signal(|| Option::<(String, String)>::None);

    rsx! {
        section { class: "panel",
            h3 {
                "ノード登録"
                HelpLink { section: anchors::NODE_REGISTER.to_string(), on_navigate: on_navigate }
            }
            p { class: "panel-lead",
                "所有者は組織 {org.name} のメンバー ({org.members.len()} 名) である必要があります。"
            }
            RegisterForm {
                client: client.clone(),
                org_id: org_id.clone(),
                on_registered: move |response: RegisterNodeResponse| {
                    registered.set(Some(response));
                    action_error.set(None);
                    refresh += 1;
                },
                on_failed: move |message: String| action_error.set(Some(message)),
                on_unauthorized: on_unauthorized,
            }
        }

        if let Some(response) = registered() {
            RegisteredSecrets { response: response, on_navigate: on_navigate }
        }

        section { class: "panel",
            h3 { "ノード一覧" }

            if let Some(message) = action_error() {
                ErrorNotice { message: message }
            }
            if let Some(message) = action_notice() {
                InfoNotice { message: message }
            }
            if let Some((name, token)) = reissued_token() {
                SecretPanel {
                    title: format!("{name} の登録トークンを再発行しました"),
                    secrets: vec![("connector_token".to_string(), token)],
                    footer: "ノード上で `sudo warp-cli connector new <token> && sudo warp-cli connect` を実行します。".to_string(),
                }
            }

            match &*nodes.read_unchecked() {
                None => rsx! { Spinner { label: "ノードを読み込んでいます...".to_string() } },
                Some(Err(err)) => rsx! { ErrorNotice { message: err.message() } },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    InfoNotice { message: "この組織にはまだノードがありません。".to_string() }
                },
                Some(Ok(list)) => rsx! {
                    table { class: "node-table",
                        thead {
                            tr {
                                th { "表示名" }
                                th { "種別" }
                                th { "所有者" }
                                th { "Mesh IP (報告 / 期待)" }
                                th { "最終 heartbeat" }
                                th { "操作" }
                            }
                        }
                        tbody {
                            for node in list.clone() {
                                NodeRow {
                                    key: "{node.id}",
                                    client: client.clone(),
                                    node: node,
                                    on_token: move |value: (String, String)| {
                                        reissued_token.set(Some(value));
                                        action_error.set(None);
                                    },
                                    on_removed: move |name: String| {
                                        action_notice.set(Some(format!("{name} を削除しました")));
                                        action_error.set(None);
                                        refresh += 1;
                                    },
                                    on_failed: move |message: String| action_error.set(Some(message)),
                                    on_unauthorized: on_unauthorized,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
