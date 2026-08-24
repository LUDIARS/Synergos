# Cloudflare Mesh 運用ガイド + 管制サーバー (synergos-control)

Synergos ノード間接続を **Cloudflare Mesh** (旧 WARP Connector + peer-to-peer connectivity)
の上で運用するための構築手順と、クローズド運用でダークノードの出現を抑止する
**管制サーバー `synergos-control`** の設計・使い方。

## 1. なぜ Tunnel ではなく Mesh か

Cloudflare **Tunnel** は hostname → サービスの一方向公開 (HTTP/WebSocket 中心) であり、
Synergos の QUIC (UDP) を通す筋が悪いことが運用検証で確定している (SETUP.md §つまずき表:
「quinn の UDP が通らない。純 UDP/QUIC を Tunnel 越しに通すのは設計上不向き」)。

Cloudflare **Mesh** は参加者全員に `100.96.0.0/12` のプライベート IP を割り当てる
双方向のプライベートネットワークで、**TCP / UDP / ICMP を通す**。つまり:

- Synergos の QUIC 直結 (`peer_bootstrap` → `QuicManager::connect`) が **Mesh IP 宛にそのまま動く**
- NAT 越え問題が消える (全ノードが同一仮想ネットワーク上)
- TURN 未実装のままでよい (Mesh がその役割を吸収)
- Zero Trust のデバイス管理・Gateway ポリシーで「誰が繋がっているか」を統制できる

| | Tunnel | Mesh |
|---|---|---|
| 方向 | 片方向 (hostname 公開) | 双方向 (private IP 同士) |
| プロトコル | HTTP/WS 中心 | TCP / UDP / ICMP |
| Synergos QUIC | ✗ (UDP 不通) | ✓ |
| 参加単位 | サービス | ノード / デバイス |

## 2. Cloudflare Mesh の構築手順 (手動・初回)

前提: Cloudflare アカウントと Zero Trust org (team name) があること。

### 2.1 Mesh の有効化

1. Cloudflare dashboard → **Networking > Mesh** を開く
2. **Add a node** で最初のノードを作成 (セットアップウィザードが以下を自動作成する)
   - デバイスエンロールポリシー (email one-time PIN)
   - Split Tunnels **Include mode** + Mesh レンジ `100.96.0.0/12` のデバイスプロファイル
   - device-to-device 通信の許可設定
   - Gateway proxy (TCP/UDP/ICMP)

手動で確認する場合: **Team & Resources > Devices > Management → Peer to peer connectivity**
で "Allow all Cloudflare One traffic to reach enrolled devices" が有効か、
Split Tunnels が Include mode で `100.96.0.0/12` を含むかを見る。

### 2.2 サーバノード (headless Linux) の参加

dashboard の Add a node が表示するコマンド (Debian/Ubuntu):

```bash
# WARP クライアント導入
curl -fsSL https://pkg.cloudflareclient.com/pubkey.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/cloudflare-warp-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg] https://pkg.cloudflareclient.com/ $(. /etc/os-release && echo $VERSION_CODENAME) main" | sudo tee /etc/apt/sources.list.d/cloudflare-client.list
sudo apt-get update && sudo apt-get install -y cloudflare-warp

# IP forwarding
printf 'net.ipv4.ip_forward = 1\nnet.ipv6.conf.all.forwarding = 1\n' | sudo tee /etc/sysctl.d/99-zzz-cloudflare-warp-connector.conf
sudo sysctl --system

# ノード登録 (TOKEN は dashboard か synergos-control が発行)
sudo warp-cli connector new <TOKEN> && sudo warp-cli connect
```

### 2.3 クライアント端末 (人の PC/スマホ) の参加

1. Cloudflare One Client (旧 WARP) をインストール
2. 設定 → Zero Trust security → **team name** を入力してログイン (エンロールポリシーで許可されたメールのみ通る)
3. 接続すると `100.96.0.0/12` の Mesh IP が割り当てられる

### 2.4 OS ファイアウォール

**Windows は `100.96.0.0/12` からの inbound を既定でブロックする。**
Synergos の QUIC listen ポート (既定 4433/UDP) を Mesh レンジに対して開ける:

