# AWS Graviton (ARM64) セットアップガイド

Synergos を AWS の ARM64 (Graviton) インスタンス上に立ち上げるための完全手順。
Linux ARM64 専用ビルドは配布していないが、ARM 阻害コードは無いのでネイティブビルドで動作する。

`synergos-relay` (中継だけ) と `synergos-core` (フルピア) で目的が違うので両方記載。

> **前提**: Cloudflare Tunnel は Web ダッシュボード (Zero Trust → Networks → Tunnels) で
> 作成済み。hostname → ingress のルーティング設定は §2.5 を参照。
> 本ガイドは EC2 側で **コネクタトークンを使って cloudflared を接続するだけ**。

---

## 0. EC2 を用意

- **インスタンス**: `t4g.small` (relay) / `t4g.medium` (core, ビルド時)
  - 必ず Graviton 系 (`t4g` / `c7g` / `m7g`)
- **AMI**: Ubuntu 22.04 LTS (arm64) 推奨
  - Amazon Linux 2023 でも可だが、本ガイドは Ubuntu 前提
- **Security Group**: SSH (22) のみ inbound 許可
  - 他は全部 cloudflared 経由なので閉じてよい

```bash
ssh ubuntu@<EC2 public ip>
```

---

## 1. ビルド (両シナリオ共通)

```bash
# --- 依存パッケージ ---
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git

# --- Rust toolchain ---
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
. "$HOME/.cargo/env"

# --- Synergos clone (private repo の場合は deploy key/PAT を仕込む) ---
git clone https://github.com/<your-org>/Synergos.git
cd Synergos

# --- ビルド (relay と core だけ。GUI は不要なので省略) ---
cargo build --release -p synergos-relay -p synergos-core

# 成果物
ls target/release/synergos-{relay,core}
```

t4g.medium で初回 ~10-15 分。完了後はインスタンスタイプを `t4g.small` に下げてもよい
(実行は軽い)。

---

## 2. cloudflared をコネクタとしてインストール

Cloudflare ダッシュボードから **コネクタトークン** を取得済み前提:

1. Zero Trust → Networks → Tunnels → 該当 Tunnel を開く
2. "Install connector" ペインで **Linux / arm64 / Debian** を選ぶ
3. 表示される `cloudflared service install <TOKEN>` の `<TOKEN>` 部分をコピー

EC2 上で:

```bash
# --- cloudflared インストール (Cloudflare 公式 deb, arm64) ---
curl -L --output cloudflared.deb \
  https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64.deb
sudo dpkg -i cloudflared.deb
cloudflared --version

# --- コネクタとして登録 (ダッシュボードのトークンを貼る) ---
sudo cloudflared service install <TOKEN>
```

これで `cloudflared.service` が systemd に登録され、自動起動済み。動作確認:

```bash
systemctl status cloudflared
journalctl -u cloudflared -f          # "Registered tunnel connection" が見えれば OK
cloudflared tunnel list               # ダッシュボードに登録された tunnel が並ぶ
```

> ingress (どの hostname をどのローカルポートに繋ぐか) は **すべてダッシュボード側で管理**。
> EC2 上に `config.yml` を置く必要は無い。

---

## 2.5 ダッシュボードで URL (Public Hostname) を設定する

EC2 側のコネクタが緑色 (HEALTHY) になったら、Cloudflare ダッシュボードで
hostname → ローカルサービスのルーティングを設定する。

### 手順

1. **Zero Trust → Networks → Tunnels** で対象 Tunnel を開く
2. **Public Hostname** タブ → **Add a public hostname** をクリック
3. 以下を入力:

| 項目 | 値 (シナリオ A: relay) | 値 (シナリオ B: core) |
|---|---|---|
| **Subdomain** | `relay` (任意) | `node1` (任意) |
| **Domain** | Cloudflare DNS に登録済みのドメインを選択 (例 `example.com`) | 同左 |
| **Path** | 空 | 空 |
| **Service Type** | `HTTP` | `TCP` |
| **URL** | `localhost:8080` | `localhost:7777` |

4. **Save hostname** で保存

これだけで `https://relay.example.com` (TLS は Cloudflare 終端) → EC2 上の
`http://localhost:8080` に到達する経路ができる。DNS レコードもダッシュボードが
自動で `CNAME` を張ってくれるので別途 DNS 設定は不要。

### 補足: Additional application settings

