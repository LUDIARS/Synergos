# 2 台のマシンで Synergos を動かす (運用手順)

別々のマシン 2 台 (以下 **A = ホスト**, **B = 参加側**) でプロジェクトを共有し、
publish → 自動同期が回るところまでを、詰まりやすい点込みで一本道にした手順。
LAN 内 (家 / オフィス) と、インターネット越し (Cloudflare Mesh) の 2 パターンを扱う。

> 内部の流れは [projects-and-peers.md](projects-and-peers.md)、残作業の全体像は
> [operations-readiness.md](operations-readiness.md) を参照。

---

## 0. 先に知っておく制約 (2026-08 時点)

| 事項 | 内容 |
|---|---|
| **同期は publish 起点** | 自動監視はしない。A で `project publish <id> <files...>` した瞬間に B へ流れる。ファイル単位・全体転送 (差分転送は未実装、[versioning-design.md](versioning-design.md)) |
| **削除 / リネームは伝搬しない** | 消したいファイルは B でも手で消す (P1 残作業) |
| **接続の向きは B → A** | B が A の `/peer-info` を GET → QUIC で A に接続する。**A 側だけ**が inbound を受ける必要がある (QUIC/UDP + peer-info/TCP)。接続後は双方向なので B → A の publish も流れる |
| **AWS/Cloudflare Tunnel 経由は不可** | Tunnel は UDP を通さない (SETUP.md)。インターネット越しは Cloudflare Mesh か、A に直接到達できる IP (VPS / ポート開放) が要る |
| **同一 daemon 内 ACL 無し** | project_id を知り A に到達できるノードは誰でも参加できる。認可はネットワーク層 (LAN / Mesh のエンロール) で担保する |

---

## 1. 共通準備 (A, B とも)

```bash
git clone https://github.com/LUDIARS/Synergos && cd Synergos
cargo build --release -p synergos-core          # GUI は不要なら省略
```

バイナリは `target/release/synergos-core[.exe]`。以降 `synergos-core` と書く
(Windows は `.\target\release\synergos-core.exe`)。

**設定ファイルは `start --config <path>` で明示的に渡す** (自動探索しない)。
置き場所の推奨:

| OS | パス |
|---|---|
| Windows | `%APPDATA%\Synergos\synergos-net.toml` |
| Linux / macOS | `~/.config/synergos/synergos-net.toml` |

identity (peer_id の元) は初回起動で `%APPDATA%\Synergos\identity.key` /
`~/.config/synergos/identity.key` に自動生成される。**消すと peer_id が変わる**。

---

## 2. パターン 1: 同じ LAN 内 (まずこれで動作確認)

### 2.1 A (ホスト) の設定

`synergos-net.toml`:

```toml
# QUIC を固定ポートで待ち受ける ([::] はデュアルスタックで IPv4 も受ける)
[quic]
listen_addr = "[::]:4433"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

# 参加側が最初に叩く /peer-info サーブレット (TCP)
peer_info_listen_addr = "0.0.0.0:7780"

# 参加側は「/peer-info を取りに行った先のホスト」に QUIC 接続する。
# 0.0.0.0 (unspecified) を告知しておくと、その置換が自動で効く。
# LAN でも Mesh でも同じ設定で済む (自分の IP を書かなくてよい)。
quic_advertised_addr = "0.0.0.0:4433"

# 招待トークンに埋め込む、B から見た A の URL (LAN の IP)
peer_info_advertised_url = "http://192.168.1.10:7780"

# 起動時の NAT 越え probe (IPv6 / UPnP / Tunnel、数秒) を省く。
# join / peer add-url の直結経路には影響しないが、LAN/Mesh 運用では不要
auto_promote = false

[tunnel]
api_token_ref = ""
hostname = ""             # 空: 内蔵 cloudflared を起動しない
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
```

> `[mesh]` `[dht]` `[gossipsub]` 等の他セクションは省略可 (既定値が入る)。
> 完全なキー一覧は SETUP.md §3-B-1 を参照。

**A のファイアウォール** (inbound を開けるのは A だけ):

```powershell
# Windows (管理者 PowerShell)
New-NetFirewallRule -DisplayName "Synergos QUIC" -Direction Inbound -Protocol UDP -LocalPort 4433 -Action Allow
New-NetFirewallRule -DisplayName "Synergos peer-info" -Direction Inbound -Protocol TCP -LocalPort 7780 -Action Allow
```

```bash
# Linux (ufw)
sudo ufw allow 4433/udp
sudo ufw allow 7780/tcp
```

### 2.2 B (参加側) の設定

B は inbound 不要。最小設定:

