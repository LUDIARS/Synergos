---
task: "mesh-setup-cloudflare-live-check"
project: "synergos"
kind: "検証"
created: "2026-08-24"
---

# Mesh 自動設定 (/v1/mesh/*) の Cloudflare 実疎通確認

## 目的

管理コンソールの「Mesh 自動設定」から使う request-scoped Cloudflare token API
(`/v1/mesh/context`, `/token-check`, `/reconcile`, `/connector-tokens`) は実装と
ユニットテストを終えているが、**Cloudflare への実疎通は未確認**。実装セッションには
アカウントと API token が無かったため、検証はモックと形式検証に留まっている。

特に `CloudflareClient::verify_token()` は今回新設した `user/tokens/verify` 呼び出しで、
レスポンス形（`TokenStatus`）が実 API と一致するかを実物で確かめていない。

## 前提

- Cloudflare で Account ID と API token が必要
  (Account > Cloudflare Tunnel:Edit, Zero Trust:Edit, Device Posture:Read)
- トークンは env / 設定ファイルへ書かず、UI の入力欄からリクエスト本文で渡す
  (サーバーは保存もログ出力もしない設計)
- 手順の正本は `docs/admin-ui.md` §6、API 表は `docs/mesh-operations.md` §3.4 / §3.6

## 作業内容

1. `/v1/mesh/context` が対象アカウント / API base を返すことを確認する
2. UI の「Mesh 自動設定」で API トークンを入力し、3 ステップを順に通す
   - step 1 `token-check`: `token_status` が `active`、`mesh_node_count` が実数と一致
   - step 2 `reconcile`: 突合レポートが返る (UI からは `revoke_dark` を行わない)
   - step 3 `connector-tokens`: 組織内 Mesh node の登録トークンが発行され、
     ClientDevice / connector 未作成ノードは `skipped_reason` 付きで落ちる
3. `TokenStatus` の実レスポンスと構造体定義がずれていないか確認する
   (ずれていれば `synergos-control/src/cloudflare/mod.rs` を修正)
4. 発行した `connector_token` で実ノードを
   `sudo warp-cli connector new <token> && sudo warp-cli connect` によりエンロールする
5. サーバーログに Cloudflare API トークンが出ていないことを確認する

## 完了条件

- 3 ステップがすべて成功し、UI に進捗と結果が正しく表示されること
- 実ノードのエンロールが成功し、ダッシュボードの dark node 点検で未登録参加者が 0 であること
- ログ・保存ファイルのいずれにも Cloudflare API トークンが残っていないこと