```powershell
New-NetFirewallRule -DisplayName "Synergos QUIC (Cloudflare Mesh)" `
  -Direction Inbound -Protocol UDP -LocalPort 4433 `
  -RemoteAddress 100.96.0.0/12 -Action Allow
```

Linux (ufw の例): `sudo ufw allow from 100.96.0.0/12 to any port 4433 proto udp`

### 2.5 Synergos を Mesh 上で動かす

各ノードの `synergos-net.toml`:

```toml
# 自ノードの Mesh IP を advertise する (auto の外部IP自己検出は Mesh では不適切)
quic_advertised_addr = "<自分の Mesh IP>:4433"
peer_info_listen_addr = "0.0.0.0:7777"

# 既知ノード (常駐サーバ等) の Mesh IP を bootstrap に指定
bootstrap_urls = ["http://<相手の Mesh IP>:7777/peer-info"]

[tunnel]
hostname = ""   # cloudflared spawn は使わない (Mesh に置き換え)
```

`/peer-info` は Mesh 内からしか到達できない (bind を Mesh IP にすればさらに絞れる)。
これで「クローズドな Mesh の中だけで Synergos P2P が完結する」構成になる。

## 3. 管制サーバー synergos-control

### 3.1 目的

クローズド運用で**ダークノード** (誰のものか分からない参加者) の出現を抑止する。

- ノード情報を**組織別**に管理し、「誰のノードが組織内にあるか」を一元把握する
- Cloudflare 設定 (Mesh connector の作成・トークン発行・削除) を**自動化**する
- Cloudflare 側の実態とレジストリを**突合 (reconcile)** し、未登録の参加者を検出・失効する

### 3.2 アーキテクチャ

```
 管理者/自動化 ──HTTP(S)──▶ synergos-control (127.0.0.1:4250)
                              │  ├─ レジストリ (org / node / owner) — JSON 永続化
                              │  ├─ Cloudflare API client
                              │  └─ reconcile (dark node 検出)
                              ▼
                       Cloudflare API v4
                         ├─ /accounts/{id}/warp_connector           (Mesh node 作成/削除/トークン)
                         └─ /accounts/{id}/devices/registrations    (端末一覧/失効)
```

- 管理 API は **loopback bind が既定** (管理面を外部公開しない)。全エンドポイントは
  bearer トークン必須 (`SYNERGOS_CONTROL_ADMIN_TOKEN`)
- **注意**: daemon heartbeat (§3.5) を使うには bind を Mesh IP まで広げる必要があり、
  管理 API も同じリスナー上で Mesh 全体から到達可能になる。この構成では
  管理トークンが唯一の防壁になるため、(a) 十分長いランダム token を使う、
  (b) `bind_addr` は `0.0.0.0` ではなく **管制サーバーの Mesh IP に限定**する、
  (c) Gateway ポリシーで 4250/TCP の到達元を絞る、のいずれか (できれば全部) を行う。
  heartbeat を使わない運用では loopback のままにする
- Cloudflare API token は環境変数 `CLOUDFLARE_API_TOKEN` からのみ読む (設定ファイル直書き禁止)
- connector 登録トークンはレスポンスで一度返すだけで**保存しない**

### 3.3 起動

```bash
cp synergos-control/control.example.toml control.toml   # account_id を記入
export SYNERGOS_CONTROL_ADMIN_TOKEN=$(openssl rand -hex 32)
export CLOUDFLARE_API_TOKEN=<token>
synergos-control serve --config control.toml
```

必須環境変数が無い場合は起動を拒否する (フォールバックしない)。

### 3.4 API

すべて `Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN` が必要 (health を除く)。

