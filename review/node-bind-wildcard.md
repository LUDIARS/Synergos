## 実装内容

未指定 bind IP の別表記が検証をすり抜ける問題を修正する。
既存の `nodeSettings` と `node-config.test.mjs` を再利用し、文字列集合を
Node 標準の `net.BlockList` に置き換えた。追加依存や独自 IP パーサーは不要なため採用しない。
有効アドレス、既存エラー文言、ポート、設定項目は維持する。

関連: `actio:f71f982b-ee28-41bc-a5ab-9ce5ef67a117`

追従: `actio:c985c205-740c-4e76-a4f6-3ee6627fc084`。
既存 PR #3422 の試験失敗への対応として、ローカル main (`3839cfa`) へ
作業ブランチを rebase した。IPC 接続先を試験ごとに分離する既存修正を再利用し、
node-config の実装・テスト・契約述語には変更を加えていない。衝突なし。
追従後の `node --test scripts/site/tests/*.test.mjs` は 12 件すべて成功した。
常駐サービスに影響しうる Rust 統合テストとサービス操作は実施していない。

## 受け入れ条件

C-1 verifyWildcardRejection(addresses): 未指定アドレスの IPv4・IPv6・IPv4 写像の別表記を既存エラーで拒否する

C-2 verifyValidAddresses(addresses): 有効な IPv4・IPv6・IPv4 写像アドレスと既定ポートを保持する

## 検証

`node --test scripts/site/tests/node-config.test.mjs` の 5 件が成功。
追加テストは修正前に `::0` の拒否漏れで失敗することを確認した。
13 の未指定表記と 9 の有効表記を確認し、既存の TOML 生成・IPv6・ポート検証も成功。
Augur の契約集計は C-1、C-2 とも covered、違反なし。
契約計測はテスト側だけで行う。サービス操作、他のテストは実施していない。

## 提出範囲

ローカル main 起点の作業ブランチから 1 件の local PR を提出する。
Revisor の merged 通知まで継続し、セッション自身はマージや main 更新を行わない。
