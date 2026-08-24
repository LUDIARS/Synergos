//! 管理トークン入力画面。
//!
//! 入力値は控えめに検証したうえで、実際に API を 1 本叩いて通ることを確かめてから
//! セッションに保存する (打ち間違いをその場で返す)。

use dioxus::prelude::*;

use crate::api::{ApiClient, ApiError};
use crate::components::ErrorNotice;

#[component]
pub fn Login(on_authenticated: EventHandler<String>) -> Element {
    let mut input = use_signal(String::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut checking = use_signal(|| false);

    let mut submit = move || {
        let value = input().trim().to_string();
        if value.is_empty() {
            error.set(Some("管理トークンを入力してください".to_string()));
            return;
        }
        error.set(None);
        checking.set(true);
        spawn(async move {
            let client = ApiClient::new(value.clone());
            match client.list_orgs().await {
                Ok(_) => {
                    checking.set(false);
                    on_authenticated.call(value);
                }
                Err(ApiError::Unauthorized) => {
                    checking.set(false);
                    error.set(Some(
                        "このトークンでは管理 API に入れません。\
                         synergos-control 起動時の SYNERGOS_CONTROL_ADMIN_TOKEN と\
                         一致しているか確認してください。"
                            .to_string(),
                    ));
                }
                Err(err) => {
                    checking.set(false);
                    error.set(Some(err.message()));
                }
            }
        });
    };

    rsx! {
        section { class: "login",
            h1 { "Synergos 管理コンソール" }
            p { class: "login-lead",
                "synergos-control の管理トークン (SYNERGOS_CONTROL_ADMIN_TOKEN) を入力してください。\
                 値はこのタブの sessionStorage にだけ保持され、タブを閉じると消えます。"
            }
            form {
                class: "login-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    submit();
                },
                label { r#for: "admin-token", "管理トークン" }
                input {
                    id: "admin-token",
                    r#type: "password",
                    autocomplete: "off",
                    placeholder: "SYNERGOS_CONTROL_ADMIN_TOKEN",
                    value: "{input}",
                    oninput: move |event| input.set(event.value()),
                }
                button {
                    class: "btn btn-primary",
                    r#type: "submit",
                    disabled: checking(),
                    if checking() { "確認中..." } else { "開始する" }
                }
            }
            if let Some(message) = error() {
                ErrorNotice { message: message }
            }
        }
    }
}
