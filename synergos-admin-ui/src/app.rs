//! アプリのルート。管理トークンの有無で「ログイン」と「本体」を切り替える。

use dioxus::prelude::*;

use crate::api::ApiClient;
use crate::components::NavBar;
use crate::screen::Screen;
use crate::session;
use crate::views::{Dashboard, Guide, Login, MeshSetup, Nodes};

const STYLES: &str = include_str!("../assets/main.css");

#[component]
pub fn App() -> Element {
    // 起動時に sessionStorage を読む。タブを開き直すと再入力になる。
    let mut token = use_signal(session::load_token);
    let mut screen = use_signal(|| Screen::Dashboard);

    let on_navigate = move |next: Screen| screen.set(next);
    let on_sign_out = move |_| {
        session::clear_token();
        token.set(None);
        screen.set(Screen::Dashboard);
    };

    rsx! {
        style { dangerous_inner_html: "{STYLES}" }
        main { class: "app",
            match token() {
                None => rsx! {
                    Login {
                        on_authenticated: move |value: String| {
                            session::store_token(&value);
                            token.set(Some(value));
                        }
                    }
                },
                Some(value) => {
                    let client = ApiClient::new(value);
                    rsx! {
                        NavBar {
                            current: screen(),
                            on_navigate: on_navigate,
                            on_sign_out: on_sign_out,
                        }
                        section { class: "content",
                            AppBody {
                                client: client,
                                screen: screen(),
                                on_navigate: on_navigate,
                                on_unauthorized: move |_| {
                                    session::clear_token();
                                    token.set(None);
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 画面本体の切り替え。App から状態管理を分けて見通しを保つ。
#[component]
fn AppBody(
    client: ApiClient,
    screen: Screen,
    on_navigate: EventHandler<Screen>,
    on_unauthorized: EventHandler<()>,
) -> Element {
    match screen {
        Screen::Dashboard => rsx! {
            Dashboard { client: client, on_navigate: on_navigate, on_unauthorized: on_unauthorized }
        },
        Screen::Nodes => rsx! {
            Nodes { client: client, on_navigate: on_navigate, on_unauthorized: on_unauthorized }
        },
        Screen::MeshSetup => rsx! {
            MeshSetup { client: client, on_navigate: on_navigate, on_unauthorized: on_unauthorized }
        },
        Screen::Guide { section } => rsx! {
            Guide { initial_section: section }
        },
    }
}
