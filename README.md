# Synergos

**Synergos**（シュネルゴス）— クロスプラットフォーム対応リアルタイムコラボレーション基盤

> ギリシャ語 *synergos*（共に働く）に由来。複数のユーザーがネットワーク越しにファイルリソースを高速にやり取りし、共同作業を行うための基盤を提供します。

**対応プラットフォーム:** Windows / Linux / macOS

## 概要

Synergos は **4つのレイヤ** で構成されます:

| レイヤ | クレート名 | 実行形態 | 責務 |
|--------|-----------|---------|------|
| **Network Foundation** | `synergos-net` | ライブラリ | Cloudflare Tunnel / QUIC / IPv6 Direct / TURN によるトランスポート基盤 |
| **IPC Protocol** | `synergos-ipc` | ライブラリ | IPC コマンド・レスポンス・イベント型定義（全クライアントが共有） |
| **Core Daemon** | `synergos-core` | **常駐バイナリ** | EventBus・IPCサーバー・CLI制御・ファイル転送・プレゼンス管理 |
| **GUI Application** | `synergos-gui` | **GUIバイナリ** | ネットワークモニター・転送管理・ピア管理（SourceTree的な専用GUI） |
| **Ars Plugin** | `ars-plugin-synergos` | ライブラリ | IPC経由でcoreに接続する薄いアダプタ。Ars EventBus ↔ IPC ブリッジ |

### git ↔ SourceTree モデル

Synergos は **git と SourceTree の関係** をモデルとした設計です:

```
  git (CLIツール)           ←→  synergos-core (常駐デーモン)
  SourceTree (GUIアプリ)    ←→  synergos-gui (専用GUIアプリ)
  IDE内蔵git機能            ←→  ars-plugin-synergos (Arsプラグイン)
```

Core は Ars に一切依存せず、独立した常駐アプリケーションとして動作します。

### 主要機能

- **Cloudflare Tunnel 経由のセキュアな外部接続** — NAT/ファイアウォールの内側からでも外部ユーザーとP2P的に接続
- **QUIC プロトコルによる高速ファイル転送** — 低レイテンシ・多重化・0-RTT対応
- **IPFS ファイルストリーム** — コンテンツアドレッシング (CID) による重複排除・Merkle DAG で差分転送
- **ファイルサイズ別ストリーム帯域制御** — Large: 60%, Medium: 30%, Small: 10% の動的帯域配分
- **クロスプラットフォーム IPC** — Unix Domain Socket (Linux/macOS) / Named Pipe (Windows)
- **専用 GUI アプリケーション** — Ars がなくてもネットワーク状態・転送・コンフリクトを管理可能
- **常駐デーモン** — バックグラウンドで動作し、複数クライアントからの同時接続をサポート
- **オプトアウト原則** — Synergos が存在しなくても Ars は完全に動作する

## アーキテクチャ

```
┌──────────────────────────┐   ┌──────────────────────────────┐
│     synergos-gui          │   │    ars-plugin-synergos        │
│     (専用GUIアプリ)        │   │    (Ars薄IPCアダプタ)         │
└─────────────┬────────────┘   └──────────────┬───────────────┘
              │ IPC                            │ IPC
              └───────────────┬────────────────┘
                              │
              ┌───────────────▼──────────────────────────────┐
              │         synergos-core (常駐デーモン)           │
              │                                               │
              │  EventBus │ IPC Server │ CLI │ Service Layer  │
              │  Exchange │ Presence │ Conflict │ Catalog     │
              ├──────────────────────┬───────────────────────┤
              │                      ▼                       │
              │         synergos-net (ネットワーク基盤)        │
              │  Conduit │ Tunnel │ Mesh │ QUIC │ DHT │ Gossip│
              └──────────────────────────────────────────────┘

              ┌──────────────────────────────────────────────┐
              │     synergos-ipc (共有IPCプロトコル定義)       │
              └──────────────────────────────────────────────┘
```

## 技術スタック

| コンポーネント | 技術 | レイヤ |
|---------------|------|--------|
| トランスポート | QUIC (quinn crate) | Network Foundation |
| トンネリング | Cloudflare Tunnel (cloudflared) | Network Foundation |
| ファイルストリーム | IPFS (CID / Merkle DAG / Bitswap) | Network Foundation |
| IPv6 / NAT traversal | TURN / STUN + 自前FQDN解決 | Network Foundation |
| DNS | hickory-resolver (DoH) | Network Foundation |
| 暗号化 | TLS 1.3 (rustls) | Network Foundation |
| シリアライズ | Protocol Buffers (ネットワーク) / MessagePack (IPC) | 複数レイヤ |
| IPC | Unix Domain Socket / Named Pipe | IPC Protocol |
| CLI | clap | Core Daemon |
| EventBus | tokio::sync::broadcast | Core Daemon |
| GUI | egui + eframe | GUI Application |
| Ars統合 | ProjectModule / EventBus | Ars Plugin |
| ランタイム | Tokio async | 全レイヤ |

## CLI 使用法

```bash
# デーモン起動
synergos-core start

# デーモン状態確認
synergos-core status

# プロジェクト管理
synergos-core project open <id> <path>
synergos-core project list
synergos-core project close <id>

# ピア管理
synergos-core peer list <project>
synergos-core peer connect <project> <peer>

# 転送管理
synergos-core transfer list
synergos-core transfer cancel <id>

# ネットワーク状態
synergos-core network

# デーモン停止
synergos-core stop
```

## ディレクトリ構成

```
Synergos/
├── README.md
├── DESIGN.md                  # 詳細設計書
├── LICENSE
├── Cargo.toml                 # Workspace root
│
├── synergos-net/              # Network Foundation Layer
│   ├── Cargo.toml
│   └── src/
│
├── synergos-ipc/              # IPC Protocol Layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs             # 公開API
│       ├── command.rs         # IpcCommand 定義
│       ├── response.rs        # IpcResponse 定義
│       ├── event.rs           # IpcEvent 定義
│       ├── transport.rs       # IPC トランスポート抽象化
│       └── client.rs          # IpcClient
│
├── synergos-core/             # Core Daemon Layer (常駐バイナリ)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs            # エントリポイント
│       ├── daemon.rs          # デーモンライフサイクル
│       ├── event_bus.rs       # 内部 EventBus
│       ├── ipc_server.rs      # IPC サーバー
│       ├── cli.rs             # CLI コマンドハンドラ
│       ├── project.rs         # プロジェクト管理
│       ├── exchange/          # ファイル転送制御
│       ├── presence/          # ピア状態管理
│       └── conflict/          # コンフリクト管理
│
├── synergos-gui/              # GUI Application (スタンドアロンバイナリ)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs            # エントリポイント
│       ├── app.rs             # アプリケーション状態
│       ├── connection.rs      # Core IPC 接続管理
│       └── ui/                # UI コンポーネント
│
├── ars-plugin-synergos/       # Ars Plugin Layer (薄IPCアダプタ)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs             # プラグインエントリポイント
│       ├── events.rs          # Ars EventBus イベント定義
│       └── bridge.rs          # IpcEvent → EventBus ブリッジ
│
└── tests/
```

## ライセンス

MIT License
