# Synergos ドキュメント

Synergos のセットアップと運用に必要な情報を段階別にまとめたドキュメント群です。
アーキテクチャや内部設計は上位の [`../DESIGN.md`](../DESIGN.md) / [`../README.md`](../README.md) を参照してください。

## 目次

| ドキュメント | 内容 |
|---|---|
| [getting-started.md](getting-started.md) | ビルド、core daemon / GUI の起動、シャットダウン、最小 2 ノード動作手順 |
| [projects-and-peers.md](projects-and-peers.md) | プロジェクト追加 (open / invite / join) と peer 管理 (list / connect / disconnect) |
| [platforms.md](platforms.md) | Windows / Linux / macOS 対応状況、IPC 経路差分、各 OS の前提条件 |
| [setup-macos.md](setup-macos.md) | macOS 専用セットアップガイド (ビルド、Gatekeeper、launchd 常駐、プロジェクト参加、Mesh 参加、検証チェックリスト) |
| [mesh-two-node-checklist.md](mesh-two-node-checklist.md) | Cloudflare Mesh で AWS Linux + Windows の 2 台を実際に動かす実施チェックリスト (人手の段とコピペの段) |
| [two-node-operations.md](two-node-operations.md) | **別マシン 2 台**で動かす手順 (LAN / Cloudflare Mesh)、firewall、トラブルシューティング |
| [operations-readiness.md](operations-readiness.md) | 運用開始に向けた完成度の棚卸しと残作業 (P0/P1/P2) |
| [versioning-design.md](versioning-design.md) | バージョン管理 (git) との兼ね合い、バイナリ/大容量ファイルの差分の扱い |

## クイックリファレンス

```bash
# 1. daemon 起動 (フォアグラウンド常駐)
./target/release/synergos-core start

# 2. 別ターミナルで GUI (任意)
./target/release/synergos-gui

# 3. プロジェクトを自分で作る
./target/release/synergos-core project open myproj /path/to/dir -n "MyProject"

# 4. 他ノードを招待 (別マシンから join させるには自ノードの /peer-info URL が要る)
./target/release/synergos-core project invite myproj --url http://<このホスト>:7780
# → syn1. で始まるトークンを相手に渡す

# 5. 相手ノード側で参加
./target/release/synergos-core project join <token> /path/to/local

# 6. 状況を見る
./target/release/synergos-core project list
./target/release/synergos-core peer list myproj
./target/release/synergos-core network
```

詳細は各ドキュメントを参照してください。
