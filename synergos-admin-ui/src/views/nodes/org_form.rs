//! 組織作成フォーム。空のレジストリからブラウザだけで初期設定できるようにする。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError, CreateOrgRequest, Org};
use crate::components::ErrorNotice;

#[component]
pub(super) fn OrgForm(
    client: ApiClient,
    on_created: EventHandler<Org>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    let mut org_id = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut members = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut submitting = use_signal(|| false);

    let submit = move |event: FormEvent| {
        event.prevent_default();
        let id = org_id().trim().to_string();
        let display_name = name().trim().to_string();
        if !valid_org_id(&id) {
            error.set(Some(
                "組織IDは64文字以内の小文字英数字とハイフンで入力してください".to_string(),
            ));
            return;
        }
        if display_name.is_empty() {
            error.set(Some("組織名を入力してください".to_string()));
            return;
        }
        let member_list = parse_members(&members());
        if member_list.is_empty() {
            error.set(Some(
                "ノード所有者として使うメンバーを1名以上入力してください".to_string(),
            ));
            return;
        }
        if member_list.iter().any(|member| !member.contains('@')) {
            error.set(Some("メンバーはメールアドレスで入力してください".to_string()));
            return;
        }

        error.set(None);
        submitting.set(true);
        let client = client.clone();
        spawn(async move {
            let request = CreateOrgRequest {
                id,
                name: display_name,
                members: member_list,
            };
            match client.create_org(&request).await {
                Ok(org) => {
                    submitting.set(false);
                    org_id.set(String::new());
                    name.set(String::new());
                    members.set(String::new());
                    on_created.call(org);
                }
                Err(ApiError::Unauthorized) => {
                    submitting.set(false);
                    on_unauthorized.call(());
                }
                Err(err) => {
                    submitting.set(false);
                    error.set(Some(err.message()));
                }
            }
        });
    };

    rsx! {
        section { class: "panel",
            h3 { "組織を作成" }
            form { class: "org-form", onsubmit: submit,
                div { class: "field",
                    label { r#for: "org-id", "組織ID" }
                    input {
                        id: "org-id",
                        value: "{org_id}",
                        placeholder: "acme",
                        oninput: move |event| org_id.set(event.value()),
                    }
                }
                div { class: "field",
                    label { r#for: "org-name", "組織名" }
                    input {
                        id: "org-name",
                        value: "{name}",
                        placeholder: "Acme Corp",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                div { class: "field",
                    label { r#for: "org-members", "メンバー (カンマ区切り、1名以上)" }
                    input {
                        id: "org-members",
                        value: "{members}",
                        placeholder: "alice@example.test, bob@example.test",
                        oninput: move |event| members.set(event.value()),
                    }
                }
                button {
                    class: "btn btn-primary",
                    r#type: "submit",
                    disabled: submitting(),
                    if submitting() { "作成中..." } else { "組織を作成" }
                }
            }
            if let Some(message) = error() {
                ErrorNotice { message: message }
            }
        }
    }
}

fn valid_org_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn parse_members(value: &str) -> Vec<String> {
    let mut members = Vec::new();
    for member in value.split(|ch| ch == ',' || ch == '\n') {
        let member = member.trim().to_ascii_lowercase();
        if !member.is_empty() && !members.contains(&member) {
            members.push(member);
        }
    }
    members
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_input_is_normalized_and_deduplicated() {
        assert_eq!(
            parse_members(" Alice@Example.test, bob@example.test\nalice@example.test "),
            vec!["alice@example.test", "bob@example.test"]
        );
    }

    #[test]
    fn org_id_matches_server_slug_rules() {
        assert!(valid_org_id("acme-2"));
        assert!(!valid_org_id("Acme"));
        assert!(!valid_org_id("acme_2"));
        assert!(!valid_org_id(""));
    }
}