```toml
[quic]
listen_addr = "[::]:0"        # 任意ポート (省略可)
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

auto_promote = false

[tunnel]
api_token_ref = ""
hostname = ""
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
```

### 2.3 起動 → 招待 → 参加

```bash
# ── A ──
synergos-core start --config <A の synergos-net.toml>
# 別ターミナル
synergos-core project open myproj D:/share/A -n "MyProj"
synergos-core project invite myproj --expires 3600
#   → Invite token: syn1.eyJ...   (syn1. で始まっていることを確認)
#     --url を付けなくても peer_info_advertised_url が入っていれば syn1. になる。
#     UUID だけのトークンが出たら A の設定に peer_info_advertised_url が無い。
#     その場合は `project invite myproj --url http://192.168.1.10:7780`

# ── B ──
synergos-core start --config <B の synergos-net.toml>
# 別ターミナル
synergos-core project join syn1.eyJ... D:/share/B
#   → Joined project.
#   (B 側で同じ project_id "myproj" が open され、A に QUIC 接続済み)

# ── 両方で確認 ──
synergos-core project list
synergos-core peer list myproj      # 相手が Connected で見えれば OK
```

`join` が `could not reach host` で失敗した場合、プロジェクト自体は B に open
済みなので、原因 (A の firewall / IP) を直したあと
`synergos-core peer add-url myproj http://192.168.1.10:7780` で接続だけやり直せる。

### 2.4 同期の確認

```bash
# ── A ── ファイルを置いて publish
cp big.bin D:/share/A/assets/big.bin
synergos-core project publish myproj assets/big.bin

# ── B ── 数秒で D:/share/B/assets/big.bin が現れる
synergos-core transfer list
```

続けて A で同じファイルを書き換えて `publish` すると、B へ **新バージョンとして再送**される
(内容が同じなら送らない)。B 側で編集して `publish` すれば A へも流れる。

同期の台帳はプロジェクトルートの `.synergos/manifest.json` (両側) に残る。
再起動しても引き継がれる。

---

## 3. パターン 2: インターネット越し (Cloudflare Mesh)

LAN 手順との差分は **「A の到達アドレスが Mesh IP になる」** だけ。

1. Cloudflare Zero Trust で Mesh を有効化し、A・B を参加させる。
   - Linux サーバ = `warp-cli connector new <TOKEN>` (Mesh node)
   - Windows / macOS = Cloudflare One Client でエンロール (client device)。
     device-to-device が許可されていれば A が Windows でも可
2. 各マシンの Mesh IP (`100.96.x.x`) を控える
   (`warp-cli status` / Cloudflare One Client の表示 / `ipconfig`)
3. **A の設定**を LAN 版から 1 行変える:
   ```toml
   peer_info_advertised_url = "http://100.96.0.5:7780"   # A の Mesh IP
   ```
   `quic_advertised_addr = "0.0.0.0:4433"` はそのまま (置換で Mesh IP になる)。
   listen を Mesh IP に絞りたければ `peer_info_listen_addr = "100.96.0.5:7780"`
4. **A の firewall** は Mesh レンジ限定で開ける:
   ```powershell
   New-NetFirewallRule -DisplayName "Synergos QUIC (Mesh)" -Direction Inbound -Protocol UDP -LocalPort 4433 -RemoteAddress 100.96.0.0/12 -Action Allow
   New-NetFirewallRule -DisplayName "Synergos peer-info (Mesh)" -Direction Inbound -Protocol TCP -LocalPort 7780 -RemoteAddress 100.96.0.0/12 -Action Allow
   ```
   (Windows は Mesh レンジ inbound が既定ブロックなのでこれが無いと繋がらない)
5. 以降は §2.3 / §2.4 と同じ。`invite` のトークンに Mesh IP が入る。

> Mesh 無しでインターネット越しにやる場合は、A を **UDP 4433 + TCP 7780 が
> 直接届く場所** (VPS、ポート開放したルーター) に置き、`peer_info_advertised_url`
> にそのグローバル IP / FQDN を書く。CGNAT / 対称 NAT の家庭回線同士は不可 (TURN 未実装)。

---

## 3.5 履歴ノード (任意、推奨: 常駐する A を履歴ノードにする)

通常ノードは最新版しか持たない。**旧版に戻せる・publisher が消えても実体が残る**ようにするには、
常駐ノード 1 台の `synergos.toml` に `[history]` を足す (docs/versioning-design.md §3):

```toml
[history]
enabled = true            # このノードを履歴ノードにする (既定 false)
projects = ["*"]          # 対象プロジェクト (既定: 参加中すべて)
root = ".synergos/history" # 保管庫。絶対パス (別ドライブ) も可
max_versions_per_file = 0 # 0 = 無制限
max_age_days = 0
max_bytes = 0
```

