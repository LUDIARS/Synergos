# AUTOFIX — 2026-05-13

autofix_count: **0** (本レビューは指摘列挙のみ、 SCM 修正は禁止ルール)

以下は本レビューで検知し、 安全範囲で「自動修正候補」になり得た項目。 実行はしない。

## A-01 (実行しない) `.copied().copied()` 冗長除去

- 位置: `synergos-core/src/peer_bootstrap.rs:228-229`
- 現状:
  ```rust
  out.extend(primary.iter().copied().copied());
  out.extend(secondary.iter().copied().copied());
  ```
- 提案: `out.extend(primary.into_iter().copied());`
- 理由: `iter().copied().copied()` は `&&SocketAddr` → `&SocketAddr` → `SocketAddr` の deref を 2 段使うが、 `into_iter().copied()` で 1 段省ける。 実害はないが clippy::redundant_copied の対象になりうる
- 影響: 動作変化なし

## A-02 (実行しない) `daemon.rs:113` `relay_endpoint_configured` を `false` 固定から `net_config` 参照に

- 位置: `synergos-core/src/daemon.rs:113`
- 現状: `false, // relay endpoint 設定有無 (現状は判定不能、後続 PR で正確化)`
- 提案: `NetConfig` に `relay_endpoint: Option<String>` を新フィールドで追加し、 `is_some()` を渡す
- 理由: コメント自身が「後続 PR」と TODO 化、 本来 capabilities 判定で見るべき値
- 影響: NetConfig の serde shape 変更を伴うので「安全範囲」を超える可能性あり (ルール上自動修正不可)

## A-03 (実行しない) `peer_info_server.rs:444-465` テスト用 HTTP クライアントの重複コードを `mod common` に集約

- 位置: `synergos-core/src/peer_info_server.rs:444-465` (`reqwest_get_json` / `reqwest_get_text`)
- 現状: ファイル内で手書き HTTP/1.1 GET を持っている
- 提案: `synergos-core/tests/common/http.rs` (もしくは dev-dependency の `reqwest`) に集約
- 理由: 同パタンが他テストでも生じる (将来 ResponseFuzz テスト等を追加するときに重複)
- 影響: refactor only、 機能変化なし

## A-04 (実行しない) `peer_info_server.rs:46` docstring 整備

- 位置: `synergos_version` フィールドの docstring
- 内容: 現状の説明で十分だが、 古いピアの fallback 値が `""` であることを `#[serde(default)]` と一緒に書いておくと、 ユーザ視点で `unknown` 表示の挙動 (`daemon.rs:373`) が読み解きやすい
- 影響: doc only

## A-05 (実行しない) docstring 補強: `peer_bootstrap.rs:223-230`

- `prefer_v6 = advertised.is_ipv6()` の意図 (unspecified 時の family preference) をコメントで補強
- 影響: doc only

---

ルール準拠の補足:
- 上記 5 件はいずれも **行わない**。 ユーザの「ソースコード修正禁止」「AUTOFIX は列挙のみ (autofix_count=0)」指示に従う
- これらは別途 PR を起こす場合の「次の候補リスト」として機能する
