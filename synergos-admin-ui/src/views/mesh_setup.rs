//! Mesh 自動設定: Cloudflare API トークンを渡して 3 ステップを順に実行する。
//!
//! 入力されたトークンはサーバーに保存されない。UI 側でも sessionStorage には置かず、
//! 実行が終わったら入力欄から消す。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError, ConnectorTokensResponse, MeshContext, Org};
use crate::components::{CopyField, ErrorNotice, HelpLink, InfoNotice, Spinner};
use crate::guide::anchors;
use crate::screen::Screen;

/// 1 ステップの進行状態。
#[derive(Debug, Clone, PartialEq)]
enum StepState {
    Pending,
    Running,
    Done(String),
    Failed(String),
}

impl StepState {
    fn badge(&self) -> &'static str {
        match self {
            Self::Pending => "待機",
            Self::Running => "実行中",
            Self::Done(_) => "完了",
            Self::Failed(_) => "失敗",
        }
    }

    fn class(&self) -> &'static str {
        match self {
            Self::Pending => "step step-pending",
            Self::Running => "step step-running",
            Self::Done(_) => "step step-done",
            Self::Failed(_) => "step step-failed",
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Pending => String::new(),
            Self::Running => "実行中です...".to_string(),
            Self::Done(detail) | Self::Failed(detail) => detail.clone(),
        }
    }
}

const STEP_TITLES: [&str; 3] = [
    "1. Cloudflare API トークンを検証する",
    "2. Cloudflare とレジストリを突合する",
    "3. 各ノードの登録トークンを発行する",
];

