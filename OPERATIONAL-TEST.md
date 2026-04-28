# Synergos 運用テスト手順書

4 PC (Linux サーバ含む) で実施する運用テストの手順書。検証対象は以下 4 要件:

1. **ノード登録** — invite token / peer announcement
2. **プロジェクトデータ転送** — Bitswap v2 + CID 重複排除
3. **プロジェクトデータ更新** — `PublishUpdate` → CatalogUpdate
4. **更新差分が他の全ノードに伝搬する** — `CatalogSyncCompletedEvent` を全ピアで観測

副次的に cross-OS 互換、disconnect-during-update、リレー経路も同時にカバーする。

---

## 0. PC 配役

| 役 | OS | 推奨配置 | 役割 |
|---|---|---|---|
| **A** | Windows | 開発機 | origin、編集主体 |
| **B** | Linux | クラウド or 別ネット | 常駐ピア (24h up)、必要なら `synergos-relay` 兼任 |
| **C** | Linux デスクトップ | LAN | 第二コンシューマ (Wayland / X11 確認) |
| **D** | Windows or macOS | LAN | 第三コンシューマ (cross-OS 検証) |

B を **別ネットワーク (クラウド)** にすると WAN レイテンシも検証できて実戦的。
全 PC が同 LAN でも要件 4 つは確認できるが、tunnel / relay 経路の実用検証は弱くなる。

---

## 1. 事前準備 (全 PC)

### 1.1 Rust ツールチェーン

最低 `stable 1.89+`、推奨 `1.95+` (CI と同じ)。

```bash
rustup update stable
rustc --version   # 1.89.0 以上を確認
```

### 1.2 OS 別の追加準備

#### Windows (A, D)

- Cloudflared (任意、B がクラウドで tunnel 経由なら必要):
  https://github.com/cloudflare/cloudflared/releases から `cloudflared-windows-amd64.exe` DL → `cloudflared.exe` にリネームして PATH へ。
- WebView2 ランタイム (GUI 経由で確認するなら): Win11 標準搭載、Win10 は別途 DL。

#### Linux (B, C)

```bash
sudo apt install build-essential pkg-config libssl-dev \
                 libxkbcommon-dev libwayland-dev libxcb1-dev \
                 libglu1-mesa-dev

# cloudflared (任意)
wget -qO- https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 \
  | sudo tee /usr/local/bin/cloudflared > /dev/null
sudo chmod +x /usr/local/bin/cloudflared
```

#### macOS (D の選択肢)

```bash
xcode-select --install
brew install cloudflared
```

### 1.3 ビルド (全 PC 共通)

```bash
git clone https://github.com/LUDIARS/Synergos && cd Synergos
cargo build --release --workspace
```

成果物:

| バイナリ | パス |
|---|---|
| `synergos-core` | `target/release/synergos-core[.exe]` |
| `synergos-gui` | `target/release/synergos-gui[.exe]` |
| `synergos-relay` | `target/release/synergos-relay[.exe]` |

### 1.4 ログ準備 (各 PC)

`peer list` / `project list` を秒間隔で吐かせるシェルを各 PC に配置。例 (Linux/macOS):

```bash
# observe.sh — 5秒おきに peer/project 状況をログる
mkdir -p ~/synergos-logs
while true; do
  ts=$(date +%FT%T)
  ./target/release/synergos-core peer list myproj 2>&1 | sed "s/^/[$ts peers] /"
  ./target/release/synergos-core project list   2>&1 | sed "s/^/[$ts proj ] /"
  sleep 5
done >> ~/synergos-logs/observe-$(hostname).log
```

Windows (PowerShell):

```powershell
mkdir $HOME\synergos-logs -Force | Out-Null
while ($true) {
  $ts = Get-Date -Format o
  & .\target\release\synergos-core.exe peer list myproj 2>&1 | ForEach-Object { "[$ts peers] $_" }
  & .\target\release\synergos-core.exe project list 2>&1 | ForEach-Object { "[$ts proj ] $_" }
  Start-Sleep 5
} *>> "$HOME\synergos-logs\observe-$env:COMPUTERNAME.log"
```

---

## 2. テストケース

各テスト終了時、観測ログ (`~/synergos-logs/observe-*.log`) と daemon ログを保存する。

### T1. ノード登録 — A → B 接続

**目的**: invite token によるノード登録、peer announcement、identity 永続化。

**手順**:

```bash
# A (Windows)
$env:RUST_LOG="synergos_core=info,synergos_net=debug"
.\target\release\synergos-core.exe start
# 別タブで:
.\target\release\synergos-core.exe project open myproj C:\synergos-test\A -n "MyProj"
.\target\release\synergos-core.exe project invite myproj
# → 出力された token を控える
```

```bash
# B (Linux)
RUST_LOG=synergos_core=info,synergos_net=debug ./target/release/synergos-core start
# 別タブで:
./target/release/synergos-core project join <token-from-A> /home/syn/synergos-test/B
```

**確認**:

