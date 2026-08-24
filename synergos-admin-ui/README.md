# synergos-admin-ui

Synergos 管制サーバー (`synergos-control`) の管理コンソール。Dioxus 0.7 の web renderer で
WASM にビルドし、control の axum が `/ui/` で静的配信する。

- サーバー側の REST API を fetch で叩くだけの薄いフロント。fullstack server functions は
  使わない (API を単一の正に保つ)
- 管理トークンは初回アクセス時に入力し、`sessionStorage` にのみ保持する
- Cloudflare API トークンは Mesh 自動設定の実行中だけ使い、保存しない

ビルド・配信・画面構成は [`docs/admin-ui.md`](../docs/admin-ui.md) を参照。

## workspace から exclude されている理由

`wasm32-unknown-unknown` 専用クレートのため、ルート workspace の member に入れると
ホストターゲットの `cargo build --workspace` / `cargo test --workspace` が壊れる。
CI では専用ジョブ (`Admin UI (wasm)`) が wasm ビルドと clippy を検証する。

## ディレクトリ

| パス | 役割 |
|---|---|
| `src/app.rs` | ルート。トークンの有無で「ログイン」と本体を切り替える |
| `src/screen.rs` | 画面の識別子 (ルーターは使わない) |
| `src/session.rs` | 管理トークンの sessionStorage 保持 |
| `src/api/` | control REST API クライアントと DTO |
| `src/components/` | 画面をまたぐ小部品 (コピー欄・?ヘルプ・通知) |
| `src/views/` | 画面 1 枚 = 1 ファイル |
| `src/guide/` | アプリ内セットアップガイドの本文データ |
