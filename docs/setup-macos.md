# macOS セットアップガイド

Synergos を macOS で導入し、既存の Windows / Linux ノードと同じプロジェクトに
参加して 2 台運用に加わるまでの通し手順。プラットフォーム差分の一覧は
[platforms.md](platforms.md) を、複数マシンでの運用フロー全般は
[two-node-operations.md](two-node-operations.md) を参照。

## 1. 前提

- **対応 macOS バージョン**: Rust の `stable` ツールチェーンと `cargo build` が通る
  バージョンであれば動作する。CI は `macos-latest` (GitHub Actions) でワークスペースの
  ビルド・テストを行う。開発は最新の macOS を推奨するが、特定バージョンへの固定要件はない。
- **CPU アーキテクチャ**: **Apple Silicon (arm64) / Intel (x86_64) の両方**に対応。
  Xcode Command Line Tools のリンカ・ツールチェーンが、ホストの CPU
  アーキテクチャ向けにネイティブビルドする。

## 2. ビルド

### 2.1 Xcode Command Line Tools

```bash
xcode-select --install
```

### 2.2 Rust ツールチェーン (rustup)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
. "$HOME/.cargo/env"
rustup update stable
```

- リポジトリは Rust の最低バージョンを固定していない。CI と同じ最新 `stable` を推奨。

### 2.3 ビルド

```bash
git clone https://github.com/LUDIARS/Synergos && cd Synergos
cargo build --release --workspace
```

GUI が不要なら `-p synergos-core` だけに絞ってもよい (`cargo build --release -p synergos-core`)。

### 2.4 生成物の場所

Linux と同様、拡張子なしで `target/release/` 配下に生成される:

| バイナリ | 役割 |
|---|---|
| `target/release/synergos-core` | 常駐デーモン (CLI 兼用) |
| `target/release/synergos-gui` | GUI クライアント (egui) |
| `target/release/synergos-relay` | 中継用 WebSocket リレー (通常は不要) |

## 3. Gatekeeper / quarantine (未署名バイナリの実行許可)

自前でビルドしたバイナリはローカル生成物なので quarantine 属性は付かず、通常は
そのまま実行できる。一方、**GitHub Actions の Artifact や zip などネットワーク経由
で受け取ったバイナリ**には macOS が `com.apple.quarantine` 属性を付与し、Gatekeeper
がブロックする。その場合の許可手順:

```bash
# 属性の確認
xattr -l target/release/synergos-core

# quarantine 属性だけを外す (署名なしバイナリを自分の判断で実行する場合)
xattr -d com.apple.quarantine target/release/synergos-core
xattr -d com.apple.quarantine target/release/synergos-gui
```

`xattr -d` を使わず GUI で許可する場合:

1. Finder でバイナリ (または `.app`) を右クリック → **開く** を選ぶ
2. 「開発元を確認できないため開けません」ダイアログで **開く** を選ぶ
   (通常のダブルクリックでは選択肢が出ないので、右クリック→開くを使う)
3. 一度許可すれば以降はダブルクリック / 通常実行で起動できる

> 実行属性 (`chmod +x`) が必要な場合は `chmod +x target/release/synergos-core` を
> 先に行う。

## 4. daemon 常駐 (launchd)

macOS には systemd が無いため、常駐化には **launchd** を使う。

### 4.1 plist ファイル (実例)

`~/Library/LaunchAgents/com.ludiars.synergos-core.plist` を作成する
(パスはすべて実際の配置場所に置き換えること):

```bash
mkdir -p ~/.config/synergos ~/Library/LaunchAgents ~/Library/Logs
touch ~/.config/synergos/synergos-net.toml
```

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ludiars.synergos-core</string>

    <key>ProgramArguments</key>
    <array>
        <string>/Users/YOUR_USER/Synergos/target/release/synergos-core</string>
        <string>start</string>
        <string>--config</string>
        <string>/Users/YOUR_USER/.config/synergos/synergos-net.toml</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/Users/YOUR_USER/Synergos</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>synergos_core=info,synergos_net=info</string>
    </dict>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <key>StandardOutPath</key>
    <string>/Users/YOUR_USER/Library/Logs/synergos-core.log</string>

    <key>StandardErrorPath</key>
    <string>/Users/YOUR_USER/Library/Logs/synergos-core.err.log</string>
</dict>
</plist>
```

`YOUR_USER` とバイナリ / 設定ファイルのパスは実環境に合わせて書き換える。
`--config` は必須ではないが (省略時デフォルトの探索はしない設計)、常駐運用では
明示的に渡すことを推奨する。空の設定ファイルは既定値で起動でき、Mesh に参加する
場合は §7 の設定を追記する。

### 4.2 launchctl での起動・停止操作

macOS Ventura 以降で推奨される `bootstrap` / `bootout` サブコマンドを使う
(`load` / `unload` は非推奨):

