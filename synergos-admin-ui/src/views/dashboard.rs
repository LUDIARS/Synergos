//! ダッシュボード: 組織一覧・ノード数・dark node の概況。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError, NodeView, Org, ReconcileReport};
use crate::components::{ErrorNotice, HelpLink, InfoNotice, Spinner};
use crate::guide::anchors;
use crate::screen::Screen;

/// 1 組織あたりの概況。
#[derive(Debug, Clone, PartialEq)]
struct OrgSummary {
    org: Org,
    nodes: Vec<NodeView>,
}

impl OrgSummary {
    fn mesh_nodes(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.kind == crate::api::NodeKind::MeshNode)
            .count()
    }

    /// heartbeat を一度も受けていないノード数 (未接続の目安)。
    fn silent_nodes(&self) -> usize {
        self.nodes
            .iter()
            .filter(|n| n.last_heartbeat_ms.is_none())
            .count()
    }
}

#[component]
pub fn Dashboard(
    client: ApiClient,
    on_navigate: EventHandler<Screen>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let summaries = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { load_summaries(&client).await }
        }
    });

    let mut report = use_signal(|| Option::<ReconcileReport>::None);
    let mut report_error = use_signal(|| Option::<String>::None);
    let mut checking = use_signal(|| false);

    let run_reconcile = {
        let client = client.clone();
        move |_| {
            let client = client.clone();
            checking.set(true);
            report_error.set(None);
            spawn(async move {
                match client.reconcile().await {
                    Ok(result) => {
                        report.set(Some(result));
                        checking.set(false);
                    }
                    Err(ApiError::Unauthorized) => {
                        checking.set(false);
                        on_unauthorized.call(());
                    }
                    Err(err) => {
                        checking.set(false);
                        report_error.set(Some(err.message()));
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "view",
            h2 { "ダッシュボード" }

            match &*summaries.read_unchecked() {
                None => rsx! { Spinner { label: "組織を読み込んでいます...".to_string() } },
                // 描画中に副作用 (サインアウト) を起こすと再描画が循環するため、
                // ここではメッセージ表示に留める。破棄は操作ハンドラ側で行う。
                Some(Err(err)) => rsx! { ErrorNotice { message: err.message() } },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    InfoNotice {
                        message: "組織がまだありません。まず組織を作成してください \
                                  (ガイドの「3. 組織とメンバーを作る」)。".to_string()
                    }
                },
                Some(Ok(list)) => rsx! {
                    div { class: "card-grid",
                        for summary in list.clone() {
                            div { class: "card", key: "{summary.org.id}",
                                h3 { "{summary.org.name}" }
                                p { class: "card-sub", "org id: {summary.org.id}" }
                                dl { class: "stats",
                                    div { dt { "ノード" } dd { "{summary.nodes.len()}" } }
                                    div { dt { "Mesh node" } dd { "{summary.mesh_nodes()}" } }
                                    div { dt { "heartbeat 未着" } dd { "{summary.silent_nodes()}" } }
                                    div { dt { "メンバー" } dd { "{summary.org.members.len()}" } }
                                }
                            }
                        }
                    }
                },
            }

            section { class: "panel",
                h3 {
                    "dark node の点検"
                    HelpLink { section: anchors::RECONCILE.to_string(), on_navigate: on_navigate }
                }
                p {
                    "Cloudflare 側の実態とレジストリを突合します (レポートのみ。失効は行いません)。\
                     サーバー起動時の CLOUDFLARE_API_TOKEN を使います。"
                }
                button {
                    class: "btn btn-primary",
                    r#type: "button",
                    disabled: checking(),
                    onclick: run_reconcile,
                    if checking() { "点検中..." } else { "dark node を点検" }
                }

                if let Some(message) = report_error() {
                    ErrorNotice { message: message }
                }
                if let Some(result) = report() {
                    ReconcileSummary { report: result }
                }
            }
        }
    }
}

/// 突合結果の要約表示。Mesh 自動設定画面とは表示粒度が違うためここに置く。
#[component]
fn ReconcileSummary(report: ReconcileReport) -> Element {
    rsx! {
        div { class: "reconcile-summary",
            if report.attention_count() == 0 {
                InfoNotice { message: "未登録の参加者・不整合はありません。".to_string() }
            } else {
                dl { class: "stats",
                    div { dt { "dark connector" } dd { "{report.dark_connectors.len()}" } }
                    div { dt { "dark device" } dd { "{report.dark_devices.len()}" } }
                    div { dt { "実体なしノード" } dd { "{report.missing_connectors.len()}" } }
                    div { dt { "Mesh IP 不一致" } dd { "{report.mesh_ip_mismatches.len()}" } }
                }
                ul { class: "detail-list",
                    for connector in report.dark_connectors.clone() {
                        li { key: "c-{connector.id}",
                            "未登録の Mesh node: {connector.name} ({connector.id})"
                        }
                    }
                    for device in report.dark_devices.clone() {
                        li { key: "d-{device.id}",
                            {
                                let email = device.user_email.clone().unwrap_or_else(|| "(メール不明)".to_string());
                                let name = device.name.clone().unwrap_or_else(|| device.id.clone());
                                rsx! { "組織メンバー外の端末: {name} / {email}" }
                            }
                        }
                    }
                    for node in report.missing_connectors.clone() {
                        li { key: "m-{node.node_id}",
                            "Cloudflare 側に実体が無いノード: {node.display_name} ({node.org_id})"
                        }
                    }
                    for mismatch in report.mesh_ip_mismatches.clone() {
                        li { key: "x-{mismatch.node.node_id}",
                            {
                                let reported = mismatch.reported_mesh_ip.clone().unwrap_or_else(|| "(未報告)".to_string());
                                let expected = mismatch.expected_mesh_ip.clone().unwrap_or_else(|| "(未設定)".to_string());
                                rsx! { "Mesh IP 不一致: {mismatch.node.display_name} 報告={reported} 期待={expected}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 組織ごとにノード一覧を引いて概況へまとめる。
async fn load_summaries(client: &ApiClient) -> Result<Vec<OrgSummary>, ApiError> {
    let orgs = client.list_orgs().await?;
    let mut summaries = Vec::with_capacity(orgs.len());
    for org in orgs {
        let nodes = client.list_nodes(&org.id).await?;
        summaries.push(OrgSummary { org, nodes });
    }
    Ok(summaries)
}