| Method | Path | 説明 |
|---|---|---|
| GET | `/v1/health` | 稼働確認 (無認証) |
| POST | `/v1/orgs` | 組織作成 `{id, name, members[]}` (members = 許可メール) |
| GET | `/v1/orgs` | 組織一覧 |
| GET/PUT | `/v1/orgs/{org}` | 組織取得 / 更新 (メンバー変更) |
| POST | `/v1/orgs/{org}/nodes` | ノード登録。`kind=mesh_node` なら **CF connector 自動作成 + 登録トークン返却** |
| GET | `/v1/orgs/{org}/nodes` | 組織のノード一覧 (誰のノードがあるか) |
| GET/PATCH/DELETE | `/v1/orgs/{org}/nodes/{id}` | 取得 / 更新 (peer_id, mesh_ip 記録) / 削除 (**CF connector も削除**) |
| POST | `/v1/orgs/{org}/nodes/{id}/connector-token` | 登録トークン再発行 |
| POST | `/v1/orgs/{org}/nodes/{id}/node-key` | heartbeat 用 node key 再発行 (旧キー即無効) |
| POST | `/v1/reconcile` | **dark node 検出**。`{"revoke_dark": true}` で失効まで実施 |
| GET | `/v1/mesh/context` | UI 用。対象アカウント / API base を返す (秘密情報なし) |
| POST | `/v1/mesh/token-check` | リクエストで渡した Cloudflare API token の検証 (保存しない) |
| POST | `/v1/mesh/reconcile` | 同トークンでの突合 |
| POST | `/v1/mesh/connector-tokens` | 同トークンで組織内 Mesh node の登録トークンを一括発行 |

`POST /v1/heartbeat` だけは管理トークンではなく node key (Bearer) で認証する
(ノード自身が叩くため)。

`revoke_dark` は指定した Cloudflare account 全体を Synergos 専用の管理境界とみなし、
レジストリに無い connector と、どの org の member にも含まれない端末を失効する。
他用途の Zero Trust 利用者・connector と同居する account では実行しないこと。

### 3.5 運用フロー

**ノード追加 (サーバ):**

```bash
curl -s -X POST http://127.0.0.1:4250/v1/orgs/acme/nodes \
  -H "Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"display_name":"build-server-1","owner_email":"alice@acme.test","kind":"mesh_node"}'
# → {"node":{...},"connector_token":"...","enroll_hint":"..."}
# 返ってきた connector_token をノードで:
#   sudo warp-cli connector new <connector_token> && sudo warp-cli connect
```

owner_email が組織 members に居ないと登録できない (所有者不明ノードを作らせない)。

**端末追加 (人):** 組織 members にメールを追加 → 本人が Cloudflare One Client でエンロール。
reconcile がメールで突合し、members に無いメールの端末は dark 扱いになる。

**daemon heartbeat (peer_id ↔ Mesh IP 照合):**

ノード登録レスポンスの `node_key` を daemon 側に配布し、`synergos-net.toml` に追記する:

```toml
[control]
heartbeat_url = "http://<管制サーバーの Mesh IP>:4250/v1/heartbeat"
node_id = "<登録時に返った node.id>"
node_key_env = "SYNERGOS_NODE_KEY"   # キー本体は環境変数で渡す
interval_secs = 60
```

daemon は起動時に設定を検証し (`heartbeat_url` があるのに node_id / キーが無ければ
起動エラー)、以後 60 秒ごとに `{node_id, peer_id, mesh_ip, synergos_version}` を報告する。
mesh_ip はローカル NIC から `100.96.0.0/12` を自己検出した値。
管制サーバーはこれで **synergos の peer_id と Mesh IP がレジストリに揃う**。
node key を漏らした場合は `POST /v1/orgs/{org}/nodes/{id}/node-key` で再発行 (旧キー即無効)。

**Mesh IP の収集と照合:**

- Mesh node: heartbeat がローカル NIC から検出した IP を `reported_mesh_ip` として記録する
- 端末: `devices/registrations` の `virtual_ipv4` をレポートに含める
- connector 一覧 API は割当 Mesh IP を返さない。また `teamnet/routes` は任意の
  サブネットルートであって connector 自身の割当 IP ではないため、正本には使用しない
- 管理者が確認済みの Mesh IP を node の `mesh_ip` へ PATCH しておくと、heartbeat の
  自己申告 IP と食い違うノードを `mesh_ip_mismatches` として報告する

