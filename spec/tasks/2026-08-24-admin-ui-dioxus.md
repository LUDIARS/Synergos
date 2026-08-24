---
task: "admin-ui-dioxus"
project: "synergos"
kind: "実装"
created: "2026-08-24"
---

# Synergos 管理サーバ Web UI (Dioxus 0.7) + アプリ内手順ガイド + Mesh 自動設定

## 目的

管制サーバー `synergos-control` は組織別ノードレジストリ・Cloudflare Mesh 自動化・
connector-token 発行を **API としては実装済み**だが、操作手段が curl と docs だけだった。
ユーザが触れる Web UI を追加し、(a) 組織/ノードの管理、(b) Cloudflare API トークンを
渡すだけの Mesh 自動設定、(c) アプリ内でのセットアップ手順ガイド表示、をできるようにする。

フレームワークは Dioxus 0.7 (web renderer)。WASM フロントをビルドして synergos-control の
axum から `/ui/` で静的配信する。fullstack server functions は使わず、既存 REST API を
fetch で叩く構成にして API を単一の正に保つ。

## 実装範囲

### 新クレート `synergos-admin-ui` (Dioxus 0.7 / WASM)

- 画面 1 枚 = 1 ファイル (`src/views/`)、共通部品は `src/components/`、
  API クライアントは `src/api/`、ガイド本文は `src/guide/` に分離 (SRP)
- 画面:
  1. **ダッシュボード** — 組織一覧、ノード数 / Mesh node 数 / heartbeat 未着数、
     dark node の点検 (`POST /v1/reconcile`、レポートのみ)
  2. **組織 / ノード管理** — ノード一覧 (種別・所有者・Mesh IP・heartbeat)、
     登録フォーム (connector_token / node_key / enroll_hint をコピーボタン付きで表示)、
     登録トークン再発行、ノード削除 (2 段階確認)
  3. **Mesh 自動設定** — Cloudflare API トークンを入力し、
     「トークン検証 → 突合 → 各ノードの登録トークン発行」をステップ進捗表示で実行
  4. **セットアップガイド** — docs の手順をステップ形式で表示。
     OS タブ (Windows / Linux / macOS) 切替、コピー可能なコマンドブロック
  5. **ガイドの文脈化** — 各画面の主要操作に「?」ヘルプ (該当ガイド節へのリンク)、
     ノード登録直後に「次に何をするか」への導線
- 認証: 初回アクセスで管理トークンを入力 → `sessionStorage` 保持 →
  全 API 呼び出しに Bearer 付与。未設定 / 不一致は明確なエラー表示
- UI 文言は日本語
- wasm32 専用のためルート workspace からは `exclude`。ホストターゲットの
  `cargo build/test --workspace` を壊さない

### `synergos-control` の追加

- `/ui/` の静的配信 (`src/api/ui.rs`)。パス traversal を自前で弾き、
  MIME (特に `application/wasm`) を付ける。SPA フォールバックあり。
  未設定時は 503 + ビルド手順の案内
- 設定 `[ui] dist_path` (省略可)。指定したのにディレクトリが無ければ起動時に落とす
- request-scoped Cloudflare token API (`src/api/mesh_setup.rs`)
  - `GET /v1/mesh/context` — 対象アカウント / API base (秘密情報なし)
  - `POST /v1/mesh/token-check` — トークン検証 + アカウント到達確認
  - `POST /v1/mesh/reconcile` — 同トークンでの突合
  - `POST /v1/mesh/connector-tokens` — 組織内 Mesh node の登録トークン一括発行
  - 受け取ったトークンは**保存もログ出力もしない** (`Debug` を伏字実装にする)。
    HTTP ヘッダへ載せる前に形式検証してヘッダインジェクションを防ぐ
  - すべて管理トークン層の内側 (トークンの持ち込み口を無認証にしない)
  - UI からは `revoke_dark` を行わない (破壊的操作は CLI 経由のまま)
- `CloudflareClient::verify_token()` を追加 (`user/tokens/verify`)
- `reconcile_api::reconcile_with()` を切り出し、env トークンと request-scoped
  トークンで同じ突合ロジックを共有する

### ドメイン宣言

- `spec/domains/admin-ui.domain.json` を新設 (`synergos-admin-ui/` の membership)

### docs / CI

- `docs/admin-ui.md` 新設 (ビルド・配信設定・画面構成・秘密情報の扱い・実環境確認手順)
- `docs/mesh-operations.md` §3.4 API 表に `/v1/mesh/*` を追記、
  §3.6 として UI からの操作手順を追加 (旧 §3.6 は §3.7 へ繰り下げ)
- `docs/README.md` の目次に mesh-operations.md / admin-ui.md を追加
- CI に `Admin UI (wasm)` ジョブを追加 (wasm32 ビルド + fmt + clippy)

## 完了条件

- `synergos-control` に `/ui/` 配信と request-scoped Mesh 自動設定 API が入り、
  追加 API には認証必須を含むテストがある
- `synergos-admin-ui` が Dioxus 0.7 で 4 画面 + ヘルプ導線を実装し、
  `wasm32-unknown-unknown` でビルドできる
- `cargo test --workspace` の対象 (= admin-ui を除くワークスペース) が壊れない。
  `cargo clippy --workspace -D warnings` / `cargo fmt --check` が通る
- CI に wasm ビルドジョブがある
- `docs/mesh-operations.md` に UI からの操作手順が追記されている
- Anatomia verify を PR diff に対して実施し、block 級の gate 失敗が無い
- Revisor local PR として提出済み

## 検証結果 (2026-08-24 実行)

| 検証 | コマンド | 結果 |
|---|---|---|
| ワークスペーステスト | `cargo test --workspace --all-targets` | 全緑 (synergos-control 30 件を含む) |
| control の lint | `cargo clippy -p synergos-control --all-targets -- -D warnings` | 通過 |
| control の整形 | `cargo fmt -p synergos-control -- --check` | 通過 |
| wasm ビルド | `cargo build --target wasm32-unknown-unknown` (synergos-admin-ui) | 成功 |
| wasm lint | `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings` | 通過 |
| wasm 整形 | `cargo fmt --all -- --check` (synergos-admin-ui) | 通過 |
| Anatomia verify | `git diff | anatomia verify --repo <repo> --json` | block 級ゲート (`rule_conformance` / `duplication` / `convention_drift`) は PASS。`spec_linkage` / `coupling_delta` は warn のみ |

初回のテスト実行で `api::ui::tests::traversal_attempts_are_rejected` が 1 件失敗した。
`safe_join` のドキュメントは「絶対パス・ルート指定は拒否する」と宣言しているのに、
先頭 `/` を空セグメントとして読み飛ばして `dist` 配下の相対パスに解決していたため。
テストを緩めるのではなく実装を宣言どおりに直した (先頭 `/` を拒否)。axum の `*ui_path`
は先頭 `/` を含まず、`/ui/` は専用ルートが index.html を返すため配信への影響は無い。

## 前提未確定 / 本 PR の範囲外

- Cloudflare の実疎通確認は行っていない (アカウント / API token が無いため)。
  実環境での確認手順は `docs/admin-ui.md` §6 に記載した
- ブラウザでの操作ログ / スクリーンショットは未取得。
  本セッションはサービス起動を行わない運用のため、実機確認は PR レビュー時に別途行う
- ワークスペース全体の `cargo fmt --all -- --check` は本 PR の対象外ファイル
  (`synergos-core` / `synergos-ipc`) で既存の整形ドリフトにより失敗する。
  rustc 1.94 の rustfmt による main 由来の差分でありスコープ外のため触っていない
