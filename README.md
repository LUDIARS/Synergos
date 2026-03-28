# Synergos

**Synergos**（シュネルゴス）— Ars エディタ向けリアルタイムコラボレーション基盤

> ギリシャ語 *synergos*（共に働く）に由来。複数のユーザーがネットワーク越しにファイルリソースを高速にやり取りし、共同作業を行うための基盤を提供します。

## 概要

Synergos は **2つのレイヤ** で構成されます:

| レイヤ | クレート名 | 責務 |
|--------|-----------|------|
| **Network Foundation** | `synergos-net` | Cloudflare Tunnel / QUIC / IPv6 Direct / TURN によるトランスポート基盤。Ars に依存しない汎用ネットワークライブラリ |
| **Ars Plugin** | `ars-plugin-synergos` | Ars の `ProjectModule` として統合。EventBus 連携・ファイル転送制御・プレゼンス管理 |

この分離により、ネットワーク基盤は Ars 以外のアプリケーション（モバイルクライアント等）からも再利用可能です。

### 主要機能

- **Cloudflare Tunnel 経由のセキュアな外部接続** — NAT/ファイアウォールの内側からでも外部ユーザーとP2P的に接続
- **QUIC プロトコルによる高速ファイル転送** — 低レイテンシ・多重化・0-RTT対応
- **IPFS ファイルストリーム** — コンテンツアドレッシング (CID) による重複排除・Merkle DAG で差分転送・ストリーミング
- **ファイルサイズ別ストリーム帯域制御** — Large (100MB+): 60%, Medium (1-100MB): 30%, Small (<1MB): 10% の動的帯域配分
- **スピードテストに基づくコネクション数自動決定** — 接続時にプローブし、最適なストリーム並列度とチャンクサイズを算出
- **帯域優先接続** — ネットワーク帯域が大きいピアに優先的に接続し、全体スループットを最大化
- **QUIC アプリケーション層ブロードキャスト** — 複数コネクションへのゼロコピー同時配信
- **IPv6 ダイレクト接続** — 外部 FQDN + TURN サーバーで自前のアドレス解決を行い、NAT を迂回してモバイルネットワークから直接接続。接続方式 (IPv6/Tunnel) は自動判定
- **ネットワークモニタリング** — 接続状況・N コネクション数・帯域使用量・ファイル送受信状況のリアルタイム監視
- **オプトアウト原則** — このプラグインが存在しなくてもArsは完全に動作する

## アーキテクチャ

```
┌────────────────────────────────────────────────────────┐
│              Ars Plugin Layer                            │
│              (ars-plugin-synergos)                       │
│                                                          │
│  ┌───────────┐  ┌──────────┐  ┌────────────────────┐   │
│  │ Exchange  │  │ Presence │  │  Module Integration │   │
│  │ (転送制御) │  │ (状態管理) │  │  (EventBus連携)    │   │
│  └─────┬─────┘  └────┬─────┘  └─────────┬──────────┘   │
│        └──────────┬───┴──────────────────┘               │
│                   │                                      │
├───────────────────┼──────────────────────────────────────┤
│                   ▼                                      │
│         Network Foundation Layer                         │
│              (synergos-net)                               │
│                                                          │
│  ┌──────────┐  ┌────────────┐  ┌─────────────────────┐  │
│  │ Conduit  │  │   Tunnel   │  │       Mesh          │  │
│  │ (接続管理) │  │  Manager   │  │ (IPv6 Direct/TURN)  │  │
│  └─────┬────┘  └──────┬─────┘  └──────────┬──────────┘  │
│        │              │                   │              │
│        └──────┬───────┴───────┬───────────┘              │
│          ┌────▼────┐    ┌────▼──────┐                    │
│          │  QUIC   │    │  Route    │                    │
│          │ (quinn) │    │ Resolver  │                    │
│          └─────────┘    └───────────┘                    │
└──────────────────────────────────────────────────────────┘
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
| シリアライズ | Protocol Buffers | 両レイヤ |
| Ars統合 | ProjectModule / EventBus | Ars Plugin |
| モニタリング | NetworkMonitor + EventBus | 両レイヤ |
| ランタイム | Tokio async | 両レイヤ |

## Ars との統合

`ars-plugin-synergos` は `ars-core` の `ProjectModule` trait を実装し、EventBus を通じて他モジュールと通信します。
`synergos-net` は Ars への依存を持たず、純粋なネットワークライブラリです。

```rust
// Ars Plugin Layer — ProjectModule 実装
pub struct SynergosPlugin {
    // Network Foundation Layer のインスタンスを保持
    net: synergos_net::SynergosNet,
}

impl ProjectModule for SynergosPlugin {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            id: "plugin-synergos",
            name: "Synergos",
            scope: ModuleScope::Project,
            depends_on: &[],  // オプトアウト原則
            emits: || vec![/* PeerConnected, FileTransferProgress, ... */],
            subscribes: || vec![/* ResourceImported (optional) */],
        }
    }
}
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
│   ├── proto/
│   │   └── synergos.proto     # Protocol Buffers 定義
│   └── src/
│       ├── lib.rs             # ネットワーク基盤エントリポイント
│       ├── conduit/           # 接続管理（Route Discovery + 接続確立）
│       ├── tunnel/            # Cloudflare Tunnel (cloudflared) 制御
│       ├── mesh/              # IPv6 Direct / TURN / STUN
│       └── quic/              # QUIC セッション管理
│
├── ars-plugin-synergos/       # Ars Plugin Layer
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs             # ProjectModule 実装
│       ├── events.rs          # ArsEvent 定義
│       ├── exchange/          # ファイル転送制御
│       └── presence/          # ピア状態管理
│
└── tests/
    ├── net_integration/       # ネットワーク基盤の統合テスト
    └── plugin_integration/    # プラグイン統合テスト
```

## ライセンス

MIT License