WebSocket を扱う場合 (relay 用途) は `Additional application settings` →
`TLS` → `No TLS Verify: Off` のままで OK (localhost 平文 HTTP なので)。
`Connection` → デフォルトのままで WebSocket upgrade は自動透過する。

### 反映確認

```bash
# ローカル PC から
curl -i https://relay.example.com/
# → 426 Upgrade Required (= relay は WS 待ち) が返れば疎通 OK
```

ダッシュボード側の変更は **EC2 の cloudflared を再起動しなくても即時反映**される
(コネクタが Cloudflare Edge から ingress 設定を pull する設計)。

---

## 3-A. シナリオ A: synergos-relay を立てる (推奨)

ダッシュボード側の ingress 設定: 例えば `relay.example.com → http://localhost:8080`。

### 3-A-1. systemd unit を作る

`/etc/systemd/system/synergos-relay.service` を作成 (コピペでそのまま実行):

```bash
sudo tee /etc/systemd/system/synergos-relay.service > /dev/null <<'EOF'
[Unit]
Description=Synergos WebSocket Relay
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/Synergos
Environment=RUST_LOG=synergos_relay=info
ExecStart=/home/ubuntu/Synergos/target/release/synergos-relay
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
```

### 3-A-2. 起動

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now synergos-relay

# 状態確認
ls -l /etc/systemd/system/synergos-relay.service   # ファイルが存在するか
systemctl status synergos-relay
journalctl -u synergos-relay -f
```

> `Failed to enable unit: Unit file synergos-relay.service does not exist.` が出る場合、
> 上記 `sudo tee` ステップが実行されていない。`ls -l` でファイル存在を確認すること。

成功ログ: `relay ready on 0.0.0.0:8080`

### 3-A-3. クライアント側の設定

ローカルの Synergos `config.toml` で relay 経由を強制したいなら:

```toml
force_relay_only = true        # AWS 経由を強制したい場合
```

クライアントが relay に WebSocket 接続するエンドポイントは
`wss://<ダッシュボードで設定した hostname>/` (Cloudflare Tunnel が TLS を終端)。

---

## 3-B. シナリオ B: synergos-core (フルピア) を立てる

ダッシュボード側の ingress 設定: 例えば `node1.example.com → tcp://localhost:7777`。

> **重要**: `synergos-core` の `tunnel.hostname` を空でない値に設定すると、daemon が
> 内部で `cloudflared tunnel run --hostname <name>` を spawn する設計になっている
> (`synergos-net/src/tunnel/mod.rs:336-340`)。これは **cert.pem ベースの旧 named tunnel
> 方式** を前提としており、Web ダッシュボード管理 (コネクタトークン) と併用すると
> 二重起動・二重接続になる。
>
> 本セットアップでは:
>
> 1. `tunnel.hostname = ""` にして core 内蔵の cloudflared spawn を無効化
> 2. 外部の `cloudflared.service` (手順 2) が token 経由で全 ingress を担う
>
> という分離構成にする。

### 3-B-1. 設定ファイル

**`/home/ubuntu/.config/synergos/synergos-net.toml`**

```toml
[tunnel]
api_token_ref = ""
hostname = ""                         # 空: 内蔵 cloudflared 起動を抑止
allow_simulation = false
auto_restart = true
restart_base_ms = 1000
restart_max_ms = 60000

[mesh]
doh_endpoint = "https://cloudflare-dns.com/dns-query"
dns_servers = []
turn_servers = []
stun_servers = []
probe_timeout_ms = 3000

[quic]
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

[dht]
k_bucket_size = 20
routing_refresh_secs = 60
peer_ttl_secs = 120

[gossipsub]
mesh_n = 6
mesh_n_low = 4
mesh_n_high = 12
heartbeat_interval_ms = 1000
message_cache_size = 1000

[stream_allocation]
large_ratio = 60
medium_ratio = 30
small_ratio = 10

[speed_test]
enabled = true
retest_interval_secs = 300
probe_count = 10

[peer_selection]
bandwidth_weight = 0.7
stability_weight = 0.3
recalculate_interval_secs = 60

[monitor]
snapshot_interval_ms = 1000
history_size = 3600
graph_sample_interval_secs = 1
```

### 3-B-2. systemd unit

`/etc/systemd/system/synergos-core.service` を作成:

