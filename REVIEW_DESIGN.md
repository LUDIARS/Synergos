# 設計レビュー (Design Review)

| 項目 | 値 |
|------|-----|
| リポジトリ | LUDIARS/Synergos |
| 対象ブランチ / PR | claude/review-aiformat-guidelines-zoVBR |
| レビュー実施日 | 2026-04-04 |
| 対象コミット範囲 | 4a47e2f..e9541f4 (全コミット) |

---

## 1. 設計強度 (Design Robustness)

| 評価 | 観点 | 所見 |
|------|------|------|
| B | 障害分離 | 4レイヤアーキテクチャで良好に分離。ただし `SynergosNet` が全サブシステムを直接保持しており、1コンポーネントの障害が他に波及するリスクあり |
| B | 冪等性 | `TransferLedger` の Want/Offer 重複検出は実装済み。`ProjectManager::open_project` も重複チェックあり。ただし `CatalogManager::add_file` は冪等でない |
| C | 入力バリデーション | IPCコマンドの `project_id`, `peer_id`, `file_paths` に対するバリデーションが不足。空文字列・不正なパス・制御文字を含む入力がそのまま処理される |
| B | エラーハンドリング | `thiserror` + `Result` 型で型安全なエラー処理。ただし `let _ = ...` によるエラー黙殺が `daemon.rs:94`, `tunnel/mod.rs:132`, `conduit/mod.rs:266` 等に散見される |
| B | リトライ・タイムアウト設計 | Conduit の Route Migration でフォールバック再接続あり。QUIC に idle_timeout 設定あり。ただし IPC 通信にリトライ/タイムアウトなし |
| B | 状態管理の明確性 | `DashMap` ベースの並行安全な状態管理。`ConnectionState`, `TunnelState`, `TransferState` 等の状態遷移は明確。ただし状態遷移の合法性を強制する仕組みがない |

### チェック項目

- [x] 単一障害点 (SPOF) が存在しないか — `synergos-core` デーモンが SPOF。ただし P2P 設計のため許容範囲
- [x] 外部サービス障害時の縮退動作が定義されているか — Cloudflare Tunnel 不在時のシミュレーションモード (`tunnel/mod.rs:191`) あり
- [ ] **入力値の境界値・異常値に対する防御が十分か — IPC コマンドのバリデーション不足 (後述の修正で対応)**
- [x] エラー発生時にシステムが安全な状態に遷移するか (fail-safe) — Conduit 接続失敗時は `Disconnected` に遷移
- [x] 非同期処理のタイムアウトとキャンセル機構があるか — QUIC idle_timeout、Mesh probe_timeout_ms あり
- [x] 競合状態 (race condition) のリスクが排除されているか — `DashMap` + `RwLock` で適切に排除

---

## 2. 設計思想の一貫性 (Design Philosophy Compliance)

| 該当箇所 | 逸脱内容 | 本来の設計思想 | 推奨修正 |
|----------|---------|--------------|---------|
| `synergos-net/src/lib.rs:1` | `#![allow(dead_code, unused_imports, unused_variables)]` がクレート全体に適用 | コンパイラ警告はコード品質のセーフティネット | 不要な suppress を除去し、個別に `#[allow]` を付与 |
| `synergos-core/src/main.rs:1` | 同上 | 同上 | 同上 |
| `synergos-ipc/src/lib.rs:1` | 同上 | 同上 | 同上 |
| `synergos-core/src/project.rs:38` | `SyncMode::from_str` が `std::str::FromStr` を実装せず固有メソッド | Rust の慣用的な変換パターン | `impl FromStr for SyncMode` として実装 |
| `synergos-net/src/lib.rs:71` | `CatalogManager::new(project_id, 256, 10)` でマジックナンバーを使用 | 設定値は `NetConfig` に集約する設計思想 | `NetConfig` に `catalog` セクションを追加 |
| `synergos-core/src/exchange/mod.rs:188` | `PeerId::new("local")` のハードコード | ピアIDは初期化時に設定 | `Exchange::new` の引数に `local_peer_id` を含めるか、セットアップフローを明確化 |

### チェック項目

- [x] レイヤー間の依存方向が規約通りか — net ← ipc ← core ← gui/plugin の方向で正しい
- [x] 命名規則がプロジェクト全体で統一されているか — Rust 慣習に準拠 (snake_case / CamelCase)
- [x] 共通パターンが一貫して適用されているか — `DashMap` + trait ベースのサービス層が統一
- [x] 既存のユーティリティを無視した再実装がないか — `now_ms()` が 3箇所で重複定義 (`catalog/manager.rs`, `exchange/mod.rs`, `conflict/mod.rs`)
- [x] 責務の配置がアーキテクチャの意図と合致しているか — 概ね良好
- [ ] **設定値のハードコーディングがないか — `CatalogManager` の chunk_max_files=256, chain_max_depth=10 がハードコード**

---

## 3. モジュール分割度 / 機能的凝集度 (Cohesion & Modularity)

| モジュール / クラス | 凝集度評価 | 所見 |
|-------------------|-----------|------|
| `synergos-net` | 機能的 | ネットワーク関連の機能が適切に集約。サブモジュール (conduit, quic, tunnel, mesh, dht, gossip, catalog, chain) の分割も良好 |
| `synergos-ipc` | 機能的 | IPC プロトコル定義に特化。command/response/event/transport の分割が明確 |
| `synergos-core::ipc_server` | 手続き的 | `dispatch_command` 関数が全コマンドのハンドリングを1関数に集約 (460行)。コマンドカテゴリ別にハンドラモジュールを分割すべき |
| `synergos-core::Exchange` | 機能的 | ファイル転送に関する責務が適切に集約 |
| `synergos-core::PresenceService` | 機能的 | ピア状態管理に特化。DHT/Gossipsub との連携も適切 |
| `synergos-core::ConflictManager` | 機能的 | コンフリクト検出・解決・ホットスタンバイが一貫して管理 |
| `SynergosNet` | 通信的 | 全サブシステムへの参照を保持するファサード。責務は薄いが、サブシステム間の結合点として機能 |
| `CoreConnection` (GUI) | 手続き的 | IPC 通信とキャッシュ管理が混在。通信層とキャッシュ層を分離すべき |

### チェック項目

- [x] 1つのクラスが複数の無関係な責務を持っていないか — 概ね良好。`ipc_server::dispatch_command` のみ要改善
- [x] God Object が存在しないか — `SynergosNet` が 9フィールドを持つが、ファサードとして許容範囲
- [x] 結合度が不必要に高くないか — trait ベースの抽象化で結合度を抑制
- [x] 循環依存が発生していないか — Cargo workspace で保証
- [x] インターフェースが適切に分離されているか — `FileSharing`, `NodeRegistry`, `ProjectConfiguration` で分離
- [x] パッケージ構成がドメインの構造を反映しているか — 4レイヤ設計が crate 構成に直接反映

---

## 総合評価

| # | レビュー観点 | 評価 | 重大指摘数 |
|---|------------|------|-----------|
| 1 | 設計強度 | B | 1 (入力バリデーション不足) |
| 2 | 設計思想の一貫性 | B | 2 (全体的な警告抑制、`now_ms()` 重複) |
| 3 | モジュール分割度 | B | 1 (`dispatch_command` の肥大化) |

**評価基準:**
- **A**: 問題なし。ベストプラクティスに準拠
- **B**: 軽微な改善点あり。運用上の影響は低い
- **C**: 改善が必要。リリース前の対応を推奨
- **D**: 重大な問題あり。即時対応が必要
