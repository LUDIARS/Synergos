# Cloudflare Mesh で 2 台 (AWS Linux + Windows 開発機) を動かす実施チェックリスト

[two-node-operations.md](two-node-operations.md) §3 を、A = AWS Graviton Linux、B = Windows 開発機に
当てはめた実施手順。**人手でしかできない段** (dashboard / SSH / インストーラ) と、
そのあとコピペで済む段を分けてある。上から順に。

役割: **A (Linux) をホスト** にする。理由 = inbound を受けるのは A だけで済み、Windows 側の
Mesh レンジ inbound 既定ブロックを気にしなくてよい。B は client device としてエンロールし、
A へ接続しに行くだけ。

> 先行検証の近道: 2 台が既に **Tailscale** で繋がっていれば、§4 以降の Synergos 手順で
> `<A_MESH_IP>` を A の Tailscale IP に読み替えれば、Cloudflare 側の設定を
> 待たずに今日回せる (Mesh は 100.96.0.0/12、Tailscale は 100.64.0.0/10 で firewall の RemoteAddress
> だけ違う)。運用は Mesh、動作確認は先に Tailscale、でもよい。

## 1. Cloudflare 側 (人手・dashboard)

- [ ] Zero Trust org (team name) を確認
- [ ] **Networking > Mesh > Add a node** でウィザードを通す (エンロールポリシー / Split Tunnels Include
      `100.96.0.0/12` / device-to-device / Gateway proxy TCP+UDP+ICMP が自動作成される)
- [ ] 表示された **connector トークン** を控える (A 用)。トークンは秘密情報として安全に保管し、
      コマンド履歴・ログ・リポジトリには残さない
- [ ] エンロールポリシーに B で使うメール (one-time PIN) が含まれていることを確認

## 2. A (AWS Linux) — SSH で実施

```bash
# WARP (Mesh node)
curl -fsSL https://pkg.cloudflareclient.com/pubkey.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/cloudflare-warp-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg] https://pkg.cloudflareclient.com/ $(. /etc/os-release && echo $VERSION_CODENAME) main" | sudo tee /etc/apt/sources.list.d/cloudflare-client.list
sudo apt-get update && sudo apt-get install -y cloudflare-warp
printf 'net.ipv4.ip_forward = 1\nnet.ipv6.conf.all.forwarding = 1\n' | sudo tee /etc/sysctl.d/99-zzz-cloudflare-warp-connector.conf && sudo sysctl --system
read -rs -p 'Connector token: ' CONNECTOR_TOKEN; echo
sudo warp-cli connector new "$CONNECTOR_TOKEN" && unset CONNECTOR_TOKEN && sudo warp-cli connect
warp-cli status && ip -4 addr | grep 100.96      # ← A の Mesh IP を控える (以下 <A_MESH_IP>)

# Synergos (必要な変更が main にマージ → GitHub リリース後)
git clone https://github.com/LUDIARS/Synergos && cd Synergos      # 既存クローンなら git pull
cargo build --release -p synergos-core
sudo ufw allow from 100.96.0.0/12 to any port 4433 proto udp
sudo ufw allow from 100.96.0.0/12 to any port 7780 proto tcp
```

- [ ] `~/.config/synergos/synergos-net.toml` (A = ホスト):

```toml
# 参加側が最初に叩く /peer-info サーブレット (TCP)
peer_info_listen_addr = "0.0.0.0:7780"

# 参加側は /peer-info を取りに行った先のホストへ QUIC 接続する。
quic_advertised_addr = "0.0.0.0:4433"

# 招待トークンに埋め込む、B から見た A の /peer-info URL
peer_info_advertised_url = "http://<A_MESH_IP>:7780"

# Mesh 運用では NAT 越え probe を省く。
auto_promote = false

[quic]
listen_addr = "[::]:4433"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

[tunnel]
api_token_ref = ""
hostname = ""
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
```

- [ ] systemd unit は SETUP.md §3-B-2 のまま (`XDG_RUNTIME_DIR` + `enable-linger` 必須)。`--config` に上のファイル
- [ ] `synergos-core project open myproj /home/ubuntu/share -n "MyProj"`
- [ ] `synergos-core project invite myproj --expires 86400` → **`syn1.` で始まるトークン**を控える

## 3. B (Windows 開発機) — 人手: インストーラ + 管理者 PowerShell

- [ ] Cloudflare One Client (旧 WARP) をインストール → 設定 → Zero Trust → team name → メール PIN でログイン
- [ ] `ipconfig` で `100.96.x.x` が付いたことを確認 (以下 `<B_MESH_IP>`、B は inbound 不要なので使わなくてよい)
- [ ] `%APPDATA%\Synergos\synergos-net.toml` (B = 参加側):

```toml
# Mesh 運用では NAT 越え probe を省く。
auto_promote = false

[quic]
listen_addr = "[::]:0"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false

[tunnel]
api_token_ref = ""
hostname = ""
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
```

- [ ] Synergos の作業ディレクトリ (main マージ後) で `cargo build --release -p synergos-core`

## 4. 疎通 → 参加 → 同期 (コピペ)

```powershell
# B: A の peer-info に届くか (Mesh 越し)
curl.exe -s http://<A_MESH_IP>:7780/peer-info      # JSON が返れば OK。quic_endpoint は 0.0.0.0:4433 で正常

# B: daemon 起動 + 参加
$env:RUST_LOG="synergos_core=info,synergos_net=info"
.\target\release\synergos-core.exe start --config $env:APPDATA\Synergos\synergos-net.toml
# 別ターミナル
.\target\release\synergos-core.exe project join syn1.eyJ... D:\share\B
.\target\release\synergos-core.exe peer list myproj        # A が Connected

# A: publish → B に届く
synergos-core project publish myproj assets/big.bin
# B: D:\share\B\assets\big.bin が現れる。書き換えて A で再 publish → B が再受信 (version 2)
# B → A も同じ (B で publish)
```

合格条件は two-node-operations.md §2.4 と operations-readiness.md P0 の通し項目
(join → publish → 受信 → 再 publish → 再受信 → 両 daemon 再起動 → 再 publish)。

## 5. 詰まったら

two-node-operations.md §5。Mesh 特有: `curl` が届かない → A の `warp-cli status` / B の Cloudflare One
接続状態 / dashboard の device-to-device 設定 / A の ufw。QUIC だけ通らない → A の ufw で 4433/udp、
B 側は outbound のみなので firewall 不要。

## 6. 依存関係 (順番)

1. この手順に必要な Synergos の変更を main へマージ
2. GitHub リリース — A が `git clone` できるようにするため。
   代替: 管理された SSH 鍵で A へソースをコピー
3. §1〜§3 (人手)
4. §4 (コピペ) — 結果を two-node-operations.md §5 に追記して実機確認を記録
