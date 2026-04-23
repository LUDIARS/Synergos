# プロジェクトとピアの管理

Synergos は **プロジェクト = 共有空間**、**ピア (peer / ノード) = そこに参加する他デーモン** のモデルです。
すべての操作は `synergos-core` CLI (= 裏で IPC → daemon) で行います。GUI 版でも同じ機能が UI として提供されます。

## プロジェクト

### 新規プロジェクトを立ち上げる (1 台目)

```bash
synergos-core project open <project-id> <local-path> [-n "表示名"]
```

- `project-id` は任意文字列。ネットワーク全体での識別子になるのでユニークなものを。
- `local-path` は **絶対パス**。そのディレクトリをプロジェクトルートとして監視対象にする。
- `-n "..."` は GUI / `project list` での表示名。

例:
```bash
synergos-core project open myasset D:/projects/my-assets -n "MyAssets"
```

プロジェクトは daemon 再起動後も `projects.json` から自動 restore されます (identity と同じ dir)。
ProjectOpen 時に対応する `CatalogManager` が立ち上がり、gossip CatalogUpdate の受信対象になります。

### 招待トークンを発行する

```bash
synergos-core project invite <project-id> [--expires <秒>]
```

- `--expires 3600` のように指定するとワンタイム的な期限付きトークンを発行。
- 省略すると無期限。

出力例:
```
Invite token: inv-xxxxxxxxxxxxxxxxxxxxxxx
Expires at:   1746000000 (epoch)
```

この token を Slack / メッセンジャー等で相手に渡します。

### 招待で既存プロジェクトに参加する (2 台目以降)

```bash
synergos-core project join <token> <local-path>
```

- token はホスト側で発行したもの。
- `local-path` は参加ノード側の保存先ディレクトリ (ローカルルート)。

参加後、ホストと同じ `project-id` で自動的に open されます。

### プロジェクトの確認・更新・終了

```bash
synergos-core project list                    # 全プロジェクトの一覧
synergos-core project get <project-id>        # 詳細 (peers, transfers, sync_mode 等)
synergos-core project update <project-id> [-n "名前"] [-d "説明"] \
                                              [-s full|manual|selective] [-m <max_peers>]
synergos-core project close <project-id>      # プロジェクトを閉じる (他の daemon との同期停止)
```

## ピア (ノード)

Synergos は **Gossipsub + DHT + Cloudflare Tunnel** で自動ピアディスカバリを行うので、通常は
`project join` さえ済めば peer は勝手に見つかります。明示的に接続したい場合だけ CLI を使います。

### ピア一覧

```bash
synergos-core peer list <project-id>
```

出力例:
```
  Alice (peer-abcd1234...) — direct | 12 ms | 50000000 bps
  Bob   (peer-efgh5678...) — tunnel | 38 ms |  8000000 bps
```

`route` 列は `direct` (IPv6 直) / `tunnel` (Cloudflare Tunnel) / `relay` (synergos-relay 経由) のいずれか。
優先度順に自動 route migration されます。

### 明示的に接続 / 切断

```bash
synergos-core peer connect <project-id> <peer-id>
synergos-core peer disconnect <peer-id>
```

peer_id は `peer list` の出力に出る `peer-xxxx...` の文字列をそのまま使います。

### ネットワーク全体の状況

```bash
synergos-core network
```

```
Network Status
  Route:       direct
  Connections: 3/64
  Bandwidth:   67000000 bps
  Latency:     24 ms
```

## 転送 (transfer)

実ファイル転送は gossip FileOffer / FileWant → QUIC TXFR ストリームで自動的に起動します。
状況確認と明示キャンセルだけ CLI で可能:

```bash
synergos-core transfer list [-p <project-id>]
synergos-core transfer cancel <transfer-id>
```

## 動作の全体像 (publish → sync)

ユーザ A がファイルを変更して `PublishUpdate` が走ったときの流れ (PR #31 の auto-update 経路):

```
A: publish_updates
  ├─ step 1/5  ledger に Offer を登録
  ├─ step 2/5  shared_files に path + meta を記録 (FileWant に即応するため)
  ├─ step 3/5  gossip FileOffer bcast
  ├─ step 4/5  ContentStore に RootCatalog snapshot を put (catalog_cid 生成)
  └─ step 5/5  gossip CatalogUpdate bcast (catalog_cid + publisher 付き)

B: gossip 受信
  ├─ CatalogUpdate の root_crc が自分と違う → catalog drift 検知
  ├─ CatalogSyncNeededEvent を emit (catalog_cid, publisher, changed_chunks 付き)
  │
  └─ CatalogSyncService が購読
     ├─ BitswapSession で A に接続し catalog_cid を取得
     ├─ RootCatalog を deserialize
     └─ CatalogSyncCompletedEvent を emit (fetched_blocks, error)
```

GUI / IPC クライアントは `CatalogSyncCompletedEvent` を subscribe しておけば
"sync 中 / sync 完了 / sync 失敗" の UI 表示ができます。

## 最小の 2 ノード例 (再掲)

```bash
# A
synergos-core start
synergos-core project open myproj /path/to/A
synergos-core project invite myproj                    # token を B に渡す

# B
synergos-core start
synergos-core project join <token> /path/to/B

# 両方で確認
synergos-core project list
synergos-core peer list myproj
```