**ダークノード点検 (定期実行を推奨):**

```bash
# レポートのみ
curl -s -X POST http://127.0.0.1:4250/v1/reconcile \
  -H "Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN"

# 検出した dark を失効・削除まで実施 (破壊的 — レポート確認後に)
curl -s -X POST http://127.0.0.1:4250/v1/reconcile \
  -H "Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN" \
  -H "Content-Type: application/json" -d '{"revoke_dark":true}'
```

判定:

| 分類 | 意味 | 対処 |
|---|---|---|
| `known_connectors` / `known_devices` | レジストリと一致 | — |
| `dark_connectors` | CF に居るが未登録の Mesh node | 調査 → 登録 or `revoke_dark` で削除 |
| `dark_devices` | どの組織メンバーでもない端末 | 調査 → members 追加 or 失効 |
| `missing_connectors` | 登録済みだが CF に実体が無い | dashboard 手動削除等。ノード再発行 or レジストリ削除 |

### 3.6 管理コンソール (Web UI) からの操作

`synergos-control` は管理 Web UI (`synergos-admin-ui` / Dioxus 0.7 の WASM アプリ) を
`/ui/` から静的配信できる。ビルドと配信設定は [admin-ui.md](admin-ui.md) を参照。

```bash
cd synergos-admin-ui && dx build --release --platform web && cd ..
# control.toml に [ui] dist_path を設定してから
synergos-control serve --config control.toml
# → http://127.0.0.1:4250/ui/
```

初回アクセスで `SYNERGOS_CONTROL_ADMIN_TOKEN` を入力する。値はブラウザの
`sessionStorage` にだけ保持され、全 API 呼び出しに Bearer として付く。

**ノード追加 (UI):**

1. 「組織 / ノード管理」で対象組織を選ぶ
2. 登録フォームに表示名 / 所有者メール / 種別 (Mesh node か Client device) を入れて登録
3. 返ってきた `connector_token` と `node_key` をコピーする
   (control には保存されないため、画面を離れる前にコピーする)
4. 画面の導線からガイド「5. ノードをエンロールする」を開き、ノード上で
   `sudo warp-cli connector new <connector_token> && sudo warp-cli connect` を実行

登録トークンを紛失したら、一覧の「トークン再発行」で再取得できる
(`POST /v1/orgs/{org}/nodes/{id}/connector-token` と同じ)。

**Mesh 自動設定 (UI):**

「Mesh 自動設定」タブで Cloudflare API トークンを入力すると、次の 3 ステップを
進捗表示付きで順に実行する。

| step | 叩く API | 内容 |
|---|---|---|
| 1 | `POST /v1/mesh/token-check` | トークンの有効性 (`user/tokens/verify`) と、アカウント配下の Mesh node を実際に読めるかを確認 |
| 2 | `POST /v1/mesh/reconcile` | レジストリと Cloudflare の突合 (dark node 検出)。**レポートのみ** |
| 3 | `POST /v1/mesh/connector-tokens` | 対象組織の Mesh node へ配る登録トークンをまとめて発行 |

これらの API は起動時 env の `CLOUDFLARE_API_TOKEN` ではなく、**リクエストで受け取った
トークン**をそのリクエストの処理中だけ使う。保存もログ出力もしない
(応答にも含めない)。いずれも管理トークン層の内側にあり、トークンの持ち込み口を
無認証にはしていない。

UI からは `revoke_dark` を行わない。失効は破壊的なので §3.5 の curl から明示的に実行する。

**ダッシュボード:** 組織ごとのノード数・Mesh node 数・heartbeat 未着数を表示し、
「dark node を点検」で `POST /v1/reconcile` (起動時 env のトークンを使う、レポートのみ)
を実行できる。

**セットアップガイド:** getting-started.md / two-node-operations.md / このドキュメントの
手順を、操作順のステップ形式でアプリ内に表示する。OS タブ (Windows / Linux / macOS) で
コマンドが切り替わり、各ブロックはコピーできる。各画面の主要操作に付いた「?」から
対応する節へ飛べる。

