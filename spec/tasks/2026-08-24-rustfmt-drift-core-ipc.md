---
task: "rustfmt-drift-core-ipc"
project: "synergos"
kind: "保守"
created: "2026-08-24"
---

# synergos-core / synergos-ipc の rustfmt ドリフト解消

## 目的

`cargo fmt --all -- --check` をワークスペース全体で走らせると、`feat/admin-ui-dioxus`
で触っていないファイルが整形差分で失敗する。rustc 1.94 の rustfmt が `main` 時点の
コードと異なる整形を出すためで、admin-ui の作業とは無関係な既存ドリフト。

CI の fmt ゲートが動いている以上、放置すると admin-ui とは無関係な PR がすべて赤くなる。
別 PR で一括整形して解消する。

## 対象ファイル (2026-08-24 時点)

- `synergos-core/src/cli_history.rs`
- `synergos-core/src/exchange/mod.rs`
- `synergos-core/src/history/gc.rs`
- `synergos-core/src/history/index.rs`
- `synergos-core/src/history/store.rs`
- `synergos-core/src/ipc_server.rs`
- `synergos-core/src/manifest.rs`
- `synergos-core/src/project.rs`
- `synergos-core/tests/history_node_e2e.rs`
- `synergos-ipc/src/command.rs`

## 作業内容

1. `cargo fmt --all` を実行し、整形のみのコミットを作る (ロジック変更を混ぜない)
2. `cargo test --workspace --all-targets` と
   `cargo clippy --workspace --all-targets -- -D warnings` で回帰が無いことを確認する
3. 他セッションの作業ブランチと衝突しやすい変更なので、単独 PR として短命に扱う

## 完了条件

- `cargo fmt --all -- --check` がワークスペース全体で通ること
- テストと clippy が緑のままであること
- 整形以外の差分が含まれていないこと