```bash
# ubuntu ユーザの /run/user/1000 を SSH 切断後も保持 (重要)
sudo loginctl enable-linger ubuntu

sudo tee /etc/systemd/system/synergos-core.service > /dev/null <<'EOF'
[Unit]
Description=Synergos Core Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/home/ubuntu/Synergos
Environment=RUST_LOG=synergos_core=info,synergos_net=info
Environment=XDG_RUNTIME_DIR=/run/user/1000
ExecStart=/home/ubuntu/Synergos/target/release/synergos-core start --config /home/ubuntu/.config/synergos/synergos-net.toml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable --now synergos-core
journalctl -u synergos-core -f
```

> **重要**: `Environment=XDG_RUNTIME_DIR=/run/user/1000` と `loginctl enable-linger`
> はセットで必須。これが無いと daemon の IPC ソケットが `/tmp/synergos/synergos.sock`
> に作られ、対話 SSH の CLI (`XDG_RUNTIME_DIR=/run/user/1000`) からは
> `Daemon not running` で見えない。
>
> `id ubuntu` で UID が 1000 でない場合は `/run/user/<実 UID>` に書き換えること。

成功ログ: `Synergos core daemon started (PID: ..., peer_id=...)`

### 3-B-3. プロジェクト作成 → 招待 token 発行

```bash
# AWS 側 (host)
./target/release/synergos-core project open myproj /home/ubuntu/projects/myproj -n "MyProj"
./target/release/synergos-core project invite myproj
# → "Invite token: inv-xxxxxxxxxxxxx" を控える

# ローカル側
./target/release/synergos-core project join inv-xxxxxxxxxxxxx /path/to/local/myproj
./target/release/synergos-core peer list myproj    # AWS ノードが見えれば成功
```

---

## 動作確認チートシート

```bash
# cloudflared コネクタの接続状況
systemctl status cloudflared
journalctl -u cloudflared --since "5 min ago"

# relay の WS が外から見えるか (ローカル PC から)
curl -i https://<dashboard で設定した hostname>/   # 426 Upgrade Required が出れば OK

# core daemon が応答するか (AWS 上)
./target/release/synergos-core status
./target/release/synergos-core network
```

---

## つまずきやすい点

| 症状 | 対処 |
|---|---|
| `Daemon not running` (CLI から) | systemd で立てた daemon は `XDG_RUNTIME_DIR` 未設定だと socket を `/tmp/synergos/synergos.sock` に作る。一方 SSH 越しの CLI は `/run/user/<uid>/synergos/synergos.sock` を見る → 不一致。unit に `Environment=XDG_RUNTIME_DIR=/run/user/1000` + `loginctl enable-linger ubuntu` を入れる (3-B-2 参照) |
| `cloudflared service install` が "already installed" で失敗 | 既に登録済み。`sudo systemctl restart cloudflared` で再接続。トークン更新時は `sudo cloudflared service uninstall` → 再 install |
| ダッシュボードに connector が "inactive" のまま | `journalctl -u cloudflared -n 50` を確認。多くは時刻ズレ (chrony 確認) か、SG outbound 443 が閉じている |
| シナリオ B で接続が二重に見える | `tunnel.hostname = ""` になっているか再確認。空でないと core が独自 cloudflared を spawn する |
| `quinn` の UDP が通らない | Cloudflare Tunnel は HTTP/WebSocket primary。純 UDP/QUIC を Tunnel 越しに通すのは設計上不向き。AWS では **relay (WS) シナリオ A が現実解** |
| EC2 t4g.small でビルドが OOM kill | swap 追加 (`fallocate -l 4G /swapfile` ...) かビルド時だけ medium 以上に上げる |

---

## 推奨

最初は **シナリオ A (relay) だけ** で動かすのが安定。クライアント間 P2P は
IPv6 / UPnP / Tunnel で互いに繋がり、AWS は「直接繋がらないペアの中継」だけを担う設計
(`force_relay_only` を使うと全ピアが必ず AWS 経由)。
シナリオ B (core を AWS に常設) は QUIC を Cloudflare Tunnel 経由で通す筋が悪いので、
必要になってからで良い。

---

## 関連ドキュメント

- [docs/getting-started.md](docs/getting-started.md): ローカルでの最小 2 ノード手順
- [docs/platforms.md](docs/platforms.md): OS ごとの前提条件
- [docs/projects-and-peers.md](docs/projects-and-peers.md): CLI リファレンス
- [DESIGN.md](DESIGN.md): 内部アーキテクチャ