```bash
# ログディレクトリを先に用意 (StandardOutPath / StandardErrorPath 用)
mkdir -p ~/Library/Logs

# 登録 + 起動 (gui/<uid> ドメインに bootstrap)
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.ludiars.synergos-core.plist

# 状態確認
launchctl print gui/$(id -u)/com.ludiars.synergos-core

# 停止 + 登録解除
launchctl bootout gui/$(id -u)/com.ludiars.synergos-core

# plist を書き換えた後の再読み込み (bootout → bootstrap の順)
launchctl bootout gui/$(id -u)/com.ludiars.synergos-core
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.ludiars.synergos-core.plist
```

### 4.3 ログの場所

上記 plist の `StandardOutPath` / `StandardErrorPath` に指定した通り:

- 標準出力: `~/Library/Logs/synergos-core.log`
- 標準エラー (daemon の通常ログ): `~/Library/Logs/synergos-core.err.log`

`tail -f ~/Library/Logs/synergos-core.err.log` で通常ログを追跡できる。

## 5. IPC (Unix Domain Socket)

macOS の IPC トランスポートは Windows の Named Pipe とは異なり、**Unix Domain
Socket** を使う (`synergos-ipc` クレート, `synergos-ipc/src/transport.rs`)。

ソケットパスは固定で、`$HOME` から以下のように解決される:

```
~/Library/Application Support/Synergos/synergos.sock
```

確認方法:

```bash
ls -la "$HOME/Library/Application Support/Synergos/synergos.sock"
```

daemon が起動していればソケットファイルが存在する。`synergos-core status` /
`synergos-core stop` などの CLI サブコマンドは、daemon を起動したのと**同じユーザ**
から呼び出す必要がある (パーミッションが `chmod 0600` + 呼び出し元 UID の一致で
絞られているため)。

> identity 鍵の保存先も同じ `Application Support` 配下: `~/Library/Application
> Support/Synergos/identity.key`。消すと `peer_id` が変わるので注意。

## 6. プロジェクト参加 (`synergos-core project join`)

mac 側の手順そのものは他 OS と同じ CLI 操作。[two-node-operations.md](two-node-operations.md)
の内容を macOS 前提で具体化する。

### 6.1 招待トークンをホスト (A) から受け取る

ホスト側 (Windows / Linux いずれでも可) で発行された招待トークンは
`syn1.` から始まる文字列 (base64url エンコードされた JSON と署名で、招待元の
`peer-info` URL などが埋め込まれている):

```
syn1.<payload>.<signature>
```

`syn1.` で始まらず UUID 単体のトークンが渡された場合、そのホストの
`peer_info_advertised_url` 未設定が原因。ホスト側で
`synergos-core project invite <id> --url http://<ホスト>:7780` を付けて再発行して
もらう。

### 6.2 mac 側で daemon を起動して参加

```bash
# 1. daemon をフォアグラウンドで起動 (常駐化は §4 を参照)
./target/release/synergos-core start --config ~/.config/synergos/synergos-net.toml

# 2. 別ターミナルで招待トークンを使って参加
mkdir -p ~/projects/myproj
./target/release/synergos-core project join syn1.eyJ... ~/projects/myproj

# 3. 接続確認
./target/release/synergos-core project list
./target/release/synergos-core peer list <project-id>  # ホストの行が表示されれば成功
```

`join` が `could not reach host` で失敗する場合、ホスト側のファイアウォール /
IP 設定を直したうえで、mac 側から
`synergos-core peer add-url <project-id> http://<ホスト>:7780` を実行すれば接続だけ
やり直せる (プロジェクト自体は既に mac 側に open 済みのため)。

## 7. Cloudflare Mesh 参加

インターネット越しに (LAN 外の) ノードと接続する場合は Cloudflare Mesh を使う。
詳細な構築・運用は [mesh-operations.md](mesh-operations.md) を参照。macOS は
**Client device** 参加 (`warp-cli` によるヘッドレス connector ではなく Cloudflare
One Client によるエンロール) が対応形態になる。

### 7.1 cloudflared の導入 (Cloudflare Tunnel を使う構成の場合)

```bash
brew install cloudflared
cloudflared --version
```

> Cloudflare **Tunnel** は QUIC (UDP) を通さないため、Synergos の QUIC P2P 経路
> には使えない ([mesh-operations.md §1](mesh-operations.md) 参照)。Tunnel は
> `synergos-relay` を外部公開する用途などに限定される。

### 7.2 warp-cli / Cloudflare One Client での Mesh 参加

Mesh node (`warp-cli connector new`) は **Linux 専用**であり、macOS は **Client
device** としてのみ Mesh に参加できる ([mesh-operations.md §4 FAQ](mesh-operations.md)):

