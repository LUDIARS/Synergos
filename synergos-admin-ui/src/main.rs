//! Synergos 管理コンソール (Dioxus 0.7 / WASM)。
//!
//! synergos-control の REST API を fetch で叩くだけの薄いフロントで、
//! サーバー側の API を単一の正に保つ (fullstack server functions は使わない)。

mod api;
mod app;
mod clipboard;
mod components;
mod guide;
mod screen;
mod session;
mod views;

fn main() {
    dioxus::launch(app::App);
}
