# Getting Started

Synergos を初めて動かすための最短ルート。ビルド → 起動 → 2 ノード同期の確認までを扱います。

## ビルド

全 OS 共通で `cargo` から:

```bash
cargo build --release --workspace
```

成果物は `target/release/` に置かれます:

| バイナリ | 役割 |
|---|---|
| `synergos-core` (Windows では `.exe`) | 常駐デーモン。IPC サーバ・EventBus・ファイル転送・CatalogSyncService を持つ |
| `synergos-gui` | egui ベースの GUI クライアント。IPC で core に接続 |
| `synergos-relay` | 中継用 WebSocket リレーサーバ (直接 P2P 不能なネットワーク向け。ローカル検証では不要) |

> Windows のパスは以降 `./target/release/synergos-core.exe` のように `.exe` を付けてください。
> Linux / macOS では拡張子なし。

## 起動

Synergos は **git ↔ SourceTree** のような 2 プロセス構成です。core daemon を先に立ててから GUI / CLI を繋ぎます。

### 1. Core daemon を起動

```bash
./target/release/synergos-core start
```

フォアグラウンドに張り付いてログを出し続けます。以下のメッセージが見えたら常駐 OK:

```
Synergos core daemon started (PID: ..., peer_id=...)
IPC server listening on ...
```

詳細ログを見たい場合 (Bitswap / CatalogSync の動きを確認したいとき):

```bash
# Linux/macOS
RUST_LOG=synergos_core=info,synergos_net=debug ./target/release/synergos-core start

# Windows (PowerShell)
$env:RUST_LOG="synergos_core=info,synergos_net=debug"; ./target/release/synergos-core.exe start
```

### 2. GUI を起動 (任意)

別ターミナルから:

```bash
./target/release/synergos-gui
```

GUI は IPC 経由で core に繋ぐので、必ず **daemon を先に起動** しておく必要があります。

### 3. CLI だけで操作する場合

GUI 不要。同じ CLI バイナリを別サブコマンドで呼ぶと IPC 経由で操作できます:

```bash
./target/release/synergos-core status
./target/release/synergos-core project list
./target/release/synergos-core network
```

コマンド一覧は [projects-and-peers.md](projects-and-peers.md) を参照。

## 停止

- **Ctrl+C**: daemon のターミナルで直接 (graceful shutdown)
- **別ターミナルから**: `./target/release/synergos-core stop`

シャットダウン時は:
- 全プロジェクトを close
- Presence で自ノードを離脱通知
- アクティブ転送をキャンセル
- QUIC / Tunnel / Mesh をクリーンアップ

の順で整理されます。

## 最小の 2 ノード動作手順

ローカル検証用。片方がファイルを公開 → もう片方が自動で追従することを確認します。

```bash
# ── マシン A ──
./target/release/synergos-core start                              # 1. daemon 起動
./target/release/synergos-core project open myproj /path/to/A     # 2. プロジェクト作成
./target/release/synergos-core project invite myproj              # 3. 招待トークン発行
# → 出力されたトークンを B に渡す

# ── マシン B ──
./target/release/synergos-core start
./target/release/synergos-core project join <token> /path/to/B    # 4. 招待で参加

# ── 両方で確認 ──
./target/release/synergos-core project list
./target/release/synergos-core peer list myproj
# お互いが表示されれば接続成立
```

この時点で A でファイルを変更 → `PublishUpdate` が発行されると:

1. `FileOffer` / `CatalogUpdate` が gossip で B に届く
2. B の daemon が `CatalogSyncNeededEvent` を emit
3. `CatalogSyncService` が BitswapSession で RootCatalog を A から取得
4. `CatalogSyncCompletedEvent` が emit (GUI / IPC サブスクライバが検知)

という自動同期フローが走ります (#25/#26 の実装)。

## トラブルシューティング

| 症状 | 対処 |
|---|---|
| `synergos-gui` が "failed to connect" | daemon が起動していない。`synergos-core start` を先に。 |
| `synergos-core status` が無応答 | IPC ソケットの uid チェックに失敗している可能性。daemon を起動したユーザと同じ UID で呼ぶこと。 |
| 2 ノードが `peer list` でお互いを見ない | Cloudflare Tunnel / cloudflared バイナリが PATH にあるか確認。ローカルネットワークなら DHT/gossip の propagation を数秒待つ。 |
| `project open` が "invalid path" | 絶対パス or 存在するディレクトリを渡す必要あり。`-n` で表示名を付けると List で識別しやすい。 |

より詳細な CLI リファレンスは [projects-and-peers.md](projects-and-peers.md)、
OS ごとの前提 (Cloudflared / XDG / Named Pipe) は [platforms.md](platforms.md) を参照してください。
