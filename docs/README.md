# Synergos ドキュメント

Synergos のセットアップと運用に必要な情報を段階別にまとめたドキュメント群です。
アーキテクチャや内部設計は上位の [`../DESIGN.md`](../DESIGN.md) / [`../README.md`](../README.md) を参照してください。

## 目次

| ドキュメント | 内容 |
|---|---|
| [getting-started.md](getting-started.md) | ビルド、core daemon / GUI の起動、シャットダウン、最小 2 ノード動作手順 |
| [projects-and-peers.md](projects-and-peers.md) | プロジェクト追加 (open / invite / join) と peer 管理 (list / connect / disconnect) |
| [platforms.md](platforms.md) | Windows / Linux / macOS 対応状況、IPC 経路差分、各 OS の前提条件 |

## クイックリファレンス

```bash
# 1. daemon 起動 (フォアグラウンド常駐)
./target/release/synergos-core start

# 2. 別ターミナルで GUI (任意)
./target/release/synergos-gui

# 3. プロジェクトを自分で作る
./target/release/synergos-core project open myproj /path/to/dir -n "MyProject"

# 4. 他ノードを招待
./target/release/synergos-core project invite myproj
# → トークンを相手に渡す

# 5. 相手ノード側で参加
./target/release/synergos-core project join <token> /path/to/local

# 6. 状況を見る
./target/release/synergos-core project list
./target/release/synergos-core peer list myproj
./target/release/synergos-core network
```

詳細は各ドキュメントを参照してください。