1. [Cloudflare One Client](https://developers.cloudflare.com/cloudflare-one/connections/connect-devices/warp/download-warp/) (旧 WARP) を macOS 用にインストール
2. 設定 → Zero Trust security → team name を入力してログイン
   (エンロールポリシーで許可されたメールのみ通る)
3. 接続すると `100.96.0.0/12` の Mesh IP が割り当てられる。

`warp-cli connector new <TOKEN>` はヘッドレス Linux ノード (常駐サーバ) 向けの
コマンドであり、macOS では通常使わない。ただし CLI (`warp-cli`) 自体は
Homebrew 経由でも導入でき、Cloudflare One Client がバックグラウンドで動いている
状態なら `warp-cli status` で Mesh 接続状態や割り当てられた Mesh IP を確認できる:

```bash
brew install --cask cloudflare-warp   # Cloudflare One Client (GUI) を Homebrew Cask で導入する場合
warp-cli status                       # Mesh 接続状態
```

Mesh IP が割り当てられたら、`synergos-net.toml` に mac 自身の Mesh IP を
advertise 設定する ([mesh-operations.md §2.5](mesh-operations.md)):

```toml
quic_advertised_addr = "<自分の Mesh IP>:4433"
peer_info_listen_addr = "0.0.0.0:7780"
bootstrap_urls = ["http://<相手の Mesh IP>:7780/peer-info"]
auto_promote = false

[quic]
listen_addr = "[::]:4433"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1452
enable_0rtt = false
```

`[tunnel]` は省略すると既定の無効状態になる。セクションを書く場合は
[two-node-operations.md §2](two-node-operations.md) の通り全キーを指定する。

## 8. GUI (synergos-gui) の起動方法

daemon (`synergos-core start`) を先に起動しておく必要がある。GUI は IPC 経由で
daemon に接続するため、daemon 未起動だと "failed to connect" になる。

```bash
./target/release/synergos-gui
```

Gatekeeper の未署名警告が出る場合は §3 の手順で許可する。GUI は OpenGL で描画
しており (Metal は未使用)、Apple Silicon / Intel いずれでも動作する。

## 9. トラブルシューティング

| 症状 | 対処 |
|---|---|
| APFS のファイル名大文字小文字 | 既定の APFS ボリュームは **case-insensitive** ([platforms.md](platforms.md))。Linux (case-sensitive) 側と大文字違いのファイル名を混在させると同期時にコンフリクトになり得るので、命名規約を統一する。 |
| ファイアウォール許可ダイアログ | 初回起動時に「ネットワーク着信を許可しますか」というダイアログが出ることがある (システム設定 → プライバシーとセキュリティ → ファイアウォール で管理)。**許可**を選ばないと QUIC (UDP) / peer-info (TCP) の待受ができず、他ノードから `join` / `peer add-url` で到達できない。 |
| `command not found: synergos-core` | `cargo build` の生成物は `target/release/` にあり、PATH には自動で入らない。`./target/release/synergos-core` のように相対/絶対パスで呼ぶか、`cp target/release/synergos-core /usr/local/bin/` などで PATH に置く。 |
| `command not found: cloudflared` | `brew install cloudflared` 済みか確認。`brew doctor` で PATH を確認 ([platforms.md](platforms.md))。 |
| `xcrun: error: invalid active developer path` | Xcode Command Line Tools 未導入。`xcode-select --install` を実行。 |
| 「開発元を確認できないため開けません」で起動できない | §3 の Gatekeeper 手順 (`xattr -d com.apple.quarantine` または Finder 右クリック→開く) を実施する。 |
| `Daemon not running` (CLI から) | daemon を起動したユーザと別ユーザで CLI を呼んでいないか確認 (IPC ソケットの UID チェックで弾かれる)。§5 のソケットパスにファイルが存在するか `ls -la` で確認する。 |

## 10. 検証チェックリスト

手順を終えた状態で以下が通ることを確認する。

```bash
# 1. daemon が起動していること (別ターミナルで daemon を起動済みの前提)
./target/release/synergos-core status
# → "Synergos Core Daemon" と PID / Projects / Connections / Transfers が表示される

# 2. ネットワーク状態が取得できること
./target/release/synergos-core network
# → "Network Status" と Route / Connections / Bandwidth / Latency が表示される

# 3. プロジェクトが参加済みであること (§6 実施後)
./target/release/synergos-core project list
# → join したプロジェクトが一覧に表示される (0 件なら "No active projects.")

# 4. ピア (ホスト) が見えていること
./target/release/synergos-core peer list <project-id>
# → ホストの行が表示される (数秒〜十数秒かかることがある)

# 5. IPC ソケットが正しい場所に存在すること
ls -la "$HOME/Library/Application Support/Synergos/synergos.sock"

# 6. (launchd 常駐にした場合) launchd に登録されていること
launchctl print gui/$(id -u)/com.ludiars.synergos-core | head -5
```

すべて期待通りに出力されれば、mac ノードとしての導入は完了。

## 関連ドキュメント

- [platforms.md](platforms.md): Windows / Linux / macOS の対応状況・IPC 経路差分の一覧
- [two-node-operations.md](two-node-operations.md): 別マシン 2 台での運用手順全般 (LAN / Mesh)
- [mesh-operations.md](mesh-operations.md): Cloudflare Mesh の構築・管制サーバー
- [getting-started.md](getting-started.md): OS 非依存の最短ビルド・起動手順
- [projects-and-peers.md](projects-and-peers.md): CLI コマンドリファレンス
- [../SETUP.md](../SETUP.md): AWS Graviton (Linux ARM64) セットアップガイド