- A で `synergos-core peer list myproj` に B が出る
- B で `synergos-core peer list myproj` に A が出る
- 両ノードで `synergos-core status` の peer_id が `~/.config/synergos/identity` (or `%APPDATA%/synergos/identity`) に永続化されている

**合格基準**: 両ノード相互に `peer list` で見える。プロセス再起動後も peer_id が変わらない。

---

### T2. 4-node full mesh 形成

**目的**: 4 ノードが全員相互可視になる (DHT + gossip propagation)。

**手順**:

T1 の続きで:

```bash
# A or B でさらに invite token を発行
synergos-core project invite myproj   # → token-2

# C で
synergos-core project join <token-2> /home/syn/synergos-test/C

# A or B or C で
synergos-core project invite myproj   # → token-3

# D で
synergos-core project join <token-3> D:\synergos-test\D
```

**確認**:

```bash
# 全 4 PC で同じコマンドを叩く
synergos-core peer list myproj
# 期待: 自分以外の 3 ピアが全部見える
```

**合格基準**: 4 ノード全員で 3 peer ずつ見える (10 秒待っても見えない場合は要調査)。

---

### T3. プロジェクトデータ転送 (大容量) — 要件 #2

**目的**: Bitswap v2 によるファイル転送、CID 重複排除、帯域制御 (Large 60% / Med 30% / Small 10%) の挙動。

**手順**:

```bash
# A: 100MB のテストファイルを投入
dd if=/dev/urandom of=C:\synergos-test\A\big-1.bin bs=1M count=100        # Linux/macOS は /dev/urandom
dd if=/dev/urandom of=C:\synergos-test\A\big-2.bin bs=1M count=500        # 500MB
# 小ファイル群
for i in {1..50}; do dd if=/dev/urandom of=C:\synergos-test\A\small-$i.txt bs=1K count=10; done

# A で publish
synergos-core project publish myproj
```

**確認** (B, C, D で):

- `~/synergos-test/{B,C,D}/big-1.bin` 等が到着し、サイズ一致
- `synergos-core` ログに `BitswapSession started ... bytes_received=...` が出る
- 同じファイルを A が再送しても再転送されない (CID キャッシュヒット)

**合格基準**: 全 3 ノードで 600MB+50 ファイルが揃い、SHA-256 が A と一致する。

---

### T4. プロジェクトデータ更新 + 4-way 伝搬 — 要件 #3 + #4

**目的**: 差分転送 + 全ノードへの propagation。

**手順**:

```bash
# A: big-1.bin の 10〜30% を書き換え
# Linux:
dd if=/dev/urandom of=A/big-1.bin bs=1M count=10 seek=20 conv=notrunc

# Windows (PowerShell):
$bytes = New-Object byte[] (10MB)
(New-Object Random).NextBytes($bytes)
$fs = [IO.File]::OpenWrite("C:\synergos-test\A\big-1.bin")
$fs.Seek(20MB, "Begin") | Out-Null
$fs.Write($bytes, 0, $bytes.Length)
$fs.Close()

# A で更新通知
synergos-core project publish myproj
```

**確認** (B, C, D で並行観測):

- daemon ログに **全 3 ノードで** 以下が順に出る:
  1. `CatalogUpdate received from <A's peer_id>`
  2. `CatalogSyncNeededEvent` emitted
  3. `BitswapSession ...` (この時 chunk 単位の差分のみ pull、全ファイル再転送ではない)
  4. `CatalogSyncCompletedEvent` emitted
- 観測ログで `peer list` の `last_seen` が更新時刻に近い

**合格基準**:
- 全 3 ノードで `CatalogSyncCompletedEvent` が記録される
- 受信バイト数が **元ファイル全体より小さい** (差分転送が効いている)
- 各ノードの `big-1.bin` の SHA-256 が A と一致

---

### T5. 不在中キャッチアップ

**目的**: ノードがオフライン中の更新を、復帰時に取り込めるか。

**手順**:

```bash
# A: daemon を停止
synergos-core stop   # または Ctrl+C

# C: A 不在中にファイル更新
echo "modified by C" >> /home/syn/synergos-test/C/notes.txt
synergos-core project publish myproj
# → B と D には伝搬する (T4 と同じ経路)

# A: 数分後に復帰
synergos-core start
```

**確認**: A 側で `CatalogSyncNeededEvent` → `CatalogSyncCompletedEvent` が起動直後に観測され、`notes.txt` の内容が C と一致する。

**合格基準**: A が起動 60 秒以内に C/B/D 由来の更新を取り込む。

---

### T6. 同時編集コンフリクト挙動 (現状確認)

**目的**: コンフリクト処理の現状ふるまいを把握 (まだ実装されていない可能性あり)。

**手順**:

4 ノード同時に同じファイルの異なる行を編集:

```bash
# A
echo "line from A" >> shared.txt && synergos-core project publish myproj

# B (同じタイミングで)
echo "line from B" >> shared.txt && synergos-core project publish myproj

# C, D も同様
```

**確認 / 記録**:

- 最終的にどの内容が残るか (last writer wins? CRDT 風 merge? 別 CID として両方残る?)
- ログに conflict 検知のメッセージが出るか
- 結果を **観察ログとして残す** だけ。修正は別タスク。

**合格基準**: 「現状この挙動」と説明できる事実が取れていれば OK。動作不能 (panic / 全ノード disconnect) は不合格。

---

### T7. リレー経路 — Linux サーバ活用

**目的**: synergos-relay 経由で接続できるか。直接 P2P が張れない (NAT/firewall) 状況の retry path を検証。

**手順**:

```bash
# B (Linux サーバ) で synergos-relay を起動
./target/release/synergos-relay --bind 0.0.0.0:7000

# A と D の host firewall で QUIC ポート (既定 4433 等) を一時的にブロック
# Windows: New-NetFirewallRule -Direction Outbound -Action Block -RemotePort 4433 ...
# Linux:   sudo iptables -A OUTPUT -p udp --dport 4433 -j DROP

# A で D 宛にファイル送信を試みる
# → relay 経由で疎通すること
```

**確認**:

- daemon ログに `relay session opened with <relay url>` 等が出る
- D で A の更新が relay 経由で届く (直接ではないが結果は同じ)

**合格基準**: 直接接続できない状態でも転送が完了する。

> ⚠️ **TURN は未実装** のため、対称 NAT / インターネット越え NAT のテストは relay (synergos-relay)
> でカバーする。TURN-only path は今回スコープ外。

---

### T8. cross-OS 互換

**目的**: Windows / Linux / macOS 混在で同じプロジェクトを共有しても整合する。

**手順**:

T1〜T4 をそのまま回す中で、各ノードのファイルを diff しながら以下をチェック:

- パス区切り (`\` vs `/`) が問題にならないか — 各 OS のネイティブパスで `project open` していればローカルパスは無関係
- 大文字小文字 — `Foo.txt` と `foo.txt` を Linux と Windows/macOS で混ぜないこと (分かっていれば再現させてみる)
- 改行コード — Synergos は CRLF↔LF 変換しない (CID = blake3 で content-addressed)。意図的に CRLF ファイルを A から push して、Linux 側でも CRLF のまま届くことを確認

**合格基準**: 各 OS 間でファイルバイト列が完全一致 (sha256 が揃う)。

---

## 3. カバー率まとめ

| 要件 / 観点 | 達成可否 |
|---|---|
| **要件 #1 ノード登録** | ✅ T1, T2 |
| **要件 #2 プロジェクトデータ転送** | ✅ T3 |
| **要件 #3 データ更新** | ✅ T4 |
| **要件 #4 全ノードへの伝搬** | ✅ T4 (4-way) |
| cross-OS 互換 | ✅ T8 |
| disconnect-during-update | ✅ T5 |
| relay fallback | ✅ T7 (Linux サーバ活用) |
| WAN レイテンシ | ⚠️ B をクラウドに置けば達成、同 LAN だと部分的 |
| 大規模 peer (10+) | ❌ 4 PC では不可、別途 stress 用構成が必要 |
| TURN-only path | ❌ TURN 未実装、本テストではスコープ外 |
| 長期安定性 (週単位 leak) | ⚠️ B を放置で粗い計測のみ。ベンチは別タスク |

---

## 4. テスト記録テンプレート

各テストで以下を残しておくと再現と原因切り分けが楽:

```
# T<N> 記録 — YYYY-MM-DD HH:MM
- 開始時刻 / 終了時刻
- 各 PC の peer_id (synergos-core status の出力)
- 各 PC のバージョン (git rev-parse HEAD)
- 各 PC の RUST_LOG 設定
- 観測ログ (~/synergos-logs/observe-*.log) のアーカイブ
- daemon stdout/stderr の保存
- ファイル sha256 比較結果 (ノード間で一致したか)
- 合格 / 不合格 / 部分合格 + 観察された異常
```

例: T4 で受信バイト数を測るには daemon ログから `BitswapSession ... bytes_received=...` を grep。

---

## 5. トラブルシューティング (運用時のみ追記)

| 症状 | 切り分け |
|---|---|
| 2 ノードが peer list でお互いを見ない | `cloudflared` が PATH にあるか、もしくは LAN の場合 firewall (UDP 4433) が開いているか。10 秒待ってから再実行 |
| `project join` が "invalid token" | token は 1 回限り使い捨ての可能性。発行ノードから再取得 |
| 4 ノード目だけ propagate が遅い | gossip fanout がまだ収束していない。30 秒〜1 分待つ。それでも届かなければ `peer list` で接続状態を確認 |
| Windows GUI が起動しない | Remote Desktop 越しは glutin が GL コンテキスト張れないことがある。コンソール経由 (CLI) で代用 |
| Linux Wayland で GUI 起動失敗 | `GDK_BACKEND=x11 ./target/release/synergos-gui` で X11 fallback |

詳細は [docs/getting-started.md](docs/getting-started.md) と [docs/platforms.md](docs/platforms.md) を参照。