#[component]
pub fn MeshSetup(
    client: ApiClient,
    on_navigate: EventHandler<Screen>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let orgs = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.list_orgs().await }
        }
    });

    // どの Cloudflare アカウントへ繋ぐのかをトークン入力前に示す (秘密情報は含まない)。
    let context = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            async move { client.mesh_context().await }
        }
    });

    let mut api_token = use_signal(String::new);
    let mut selected_org = use_signal(|| Option::<String>::None);
    let mut steps = use_signal(|| vec![StepState::Pending, StepState::Pending, StepState::Pending]);
    let mut tokens = use_signal(|| Option::<ConnectorTokensResponse>::None);
    let mut running = use_signal(|| false);
    let mut form_error = use_signal(|| Option::<String>::None);

    // 組織が読めたら先頭を既定選択にする (描画中に signal を書かないよう effect で行う)。
    use_effect(move || {
        if selected_org.peek().is_none() {
            if let Some(Ok(list)) = &*orgs.read() {
                if let Some(first) = list.first() {
                    selected_org.set(Some(first.id.clone()));
                }
            }
        }
    });

    let start = {
        let client = client.clone();
        move |_| {
            let token = api_token().trim().to_string();
            let Some(org_id) = selected_org() else {
                form_error.set(Some("組織を選択してください".to_string()));
                return;
            };
            if token.is_empty() {
                form_error.set(Some(
                    "Cloudflare API トークンを入力してください".to_string(),
                ));
                return;
            }
            form_error.set(None);
            tokens.set(None);
            steps.set(vec![
                StepState::Pending,
                StepState::Pending,
                StepState::Pending,
            ]);
            running.set(true);

            let client = client.clone();
            spawn(async move {
                run_setup(&client, &token, &org_id, steps, tokens, on_unauthorized).await;
                running.set(false);
                // 成否にかかわらず、処理が終わった秘密値を入力状態へ残さない。
                api_token.set(String::new());
            });
        }
    };

    rsx! {
        div { class: "view",
            h2 {
                "Mesh 自動設定"
                HelpLink { section: anchors::CONTROL_SETUP.to_string(), on_navigate: on_navigate }
            }
            p { class: "panel-lead",
                "Cloudflare API トークンを渡すと、検証 → 突合 → 登録トークン発行 を順に実行します。\
                 入力したトークンはリクエストに載せて使うだけで、サーバーにもブラウザにも保存されません。"
            }

            match &*context.read_unchecked() {
                Some(Ok(ctx)) => rsx! { MeshTarget { context: ctx.clone() } },
                // 接続先の表示に失敗しても自動設定自体は実行できるため、警告に留める。
                Some(Err(err)) => rsx! { ErrorNotice { message: err.message() } },
                None => rsx! {},
            }

            match &*orgs.read_unchecked() {
                None => rsx! { Spinner { label: "組織を読み込んでいます...".to_string() } },
                Some(Err(err)) => rsx! { ErrorNotice { message: err.message() } },
                Some(Ok(list)) if list.is_empty() => rsx! {
                    InfoNotice {
                        message: "組織がありません。先に組織とノードを登録してください。".to_string()
                    }
                },
                Some(Ok(list)) => {
                    let list: Vec<Org> = list.clone();
                    let current = selected_org().unwrap_or_else(|| list[0].id.clone());
                    rsx! {
                        section { class: "panel",
                            div { class: "field",
                                label { r#for: "mesh-org", "対象組織" }
                                select {
                                    id: "mesh-org",
                                    value: "{current}",
                                    onchange: move |event| selected_org.set(Some(event.value())),
                                    for org in list {
                                        option { key: "{org.id}", value: "{org.id}", "{org.name} ({org.id})" }
                                    }
                                }
                            }
                            div { class: "field",
                                label { r#for: "cf-token", "Cloudflare API トークン" }
                                input {
                                    id: "cf-token",
                                    r#type: "password",
                                    autocomplete: "off",
                                    value: "{api_token}",
                                    placeholder: "Cloudflare Tunnel:Edit / Zero Trust:Edit を含むトークン",
                                    oninput: move |event| api_token.set(event.value()),
                                }
                            }
                            button {
                                class: "btn btn-primary",
                                r#type: "button",
                                disabled: running(),
                                onclick: start,
                                if running() { "実行中..." } else { "自動設定を実行" }
                            }
                        }
                    }
                }
            }

            if let Some(message) = form_error() {
                ErrorNotice { message: message }
            }

            section { class: "panel",
                h3 { "進捗" }
                ol { class: "step-list",
                    for (index, title) in STEP_TITLES.iter().enumerate() {
                        {
                            let state = steps()[index].clone();
                            rsx! {
                                li { key: "{title}", class: "{state.class()}",
                                    div { class: "step-title",
                                        span { "{title}" }
                                        span { class: "step-badge", "{state.badge()}" }
                                    }
                                    if !state.detail().is_empty() {
                                        p { class: "step-detail", "{state.detail()}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(result) = tokens() {
                IssuedTokens { result: result, on_navigate: on_navigate }
            }
        }
    }
}

/// 自動設定の接続先 (control の設定値)。
#[component]
fn MeshTarget(context: MeshContext) -> Element {
    rsx! {
        div { class: "mesh-target",
            span { class: "mesh-target-label", "接続先 Cloudflare アカウント" }
            code { "{context.account_id}" }
            span { class: "mesh-target-label", "API" }
            code { "{context.api_base}" }
        }
    }
}

/// 発行された登録トークンの一覧。
#[component]
fn IssuedTokens(result: ConnectorTokensResponse, on_navigate: EventHandler<Screen>) -> Element {
    rsx! {
        section { class: "panel",
            h3 { "発行された登録トークン ({result.issued} 件)" }
            p { class: "secret-warning",
                "トークンはサーバーに保存されません。この画面を離れる前にコピーしてください。"
            }
            for entry in result.entries.clone() {
                div { class: "token-entry", key: "{entry.node_id}",
                    h4 { "{entry.display_name}" }
                    match (entry.connector_token.clone(), entry.skipped_reason.clone()) {
                        (Some(token), _) => rsx! {
                            CopyField { label: "connector_token".to_string(), value: token }
                            if let Some(command) = entry.enroll_command.clone() {
                                CopyField { label: "ノード上で実行".to_string(), value: command }
                            }
                        },
                        (None, Some(reason)) => rsx! { InfoNotice { message: reason } },
                        (None, None) => rsx! { InfoNotice { message: "発行されませんでした".to_string() } },
                    }
                }
            }
            div { class: "next-step",
                span { "次にすること: 各ノードでエンロールし、ファイアウォールを開けます。" }
                button {
                    class: "btn btn-small",
                    r#type: "button",
                    onclick: move |_| on_navigate.call(Screen::guide(anchors::NODE_ENROLL)),
                    "ガイド「5. ノードをエンロールする」を開く"
                }
            }
        }
    }
}

/// 3 ステップを順に実行する。途中で失敗したらそこで止める。
async fn run_setup(
    client: &ApiClient,
    api_token: &str,
    org_id: &str,
    mut steps: Signal<Vec<StepState>>,
    mut tokens: Signal<Option<ConnectorTokensResponse>>,
    on_unauthorized: EventHandler<()>,
) {
    set_step(&mut steps, 0, StepState::Running);
    match client.mesh_check_token(api_token).await {
        Ok(check) => set_step(
            &mut steps,
            0,
            StepState::Done(format!(
                "トークン状態: {} / アカウント {} / 既存 Mesh node {} 件",
                check.token_status, check.account_id, check.mesh_node_count
            )),
        ),
        Err(err) => {
            fail(&mut steps, 0, err, on_unauthorized);
            return;
        }
    }

    set_step(&mut steps, 1, StepState::Running);
    match client.mesh_reconcile(api_token).await {
        Ok(report) => set_step(
            &mut steps,
            1,
            StepState::Done(format!(
                "未登録の Mesh node {} 件 / 組織外の端末 {} 件 / 実体なし {} 件 / Mesh IP 不一致 {} 件",
                report.dark_connectors.len(),
                report.dark_devices.len(),
                report.missing_connectors.len(),
                report.mesh_ip_mismatches.len()
            )),
        ),
        Err(err) => {
            fail(&mut steps, 1, err, on_unauthorized);
            return;
        }
    }

    set_step(&mut steps, 2, StepState::Running);
    match client.mesh_connector_tokens(api_token, org_id).await {
        Ok(result) => {
            set_step(
                &mut steps,
                2,
                StepState::Done(format!(
                    "{} 件のノードに登録トークンを発行しました",
                    result.issued
                )),
            );
            tokens.set(Some(result));
        }
        Err(err) => fail(&mut steps, 2, err, on_unauthorized),
    }
}

fn set_step(steps: &mut Signal<Vec<StepState>>, index: usize, state: StepState) {
    let mut current = steps();
    current[index] = state;
    steps.set(current);
}

fn fail(
    steps: &mut Signal<Vec<StepState>>,
    index: usize,
    err: ApiError,
    on_unauthorized: EventHandler<()>,
) {
    set_step(steps, index, StepState::Failed(err.message()));
    if err == ApiError::Unauthorized {
        on_unauthorized.call(());
    }
}