### 3.7 抑止の考え方 (予防 + 検出)

1. **予防**: connector トークンは管制サーバー経由でのみ発行し、発行時に必ず org/owner が
   記録される。端末エンロールはエンロールポリシー (メール) で制限し、そのメールは
   org members と一致させる
2. **検出**: reconcile を定期実行し、dashboard 直操作や共有トークン流用で生まれた
   未登録参加者を dark として炙り出す
3. **排除**: `revoke_dark` で CF 側から失効させる (Synergos 層でも到達不能になる)

## 4. FAQ (2026-07-31 neco Q&A より)

### Q. Mesh は Windows 対応していない?

**参加はできる。参加の仕方が Linux と違うだけ。**

| 参加形態 | 対応 OS | 参加方法 |
|---|---|---|
| Mesh node (headless, `warp-cli connector new`) | **Linux のみ** (Ubuntu 22.04+/Debian 12+/RHEL 9+/Fedora) | connector トークン |
| Client device | **Windows / macOS / Linux / iOS / Android** | Cloudflare One Client でエンロール |

client device にも Mesh IP が割り当てられ、**device-to-device で双方向通信できる**
("Reach any enrolled device by its Mesh IP. No Mesh nodes involved")。
Synergos の QUIC P2P は Windows 機同士でも Mesh 越しに動く。

Windows 固有の注意:
1. Windows Firewall が Mesh レンジからの inbound を既定ブロック → §2.4 のルール追加が必須
2. エンロールは対話式 (ブラウザ + メール認証)。無人 Windows サーバは MDM + service token が
   必要になるため、無人常駐は Linux (Mesh node) にするのが楽

### Q. 最初のノードは絶対 Linux?

**いいえ。** 公式に "Mesh nodes are optional" / "Client-to-client connectivity works
without any Mesh nodes" と明記されている。セットアップウィザードは初回に一度回す必要が
あるが、ノードインストール工程はスキップ可能。**Windows 機 2 台だけで開始できる**。
Linux node が要るのは無人常駐サーバやサブネットルーティングが欲しくなってからで、後付け可能。

### Q. お金はかかる?

**基本無料。課金軸は「ユーザー数」、ノード数は課金ではなく上限。**

- Free: **50 ユーザー + 50 Mesh node** まで無料 (全 Cloudflare アカウント付属)
- ユーザー 51 人目以降: 認証が**ブロック**される (勝手に課金されない)。継続するには
  Pay-as-you-go **$7/ユーザー/月** へ移行 (全シート課金、例: 60 人 → $420/月)。
  有料化でログ保持 24h → 30 日等の付加あり
- Mesh node 50 台は**ハードキャップ** (超過分を買う従量メニューは非公開、それ以上は
  Enterprise 相談)。人の端末は node 数にカウントされない
- Mesh トラフィックへの従量課金はなし (2026-07 時点)
- 1 人が複数端末を持ってもユーザーとしては 1 カウント
- 別途かかるのは AWS に Linux 常駐ノードを置く場合の EC2 代程度 (t4g.small で月 $12 前後)

LUDIARS 想定 (数十人 + サーバ数台) なら当面 $0。最初の課金判断ポイントはユーザー 50 人の壁。

## 5. 制限・今後

- 端末とレジストリの突合キーはメールのみ。デバイス単位の厳密な突合 (serial / device id
  の事前登録) は必要になったら追加
- 端末 (ClientDevice) の virtual_ipv4 はレポート表示のみ (端末は人単位・複数台に
  なりうるため)。Mesh node の割当 IP も connector 一覧 API からは取得できないため、
  heartbeat の報告値を記録し、必要なら管理者設定の期待値と照合する
- heartbeat は HTTP 平文を想定 (Mesh 内通信)。Mesh 外へ出す場合は HTTPS 終端を挟む

参考:
- https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-mesh/
- https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-mesh/get-started/
- https://developers.cloudflare.com/api/resources/zero_trust/subresources/tunnels/subresources/warp_connector/
- https://developers.cloudflare.com/api/resources/zero_trust/subresources/devices/subresources/registrations/
