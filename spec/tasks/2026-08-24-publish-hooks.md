---
task: "publish-hooks"
project: "synergos"
kind: "実装"
created: "2026-08-24"
---

# Synergos ファイルコミット (配信) 時フックスクリプト

## 目的

neco 指示 (2026-08-24) 項目6。ファイルコミット (publish) / 受信時に任意のスクリプト
(CI 起動・オートコンバート等) を実行できるフック機構を git hooks 相当として追加する。

## 実装範囲

### フック本体 (`synergos-core/src/hooks/`)

- `runner.rs` — `HookRunner`。daemon 設定 (`synergos-net::config::HooksConfig`) と
  プロジェクト設定 (`.synergos/hooks.toml`) を束ねてイベント・ファイル一致で実行する。
  - `pre-publish` は同期待ちで非 0 exit / timeout / spawn 失敗を `Err` にして publish を中止する
  - `post-publish` / `post-receive` は `tokio::spawn` するだけで待たない
    (転送・イベントループをブロックしない)
  - 実行環境変数 `SYNERGOS_EVENT` / `SYNERGOS_PROJECT` / `SYNERGOS_FILE` /
    `SYNERGOS_VERSION` / `SYNERGOS_PEER` を渡す
  - シェルは Windows = `cmd /C`、unix = `sh -c`
- `project_file.rs` — `<project root>/.synergos/hooks.toml` の読み込み (`[[hook]]` 配列)
- `wiring.rs` — `Exchange::PostReceiveHook` へ `HookRunner` を束ねる daemon 起動時配線

### 設定 (`synergos-net/src/config.rs`)

- `HooksConfig { allow_project_hooks: bool (既定 false), hooks: Vec<HookDef> }` を
  `NetConfig.hooks` に追加
- `HookDef { event, command, match: Vec<String>, timeout_sec (既定60) }`
- `match` 用に外部 crate 非依存の最小 glob マッチャ (`*` は `/` を跨がない,
  `**` は跨ぐ, `?` は任意 1 文字) を追加

### 配線

- `synergos-core/src/ipc_server.rs`: `PublishUpdate` ハンドラで
  CRC 計算・バージョン発番の前に `pre-publish` を同期実行し、失敗なら publish 全体を
  `IpcResponse::Error` で中止する。`publish_updates` 成功後に `post-publish` を発火
- `synergos-core/src/exchange/mod.rs`: 受信完了処理の末尾に `PostReceiveHook` 型を追加し、
  `post-receive` を発火するフックポイントを用意
- `synergos-core/src/daemon.rs`: 起動時に `HookRunner` を組み立て、
  `ServiceContext.hooks` と `Exchange::attach_post_receive_hook` へ配線
- `synergos-ipc`: `IpcCommand::HooksList` / `HooksRun`、
  `IpcResponse::HooksList` / `HooksRunReport` (DTO: `HookInfoDto` / `HookRunResultDto`)

### CLI (`synergos-core/src/cli_hooks.rs`)

- `synergos-core hooks ls <project>` — 有効なフック一覧
  (定義元 daemon/project の別、`allow_project_hooks=false` による無効化を表示)
- `synergos-core hooks run <project> <event> <file>` — 手動発火 (デバッグ用)

### セキュリティ (opt-in)

`.synergos/hooks.toml` はリポジトリ由来スクリプトの自動実行になるため、
`hooks.allow_project_hooks = true` (既定 false) を明示したノードだけがプロジェクトフックを
実行する。daemon 設定 (`[[hooks.hooks]]`) はノードローカルなので常に有効。

### docs

- `docs/hooks.md` を新設 (2 層定義、イベント表、hooks.toml 形式、環境変数、CLI、
  設定例: PNG 自動変換 / CI キック / pre-publish オートコンバート)
- `docs/README.md` の目次に追記

### テスト

- `synergos-net/src/config.rs`: `HooksConfig` / `HookDef` の既定値・TOML パース・
  glob マッチャの単体テスト
- `synergos-core/src/hooks/runner.rs`: pre-publish 成功/失敗/timeout/match 対象外、
  opt-in 有効/無効、`effective_hooks`、`run_manual` の単体テスト
- `synergos-core/src/hooks/project_file.rs`: `.synergos/hooks.toml` 読み込みの単体テスト
- `synergos-core/tests/ipc_handlers.rs` (e2e): pre-publish 非 0 exit で publish が中止され
  manifest が変わらないこと、opt-in 無効時にプロジェクトフックが実行されないこと、
  `hooks run` の post-receive で env が正しく渡ること、timeout で kill され publish が
  中止されることを検証

## 完了条件

- 3 イベント (`pre-publish` / `post-publish` / `post-receive`) + 2 層定義
  (daemon / project) + opt-in (`allow_project_hooks`) + CLI (`hooks ls` / `hooks run`) を実装
- `docs/hooks.md` を新設し設定例 (PNG 自動変換、CI キック) を記載
- Anatomia verify を PR diff に対して実施し、block 級の gate 失敗が無い
  (`rule_conformance` / `duplication` / `convention_drift` は PASS。
  `spec_linkage` / `coupling_delta` は本リポでは warn 級で verdict を落とさない)
- Revisor local PR として提出済み

## 前提未確定 / 本 PR の範囲外

- 本セッションはユーザ指示によりテスト・サービス起動を実行しない運用のため、
  `cargo test` の実行結果は本 PR では未確認 (レビュー時に別途確認)