- 有効化後に publish / 受信した版から保管される (既に作業ツリーにある版は次の publish で入る)
- `synergos-core history ls <project>` で保持版を確認、`history gc` で保持ポリシー適用
- 他ノードで戻す: `synergos-core project restore <project> assets/big.bin --version 1`
  → 履歴ノードが v1 を送ってくる。`git checkout <古いコミット>` の後は
  `synergos-core project checkout <project>` で manifest に合わせて一括で取り直す
- `.gitignore` に `.synergos/history/` と `.synergos/state.json` を追加する
  (履歴実体と版番号高水位は node ローカル。`manifest.json` だけを git に入れる)
- 現段階の project topic には参加ピア ACL がないため、履歴ノードは信頼できる mesh 内だけで使う。
  project ID や招待トークンを、履歴データのアクセス制御境界として扱わない

---

## 4. 常駐化

- **Linux**: SETUP.md §3-B-2 の systemd unit をそのまま使う
  (`Environment=XDG_RUNTIME_DIR=/run/user/1000` + `loginctl enable-linger` を忘れない)。
- **Windows**: `synergos-core.exe start --config ...` をタスクスケジューラ
  「ログオン時」またはスタートアップに登録。サービス化 (`sc create`) は
  IPC (Named Pipe) の呼び出しユーザ検証がセッション違いで弾くので**しない**。
  常駐と CLI 操作は同じユーザで行う。

---

## 5. トラブルシューティング

| 症状 | 見るところ / 直し方 |
|---|---|
| `project join` → `invalid invite token` / UUID 形のトークン | 従来型 (発行 daemon 内限定) トークン。A で `--url` を付けるか `peer_info_advertised_url` を設定して再発行 |
| `join` → `bootstrap failed: http error` | B から A の 7780/TCP に届いていない。`curl http://<A>:7780/peer-info` で切り分け。A の firewall / `peer_info_listen_addr` |
| `join` → `quic connect failed` / timeout | 4433/UDP。Windows の firewall、Mesh レンジ許可。A の `listen_addr` が `[::]:4433` になっているか (`start` ログの `QUIC listening on`) |
| A が Linux で IPv4 の B から繋がらない | `[::]` はデュアルスタック bind するので通常 OK。`sysctl net.ipv6.bindv6only=1` の環境なら `listen_addr = "0.0.0.0:4433"` にする |
| `peer at ... is xxxx but invite was issued by yyyy` | URL の先に別の daemon がいる (IP 使い回し / 別ノード)。トークンを発行した A の URL を確認 |
| publish しても B にファイルが来ない | B の `project list` に同じ project_id があるか。`peer list` で A が Connected か。A の daemon ログに `[asset-update]`、B に `auto-pull` が出るか (`RUST_LOG=synergos_core=debug`) |
| 2 回目以降の publish が反映されない | 内容が変わっていない (CRC 同一) と再送しない仕様。変わっているのに来ないなら `.synergos/manifest.json` の version を A/B で見比べる |
| B に `.synergos/incoming/*.part` が残る | 転送途中で切れた残骸。消してよい。次の publish で取り直す |
| Windows: `project publish` が `not a regular file` | ディレクトリは publish できない。ファイルを列挙して渡す (`Get-ChildItem -Recurse -File`) |
| A を再起動したら B が取りに来ない | A は `.synergos/manifest.json` から Offer 台帳を復元する。復元ログ `restored N shared file record(s)` を確認。実ファイルを移動/削除していると復元対象外 |
| `project restore` / `checkout` しても旧版が来ない | 履歴ノードが 1 台も無い (旧版は誰も持っていない)、または履歴ノードの `history ls` にその版が無い (有効化前の版)。履歴ノードの daemon ログに `history node: sending ...` が出るか |
| `Daemon not running` (CLI) | Linux systemd 常駐時の `XDG_RUNTIME_DIR` 不一致 (SETUP.md つまずき表)。Windows は別ユーザで CLI を叩いていないか |

ログを増やす: `RUST_LOG=synergos_core=debug,synergos_net=debug synergos-core start ...`
(Windows PowerShell: `$env:RUST_LOG="..."; .\synergos-core.exe start ...`)。

---

## 6. 運用テストへ

2 台で §2.4 が回ったら [OPERATIONAL-TEST.md](../OPERATIONAL-TEST.md) の T1 / T3 / T4 を
そのまま 2 台版で実施できる (T2 の 4 台メッシュ、T5 不在中キャッチアップ、T7 relay は
残作業の解消後)。
