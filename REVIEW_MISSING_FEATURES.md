# 不足機能評価 (Missing Feature Evaluation)

| 項目 | 値 |
|------|-----|
| リポジトリ | LUDIARS/Synergos |
| 対象ブランチ / PR | claude/review-aiformat-guidelines-zoVBR |
| レビュー実施日 | 2026-04-04 |
| 対象コミット範囲 | 4a47e2f..e9541f4 (全コミット) |

---

## 1. 機能の改善提案 (Feature Improvement)

| 対象機能 | 改善提案 | 期待効果 | 優先度 |
|---------|---------|---------|--------|
| `Conduit::try_connect_routes` | 接続試行を `tokio::select!` で並列化し、最初に成功した経路を使用 | 接続確立の高速化。特にフォールバック時の待ち時間を短縮 | Medium |
| `GossipNode::gc` | `MessageCache::gc` が全エントリをソートしてから 25% を削除。大量メッセージ時に O(n log n) のコスト | `LRU` ベースのキャッシュに置き換え、O(1) での eviction を実現 | Low |
| `CoreConnection` (GUI) | 毎回 `IpcClient::connect()` で新しい接続を作成。2秒ごとのリフレッシュでコネクション数が増大 | 持続接続 + イベント購読ベースのアーキテクチャに変更 | High |
| `CatalogManager::diff_catalog` | チャンク単位でシーケンシャルに比較。大規模プロジェクトではチャンク数が多くボトルネック | 変更チャンクの並列取得、差分比較の並列化 | Medium |
| `RoutingTable::closest` | 全エントリをフラットに集めてソート。ルーティングテーブルが大きい場合に非効率 | bucket index を活用した効率的な最近傍探索 (ターゲットの bucket から順に探索) | Low |
| `Daemon::run` の設定読み込み | `config_path` が指定されても実際には読み込まれない (`// 現時点ではデフォルト設定を使用`) | 設定ファイルの実際の読み込み・パース・適用を実装 | High |

### 観点

- **パフォーマンス最適化**: GUI の IPC 接続管理、GossipNode のキャッシュ GC
- **ユーザ体験の向上**: 設定ファイルの実際の読み込み、接続確立の並列化
- **テスタビリティの向上**: trait ベースの設計で DI は良好。ただしテストカバレッジは低い
- **運用負荷の軽減**: 設定ホットリロード、メトリクス自動収集

---

## 2. 不足機能の提案 (Missing Feature Proposal)

| 提案機能 | 必要性の根拠 | 実装優先度 | 想定影響範囲 |
|---------|------------|-----------|------------|
| **ピア認証 (mTLS / PSK)** | QUIC 接続で自己署名証明書のみ。ピアの身元検証がなく、なりすまし・MITM のリスク。P2P ネットワークの信頼基盤として必須 | High | `synergos-net/quic`, `synergos-net/conduit` |
| **IPC コマンドの入力バリデーション** | `project_id`, `peer_id`, `file_paths` 等にバリデーションなし。空文字・制御文字・過長入力が処理される | High | `synergos-core/ipc_server`, `synergos-ipc/command` |
| **Windows Named Pipe 実装** | Windows サポートが `// TODO` のスタブ。クロスプラットフォーム対応を謳うプロジェクトとして必須 | High | `synergos-ipc/client`, `synergos-core/ipc_server` |
| **永続化レイヤ** | 全状態がインメモリ。デーモン再起動でプロジェクト設定・ピア情報・転送履歴が消失 | High | `synergos-core/project`, `synergos-core/presence` |
| **メトリクス収集** | 接続数・転送速度・エラー率等のメトリクスが収集されていない。運用監視が不可能 | Medium | 全クレート |
| **WebSocket Relay 実装** | `conduit/mod.rs:379` で `Ok(RouteKind::Relay)` を返すが実際の接続処理は未実装。NAT/FW 環境での接続に必要 | Medium | `synergos-net/conduit`, 新規モジュール |
| **設定ホットリロード** | 設定変更にデーモン再起動が必要。運用中のパラメータ調整が困難 | Medium | `synergos-core/daemon` |
| **レート制限** | IPC コマンドの流量制限なし。悪意ある/バグのあるクライアントがデーモンを過負荷にできる | Medium | `synergos-core/ipc_server` |
| **テストカバレッジの向上** | 既存テストは `routing.rs`, `gossip/node.rs`, `catalog/manager.rs`, `ledger.rs`, `file_chain.rs` のみ。IPC サーバー、Exchange、Presence にテストなし | Medium | 全クレート |
| **mDNS によるローカルピア発見** | LAN 内のピア発見が DHT/Gossipsub のみ。同一 LAN 内のゼロコンフィグ発見が不可能 | Low | `synergos-net`, 新規モジュール |
| **DashMap のサイズ上限** | `connections`, `peers`, `transfers`, `store` 等の DashMap にサイズ上限なし。メモリリークのリスク | Low | 全クレート |

### 観点

- **入力バリデーションの不足** — IPC コマンドのバリデーションなし (High)
- **エラー通知・アラートの欠如** — メトリクス/アラートなし (Medium)
- **監査ログの不足** — tracing はあるがセキュリティ監査向けではない (Medium)
- **ヘルスチェック・死活監視の未実装** — HTTP ヘルスチェックなし (Medium)
- **レート制限の未実装** — IPC/ネットワークともに (Medium)
- **バッチ処理・リトライ機構の不足** — IPC 通信にリトライなし (Low)
- **ドキュメント・API仕様の不足** — DESIGN.md はあるが API ドキュメントなし (Low)

---

## 総合評価

| # | レビュー観点 | 指摘数 | 優先度別内訳 |
|---|------------|--------|------------|
| 1 | 機能改善 | 6 | High: 2 / Medium: 2 / Low: 2 |
| 2 | 不足機能 | 11 | High: 4 / Medium: 5 / Low: 2 |

---
