//! セットアップガイド画面。節の目次 + OS タブ + コピー可能なコマンドブロック。

use dioxus::prelude::*;

use crate::components::CommandBlock;
use crate::guide::{commands_for, sections, Os, Section};

#[component]
pub fn Guide(initial_section: Option<String>) -> Element {
    let all = sections();
    let default_id = initial_section
        .clone()
        .unwrap_or_else(|| all[0].id.to_string());
    let mut current_id = use_signal(|| default_id);
    let mut os = use_signal(|| Os::Windows);

    // 他画面の「?」から別の節を指して来たら追従する。
    use_effect(move || {
        if let Some(section) = initial_section.clone() {
            current_id.set(section);
        }
    });

    let selected = all
        .iter()
        .find(|s| s.id == current_id())
        .copied()
        .unwrap_or(all[0]);

    rsx! {
        div { class: "view guide",
            h2 { "セットアップガイド" }
            p { class: "panel-lead",
                "管制サーバーの起動からノード参加・点検までを順に説明します。\
                 コマンドは OS タブを切り替えると、その OS 向けのものだけが表示されます。"
            }

            div { class: "guide-layout",
                nav { class: "guide-toc",
                    for section in all.iter() {
                        {
                            let id = section.id.to_string();
                            let is_current = section.id == current_id();
                            let title = section.title;
                            rsx! {
                                button {
                                    key: "{id}",
                                    class: if is_current { "toc-item toc-item-active" } else { "toc-item" },
                                    r#type: "button",
                                    onclick: move |_| current_id.set(id.clone()),
                                    "{title}"
                                }
                            }
                        }
                    }
                }

                article { class: "guide-body",
                    div { class: "os-tabs",
                        for option in Os::ALL {
                            button {
                                key: "{option.label()}",
                                class: if os() == option { "os-tab os-tab-active" } else { "os-tab" },
                                r#type: "button",
                                onclick: move |_| os.set(option),
                                "{option.label()}"
                            }
                        }
                    }
                    GuideSection { section: selected, os: os() }
                }
            }
        }
    }
}

#[component]
fn GuideSection(section: Section, os: Os) -> Element {
    rsx! {
        section { class: "guide-section",
            h3 { "{section.title}" }
            p { class: "guide-summary", "{section.summary}" }
            ol { class: "guide-steps",
                for step in section.steps.iter() {
                    li { key: "{step.title}", class: "guide-step",
                        h4 { "{step.title}" }
                        p { "{step.body}" }
                        for command in commands_for(step, os) {
                            CommandBlock {
                                key: "{command.caption}",
                                caption: command.caption.to_string(),
                                body: command.body.to_string(),
                            }
                        }
                    }
                }
            }
        }
    }
}
