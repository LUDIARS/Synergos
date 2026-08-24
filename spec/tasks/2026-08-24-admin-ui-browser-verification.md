---
task: "admin-ui-browser-verification"
project: "synergos"
kind: "検証"
created: "2026-08-24"
---

# 管理コンソール (/ui/) のブラウザ実機確認

## 目的

`synergos-admin-ui` の実装は完了し、ユニットテスト・wasm ビルド・lint はすべて通っている
が、**ブラウザで実際に一連の操作を行った確認が取れていない**。実装セッションはサービス
起動を行わない運用だったため、操作ログ / スクリーンショットが未取得のまま残っている。

これは元タスク (`2026-08-24-admin-ui-dioxus.md`) の完了条件のうち唯一未達の項目であり、
レビュー時に人が実機で埋める必要がある。

## 前提

- PR の対象ブランチをcheckout済みであること
- ビルド手順・配信設定・実環境確認手順は `docs/admin-ui.md` §2 / §3 / §6 に記載済み
- サービス起動は Excubitor 経由でプロジェクト本体フォルダのみ。worktree から起動しない
  (`cc-test` スキル。起動前に Concordia へ testing claim を入れる)

## 作業内容

1. `cargo install dioxus-cli --locked` で `dx` を用意し、`synergos-admin-ui` で
   `dx build --release --platform web` を実行する
   (`dx` は現状この環境に未インストール。CI ジョブは `cargo build --target
   wasm32-unknown-unknown` のみを検証しており、`dx` 経由のバンドルは未検証)
2. `control.toml` に `[ui] dist_path` を設定して `synergos-control serve` を起動する
3. ブラウザで `/ui/` を開き、次の一連を通す
   - 管理トークン入力 → sessionStorage 保持の確認 (タブを閉じると再入力になること)
   - 誤ったトークンでの明確なエラー表示
   - 空のレジストリから組織を作成
   - ノード登録 → `connector_token` / `node_key` / `enroll_hint` の表示とコピー
   - 登録トークン再発行、ノード削除 (2 段階確認)
   - ダッシュボードの「dark node を点検」
   - セットアップガイドの OS タブ切替とコマンドブロックのコピー
   - 各画面の「?」ヘルプからガイド該当節への遷移
4. スクリーンショットまたは操作ログを取得し、Revisor local PR へ添付する

## 完了条件

- 上記 3 の一連がエラーなく通ること
- `dx build` の成果物が `/ui/` から正しく配信されること
  (`base_path = "ui"` により JS / wasm の URL がずれないこと)
- スクリーンショットまたは操作ログが PR に添付されていること
