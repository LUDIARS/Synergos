# プラットフォーム対応

Synergos は **Windows / Linux / macOS** の 3 OS をネイティブサポートしています。
CI (GitHub Actions) は `Test (windows-latest)` / `Test (ubuntu-latest)` / `Test (macos-latest)` を毎コミット並列実行して全部 green を維持しており、2026-04-23 時点で 171 tests 全合格。

## 共通 / OS 固有の切り分け

プラットフォーム固有コードは **IPC トランスポート層のみ** で、他 (QUIC / gossip / bitswap / catalog sync / GUI) は完全に共通です。

| 層 | Windows | Linux | macOS |
|---|---|---|---|
| **IPC 経路** | Named Pipe (`\\.\pipe\synergos-core`) | Unix Domain Socket (`$XDG_RUNTIME_DIR/synergos-core.sock`、無ければ `/tmp/synergos-core-<uid>.sock`) | 同左 |
| **IPC 権限** | Named Pipe ACL で呼び出しユーザ限定 | `chmod 0600` + SO_PEERCRED (via `libc::geteuid`) で UID チェック | 同左 |
| **QUIC (quinn)** | ○ | ○ | ○ |
| **Cloudflare Tunnel** | cloudflared.exe が PATH 必須 | cloudflared が PATH 必須 | cloudflared が PATH 必須 |
| **GUI (egui/eframe/glutin)** | ○ (Direct3D / OpenGL) | ○ (X11 / Wayland、OpenGL 必須) | ○ (Metal 非使用、OpenGL) |
| **Identity 保存先** | `%APPDATA%/synergos/identity` | `$XDG_CONFIG_HOME/synergos/identity` (既定 `~/.config/synergos/`) | `~/Library/Application Support/synergos/identity` |

## ビルド

全 OS 共通コマンド:

```bash
cargo build --release --workspace
```

成果物の拡張子だけ OS で変わります:

| OS | core / gui / relay バイナリ |
|---|---|
| Windows | `target/release/synergos-core.exe` 他 |
| Linux | `target/release/synergos-core` 他 (拡張子なし) |
| macOS | 同左 |

### Rust ツールチェーン

- 最低要件: Rust **stable 1.89+** (Cargo edition 2021)
- CI / clippy は Rust 1.95 で回しているので、ローカルも最新 stable が推奨
- `rustup update stable` で揃う

### 依存システムライブラリ

**Windows**
- とくに不要 (`windows-sys` / `windows-core` は Cargo 経由で取得)
- Cloudflare Tunnel を使うなら cloudflared.exe を PATH に入れる

**Linux**
- 開発パッケージ (例: Ubuntu):
  ```bash
  sudo apt install build-essential pkg-config libssl-dev \
                   libxkbcommon-dev libwayland-dev libxcb1-dev \
                   libglu1-mesa-dev
  ```
- Cloudflared: `wget https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 -O cloudflared && chmod +x cloudflared && sudo mv cloudflared /usr/local/bin/`

**macOS**
- Xcode Command Line Tools: `xcode-select --install`
- Cloudflared: `brew install cloudflared`
- GUI は OpenGL 4.x で動作。Apple Silicon でも問題なく動く。

## 混在構成の注意

`project join` を経由した参加なら、**Windows + Linux + macOS の混在でもそのまま通信できます**。
gossip メッセージは msgpack (バイト列) で OS 非依存、QUIC は TLS1.3 + ring/aws-lc-rs で揃っているので、
署名・暗号化の互換性問題は発生しません。

ただし以下だけ注意:

- **パス区切り**: `project open` に渡す `<local-path>` は各ノードの OS ネイティブパスで書く。
  Windows 側で `D:/a`、Linux 側で `/home/user/a` でもプロジェクト自体は同じ。
- **ファイル名の大文字小文字**: Linux は case-sensitive、macOS (既定の APFS) と Windows (NTFS) は
  case-insensitive。大文字違いのファイルを混ぜると同期時にコンフリクトになり得るので、
  命名規約を統一するのが無難。
- **改行コード**: Synergos はバイト列を content-addressed (CID = blake3) で扱うので、
  CRLF ↔ LF 変換で CID が変わります。git の `core.autocrlf` のような変換は入りません。

## OS ごとの起動メモ

### Linux (systemd でバックグラウンド化したい場合)

Synergos 自体は systemd unit を同梱していませんが、`synergos-core start` がフォアグラウンド
プロセスなので簡易 unit で包めます:

```ini
# ~/.config/systemd/user/synergos-core.service
[Unit]
Description=Synergos Core Daemon

[Service]
ExecStart=%h/bin/synergos-core start
Environment=RUST_LOG=synergos_core=info
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now synergos-core
```

### macOS (launchd)

`~/Library/LaunchAgents/io.ludiars.synergos-core.plist` に `ProgramArguments` で
`synergos-core start` を指定。詳細は Apple の launchd ドキュメント参照。

### Windows (サービス化)

現状 Windows サービス化のサポートはありません。
スタートアップフォルダに `synergos-core.exe start` を置くか、NSSM / winsw 等で
ラップするのが簡単です。GUI だけ使うなら通常のショートカットで問題なし。

## トラブルシューティング (OS 別)

| OS | 症状 | 対処 |
|---|---|---|
| Windows | `synergos-gui` がウィンドウを出さず終了 | GPU ドライバ / OpenGL 対応を確認。リモートデスクトップ越しでは `glutin` が GL コンテキストを張れないことがある |
| Linux (Wayland) | GUI 起動時に `failed to initialize winit` | `WAYLAND_DISPLAY` が立っていない / X11 fallback が無効。`GDK_BACKEND=x11` を付けて起動 |
| Linux | `cargo build` でリンクエラー (libssl) | 上記 apt パッケージを入れ直す |
| macOS | `cloudflared` が "command not found" | `brew install cloudflared` 済か確認、`brew doctor` で PATH 確認 |
| 全 OS | 2 ノードが peer を見つけない | DHT / gossip の propagation は数秒かかる。`peer list` を 10 秒ほど待って再実行 |

## 関連ドキュメント

- [getting-started.md](getting-started.md): ビルド + 起動 + 最小 2 ノード手順
- [projects-and-peers.md](projects-and-peers.md): CLI リファレンスと auto-sync 経路の詳細
- [../DESIGN.md](../DESIGN.md): 内部アーキテクチャ (Exchange / CatalogSync / Bitswap など)
