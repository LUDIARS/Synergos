# Synergos 設計書

## 1. 概要

Synergos は **4つのレイヤ** で構成されるクロスプラットフォーム対応のリアルタイムコラボレーション基盤です。

- **Network Foundation Layer (`synergos-net`)** — Ars に依存しない汎用ネットワークライブラリ
- **Core Daemon Layer (`synergos-core`)** — Ars非依存の常駐型アプリケーション。EventBus・IPC・CLI制御を提供
- **GUI Application (`synergos-gui`)** — 専用のスタンドアロンGUIアプリケーション（git に対する SourceTree の関係）
- **Ars Plugin Layer (`ars-plugin-synergos`)** — Ars の ProjectModule として統合する薄いIPCアダプタ

Cloudflare Tunnel を介して外部ユーザーと QUIC コネクションを確立し、ファイルリソースを高速にやり取りします。

**対応プラットフォーム:** Windows / Linux / macOS

### 1.1 名前の由来

Synergos（シュネルゴス）— ギリシャ語で「共に働く者」を意味し、分散した作業者間のリソース共有を担います。

### 1.2 設計原則

| 原則 | 説明 |
|------|------|
| **4レイヤ分離** | ネットワーク基盤 → コアデーモン → GUI/プラグインの明確な責務分離。git ↔ SourceTree の関係に近い |
| **Ars非依存コア** | `synergos-core` はArsに一切依存せず、独立した常駐アプリケーションとして動作する |
| **クロスプラットフォーム** | Windows / Linux / macOS の全環境で同一機能を提供 |
| **IPC駆動** | コアデーモンとクライアント（GUI/CLI/Arsプラグイン）間はプロセス間通信で疎結合 |
| **オプトアウト** | Synergos が存在しなくても Ars は完全に動作する。EventBus の Optional 購読で疎結合を維持 |
| **Clio 同等基盤** | Clio のネットワーク抽象化と同等の設計パターンを採用。Depot/Resolver パターンで接続先を管理 |
| **ゼロコンフィグ接続** | Cloudflare Tunnel により NAT/FW 内からでも設定なしで外部公開可能 |
| **モバイル対応** | IPv6 + TURN/STUN でモバイルネットワークから NAT を介さず直接接続 |
| **転送効率** | QUIC の多重化・0-RTT で大容量ファイルを低レイテンシ転送 |

### 1.3 ars-collab との関係

既存の `ars-collab` モジュールは WebSocket ベースのプレゼンス・カーソル共有・ロック管理を担い、**同一ネットワーク内**のリアルタイム編集協調に特化しています。

Synergos は以下の点で補完的です:

| 観点 | ars-collab | Synergos |
|------|-----------|----------|
| 対象 | 同一サーバーに接続するユーザー | 外部ネットワークのユーザー |
| プロトコル | WebSocket (TCP) | QUIC (UDP) |
| 主目的 | プレゼンス・ロック・カーソル同期 | ファイルリソースの高速転送 |
| 接続形態 | クライアント→サーバー | P2P的（Tunnel/Direct経由） |
| ネットワーク要件 | 同一サーバーへのHTTPアクセス | NAT越え・外部ネットワーク対応 |

Synergos が利用可能な場合、`ars-collab` のファイル同期を Synergos の高速パスにオフロードできます。

## 2. レイヤアーキテクチャ

### 2.1 全体構成

```
┌──────────────────────────┐   ┌──────────────────────────────┐
│     synergos-gui          │   │    ars-plugin-synergos        │
│     (専用GUIアプリ)        │   │    (Ars薄IPCアダプタ)         │
│                           │   │                               │
│  ┌─────────┐ ┌─────────┐ │   │  ┌────────────────────────┐  │
│  │ Monitor │ │ Transfer│ │   │  │ ProjectModule impl    │  │
│  │ Panel   │ │ Manager │ │   │  │ EventBus ↔ IPC Bridge │  │
│  └────┬────┘ └────┬────┘ │   │  └───────────┬────────────┘  │
│       └─────┬─────┘      │   │              │               │
│             │ IPC        │   │              │ IPC           │
└─────────────┼────────────┘   └──────────────┼───────────────┘
              │                               │
              └───────────────┬───────────────┘
                              │
              ┌───────────────▼──────────────────────────────┐
              │         synergos-core (常駐デーモン)           │
              │                                               │
              │  ┌───────────┐  ┌────────────┐  ┌─────────┐ │
              │  │ EventBus  │  │ IPC Server │  │  CLI    │ │
              │  │ (内部)    │  │ (Unix/Pipe)│  │ Handler │ │
              │  └─────┬─────┘  └──────┬─────┘  └────┬────┘ │
              │        │               │             │      │
              │  ┌─────▼───────────────▼─────────────▼────┐ │
              │  │        Service Layer                    │ │
              │  │  Exchange │ Presence │ Conflict │ Catalog│ │
              │  └───────────────────┬─────────────────────┘ │
              │                      │ uses                   │
              ├──────────────────────┼───────────────────────┤
              │                      ▼                       │
              │    Network Foundation Layer (synergos-net)    │
              │                                               │
              │  ┌──────────┐ ┌──────────┐ ┌──────────────┐ │
              │  │ Conduit  │ │  Tunnel  │ │    Mesh      │ │
              │  │ (接続管理)│ │ Manager  │ │(IPv6/TURN)   │ │
              │  └────┬─────┘ └────┬─────┘ └──────┬───────┘ │
              │       └─────┬──────┴───────┬──────┘         │
              │        ┌────▼────┐   ┌────▼──────┐          │
              │        │  QUIC   │   │  Route    │          │
              │        │ (quinn) │   │ Resolver  │          │
              │        └─────────┘   └───────────┘          │
              └──────────────────────────────────────────────┘

              ┌──────────────────────────────────────────────┐
              │     synergos-ipc (共有IPCプロトコル定義)       │
              │  Command / Response / Event 型を全クレートで共有│
              └──────────────────────────────────────────────┘
```

### 2.2 git ↔ SourceTree モデル

Synergos の設計は **git と SourceTree の関係** をモデルとしています。

```
  git の世界                       Synergos の世界
  ─────────────                    ──────────────────
  git (CLIツール)           ←→     synergos-core (常駐デーモン)
  SourceTree (GUIアプリ)    ←→     synergos-gui (専用GUIアプリ)
  IDE内蔵git機能            ←→     ars-plugin-synergos (Arsプラグイン)
  git protocol              ←→     synergos-ipc (IPCプロトコル)
```

| 特性 | git | synergos-core |
|------|-----|---------------|
| 実行形態 | コマンド実行型 | 常駐デーモン型 |
| 通信方式 | stdin/stdout/stderr | IPC (Unix Socket / Named Pipe) |
| GUI | なし（外部ツールが担当） | なし（synergos-gui が担当） |
| IDE統合 | IDE内蔵git / Git Extension | ars-plugin-synergos |
| Ars依存 | — | なし |

### 2.3 レイヤの責務

| レイヤ | クレート | Ars依存 | 実行形態 | 責務 |
|--------|---------|---------|---------|------|
| **Network Foundation** | `synergos-net` | なし | ライブラリ | トランスポート抽象化、接続管理、QUIC/IPv6/Tunnel制御 |
| **IPC Protocol** | `synergos-ipc` | なし | ライブラリ | IPC コマンド・レスポンス・イベント型定義。全クライアントが共有 |
| **Core Daemon** | `synergos-core` | なし | **常駐バイナリ** | EventBus、IPCサーバー、CLI制御、ファイル転送・プレゼンス・コンフリクト管理 |
| **GUI Application** | `synergos-gui` | なし | **GUIバイナリ** | ネットワークモニター、転送管理、ピア管理、設定UI |
| **Ars Plugin** | `ars-plugin-synergos` | `ars-core` | ライブラリ | IPC経由でcoreに接続する薄いアダプタ。Ars EventBus ↔ IPC ブリッジ |

### 2.4 レイヤ間インターフェース

#### synergos-net → synergos-core（trait コールバック）

```rust
// synergos-net が定義するコールバック trait
// synergos-core がこれを実装して注入する
pub trait NetEventHandler: Send + Sync + 'static {
    /// ピアとの接続が確立した
    fn on_peer_connected(&self, peer_id: &PeerId, route: &RouteKind, rtt_ms: u32);
    /// ピアとの接続が切断した
    fn on_peer_disconnected(&self, peer_id: &PeerId, reason: &str);
    /// データ受信
    fn on_data_received(&self, peer_id: &PeerId, stream_id: u64, data: &[u8]);
    /// 接続経路が変更された
    fn on_route_changed(&self, peer_id: &PeerId, old: &RouteKind, new: &RouteKind);
}
```

#### synergos-core ↔ クライアント（IPC プロトコル）

```rust
// synergos-ipc で定義
// GUI / CLI / Ars Plugin が共通で利用するコマンド体系

/// クライアント → Core へのコマンド
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcCommand {
    // --- デーモン制御 ---
    Ping,
    Shutdown,
    Status,

    // --- プロジェクト管理 ---
    ProjectOpen { project_id: String, root_path: PathBuf },
    ProjectClose { project_id: String },
    ProjectList,

    // --- ピア管理 ---
    PeerList { project_id: String },
    PeerConnect { project_id: String, peer_id: String },
    PeerDisconnect { peer_id: String },

    // --- ファイル転送 ---
    TransferRequest { project_id: String, file_id: String, peer_id: String },
    TransferList { project_id: Option<String> },
    TransferCancel { transfer_id: String },
    PublishUpdate { project_id: String, file_paths: Vec<PathBuf> },

    // --- モニタリング ---
    NetworkStatus,
    Subscribe { events: Vec<EventFilter> },
    Unsubscribe { subscription_id: String },
}

/// Core → クライアントへのレスポンス
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    Ok,
    Error { code: u32, message: String },
    Status(DaemonStatus),
    ProjectList(Vec<ProjectInfo>),
    PeerList(Vec<PeerInfo>),
    TransferList(Vec<TransferInfo>),
    NetworkStatus(NetworkSnapshot),
    Subscribed { subscription_id: String },
}

/// Core → クライアントへのプッシュイベント（購読時）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcEvent {
    PeerConnected { peer_id: String, route: String, rtt_ms: u32 },
    PeerDisconnected { peer_id: String, reason: String },
    TransferProgress { transfer_id: String, bytes: u64, total: u64, speed_bps: u64 },
    TransferCompleted { transfer_id: String, file_path: String },
    ConflictDetected { file_id: String, involved_peers: Vec<String> },
    NetworkStatusUpdated(NetworkSnapshot),
}
```

#### IPC トランスポート（クロスプラットフォーム）

```
┌─────────────────────────────────────────────────────┐
│            Cross-Platform IPC Transport               │
│                                                       │
│  ┌─── Linux ────────┐  ┌─── macOS ────────┐          │
│  │ Unix Domain      │  │ Unix Domain      │          │
│  │ Socket           │  │ Socket           │          │
│  │                  │  │                  │          │
│  │ /tmp/synergos/   │  │ ~/Library/       │          │
│  │   synergos.sock  │  │ Application      │          │
│  │                  │  │ Support/Synergos │          │
│  └──────────────────┘  │  /synergos.sock  │          │
│                        └──────────────────┘          │
│  ┌─── Windows ──────┐                                │
│  │ Named Pipe       │                                │
│  │                  │                                │
│  │ \\.\pipe\        │                                │
│  │ synergos         │                                │
│  └──────────────────┘                                │
│                                                       │
│  プロトコル: 長さプレフィクス付きメッセージフレーム    │
│  シリアライズ: MessagePack (高速・コンパクト)         │
│  フロー: Request/Response + Server Push (Event)       │
└─────────────────────────────────────────────────────┘
```

### 2.5 ネットワーク間通信の全体像

```
                          Internet
                             │
              ┌──────────────┼──────────────┐
              │              │              │
     ┌────────▼───────┐     │     ┌────────▼───────┐
     │   Peer A        │     │     │   Peer B        │
     │                 │     │     │                 │
     │  ┌───────────┐ │     │     │  ┌───────────┐ │
     │  │synergos   │ │     │     │  │synergos   │ │
     │  │-core      │ │     │     │  │-core      │ │
     │  │(デーモン)  │ │     │     │  │(デーモン)  │ │
     │  ├───────────┤ │     │     │  ├───────────┤ │
     │  │synergos   │ │     │     │  │synergos   │ │
     │  │-net       │─┼─────┼─────┼──│-net       │ │
     │  └───────────┘ │     │     │  └───────────┘ │
     │       ▲ IPC    │     │     │       ▲ IPC    │
     │  ┌────┴──────┐ │     │     │  ┌────┴──────┐ │
     │  │GUI / Ars  │ │     │     │  │GUI / Ars  │ │
     │  │Plugin     │ │     │     │  │Plugin     │ │
     │  └───────────┘ │     │     │  └───────────┘ │
     └────────────────┘     │     └────────────────┘
                            │
              ┌─────────────┼─────────────┐
              │                           │
     ┌────────▼────────┐       ┌─────────▼────────┐
     │ Cloudflare      │       │ TURN/STUN        │
     │ Tunnel (QUIC)   │       │ Server (IPv6)    │
     └─────────────────┘       └──────────────────┘

     ┌────────────────────────────────────────────┐
     │          Mobile Client (non-Ars)           │
     │                                            │
     │  synergos-core + synergos-gui で接続可能   │
     │  Ars 不要                                  │
     └────────────────────────────────────────────┘
```

## 3. Network Foundation Layer (`synergos-net`)

### 3.1 概要

Ars への依存を一切持たない、汎用のピアツーピアネットワークライブラリです。
モバイルクライアントや他のアプリケーションからも直接利用可能です。

### 3.2 Conduit（接続管理）

外部ユーザーとのコネクションのライフサイクルを管理するコアコンポーネントです。

- **役割**: 接続の確立・維持・切断、接続方式の選択と切り替え
- **配置場所**: `synergos-net/src/conduit/`

**接続戦略（優先度順フォールバック）:**

```
接続優先度:
  1. IPv6 Direct (最速・NAT不要)
  2. QUIC via Cloudflare Tunnel (確実・セキュア)
  3. WebSocket Relay (フォールバック・既存collab基盤利用)
```

```rust
/// 接続先の抽象化（Clio の Depot パターンに準拠）
pub struct PeerEndpoint {
    /// ピア識別子
    pub peer_id: PeerId,
    /// 表示名
    pub display_name: String,
    /// 利用可能な接続経路（優先度順）
    pub routes: Vec<Route>,
    /// 接続状態
    pub state: ConnectionState,
}

pub enum Route {
    /// IPv6 ダイレクト接続
    Direct {
        addr: SocketAddrV6,
        fqdn: Option<String>,
    },
    /// Cloudflare Tunnel 経由
    Tunnel {
        tunnel_id: String,
        hostname: String,
    },
    /// WebSocket リレー（フォールバック）
    Relay {
        server_url: String,
        room_id: String,
    },
}

pub enum ConnectionState {
    Discovered,
    Connecting,
    Connected { rtt_ms: u32, route: RouteKind },
    Disconnected { reason: String },
}
```

**Conduit の責務:**

| 機能 | 説明 |
|------|------|
| Route Discovery | 各ピアの利用可能経路を検出。IPv6到達性のプローブ → Tunnel可用性確認 → Relay利用可能性 |
| Connection Establishment | 最高優先度の経路から順に接続試行。失敗時は次の経路にフォールバック |
| Connection Monitoring | RTT計測、帯域推定、経路品質の継続的モニタリング |
| Route Migration | 接続中に高優先度の経路が利用可能になった場合、透過的に切り替え |
| Keepalive | QUIC の idle timeout 前にピンポンで接続維持 |

### 3.3 Tunnel Manager（Cloudflare Tunnel 管理）

cloudflared プロセスの制御と Tunnel セッションの管理を行います。

- **役割**: cloudflared の起動・停止、Tunnel 認証、ヘルスチェック
- **配置場所**: `synergos-net/src/tunnel/`

```rust
pub struct TunnelManager {
    /// cloudflared プロセスハンドル
    process: Option<Child>,
    /// Tunnel の公開ホスト名
    pub hostname: String,
    /// Tunnel の状態
    pub state: TunnelState,
    /// QUIC トランスポート設定
    pub quic_config: QuicConfig,
}

pub enum TunnelState {
    Idle,
    Starting,
    Active { tunnel_id: String, uptime_secs: u64 },
    Error { message: String, retry_at: Instant },
}

pub struct QuicConfig {
    pub max_concurrent_streams: u32,
    pub idle_timeout_ms: u64,
    pub max_udp_payload_size: u16,
    pub enable_0rtt: bool,
}
```

### 3.4 Mesh（IPv6 Direct / TURN 接続）

NAT を介さない直接接続を確立するためのコンポーネントです。モバイルネットワーク上での利用を主目的とします。

- **役割**: IPv6 アドレス解決、TURN/STUN セッション管理、FQDN による名前解決
- **配置場所**: `synergos-net/src/mesh/`

```
┌─────────────────────────────────────────────────┐
│                    Mesh                          │
│                                                  │
│  ┌───────────────┐  ┌───────────────────────┐   │
│  │ FQDN Resolver │  │ TURN/STUN Manager    │   │
│  │               │  │                       │   │
│  │ DNS-over-HTTPS│  │ Allocation 管理       │   │
│  │ AAAA 解決     │  │ Permission 管理       │   │
│  │ 自前DNS管理   │  │ Channel Binding       │   │
│  └──────┬────────┘  └──────────┬────────────┘   │
│         │                      │                │
│  ┌──────▼──────────────────────▼────────────┐   │
│  │        IPv6 Transport Layer              │   │
│  │                                          │   │
│  │  直接接続 or TURN リレー                  │   │
│  │  QUIC over IPv6                          │   │
│  └──────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘
```

**IPv6 ダイレクト接続フロー:**

```
1. ピアの FQDN を DNS-over-HTTPS で AAAA レコード解決
2. IPv6 アドレスに対して QUIC 接続を試行
3. 到達可能 → 直接接続確立（最速パス）
4. 到達不可 → TURN サーバー経由のリレーにフォールバック
```

```rust
/// ネットワーク基盤設定（synergos-net の設定。Ars非依存）
pub struct NetConfig {
    pub tunnel: TunnelConfig,
    pub mesh: MeshConfig,
    pub quic: QuicConfig,
}

pub struct MeshConfig {
    /// 自前の DNS サーバー（AAAA レコード管理）
    pub dns_servers: Vec<String>,
    /// DNS-over-HTTPS エンドポイント
    pub doh_endpoint: String,
    /// TURN サーバー一覧
    pub turn_servers: Vec<TurnServer>,
    /// STUN サーバー一覧
    pub stun_servers: Vec<String>,
    /// IPv6 到達性プローブのタイムアウト
    pub probe_timeout_ms: u32,
}

pub struct TurnServer {
    pub uri: String,
    pub username: String,
    pub credential: String,
    pub auth_method: TurnAuthMethod,
}

pub enum TurnAuthMethod {
    /// 長期認証（ユーザー名＋パスワード）
    LongTerm,
    /// 短期認証（REST API 経由で一時クレデンシャル取得）
    EphemeralRest { token_endpoint: String },
}
```

**TURN の利用ポリシー:**

| シナリオ | 接続方式 |
|---------|---------|
| 双方 IPv6 到達可能 | Direct IPv6 (TURN 不使用) |
| 片方のみ IPv6 | TURN リレー経由 |
| 双方 IPv4 のみ | Cloudflare Tunnel にフォールバック |
| モバイル回線 (IPv6) | Direct IPv6 を優先試行 → TURN フォールバック |

### 3.5 QUIC セッション管理

- **配置場所**: `synergos-net/src/quic/`

```rust
/// QUIC コネクションのラッパー
pub struct QuicStream {
    /// 双方向ストリーム
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    /// ストリーム種別
    pub stream_type: StreamType,
}

pub enum StreamType {
    /// 制御メッセージ（Handshake, Ping/Pong, Bye）
    Control,
    /// ファイルデータ転送
    Data { transfer_id: TransferId },
}
```

### 3.6 Protocol Buffers 定義

synergos-net のワイヤプロトコルです。

```protobuf
syntax = "proto3";
package synergos;

// ピア間メッセージの共通エンベロープ
message Envelope {
    string message_id = 1;
    string sender_id = 2;
    uint64 timestamp_ms = 3;
    oneof payload {
        Handshake handshake = 10;
        TransferOffer transfer_offer = 11;
        TransferAccept transfer_accept = 12;
        TransferReject transfer_reject = 13;
        Chunk chunk = 14;
        ChunkAck chunk_ack = 15;
        Ping ping = 20;
        Pong pong = 21;
        Bye bye = 30;
    }
}

message Handshake {
    string peer_id = 1;
    string display_name = 2;
    string project_id = 3;
    string project_token = 4;
    uint32 protocol_version = 5;
    repeated string capabilities = 6;
}

message TransferOffer {
    string transfer_id = 1;
    string resource_id = 2;
    string file_name = 3;
    uint64 file_size = 4;
    bytes checksum = 5;  // Blake3
    uint32 chunk_size = 6;
    uint32 total_chunks = 7;
    TransferPriority priority = 8;
}

enum TransferPriority {
    INTERACTIVE = 0;
    BACKGROUND = 1;
    PREFETCH = 2;
}

message TransferAccept {
    string transfer_id = 1;
    bytes owned_chunks_bitmap = 2;
}

message TransferReject {
    string transfer_id = 1;
    string reason = 2;
}

message Chunk {
    string transfer_id = 1;
    uint32 chunk_index = 2;
    bytes data = 3;
    bytes checksum = 4;
}

message ChunkAck {
    string transfer_id = 1;
    uint32 chunk_index = 2;
}

message Ping {
    uint64 sent_at_ms = 1;
}

message Pong {
    uint64 ping_sent_at_ms = 1;
    uint64 pong_sent_at_ms = 2;
}

message Bye {
    string reason = 1;
}
```

## 4. Core Daemon Layer (`synergos-core`)

### 4.1 概要

`synergos-core` は Ars に一切依存しない**常駐型デーモンアプリケーション**です。
`synergos-net` の上に構築され、ファイル転送・プレゼンス・コンフリクト管理などの
ビジネスロジックをすべて担います。外部との通信は IPC サーバー経由で行います。

**実行形態:** バックグラウンドデーモン（OS サービス / ユーザープロセス）

### 4.2 デーモンライフサイクル

```
┌───────────────────────────────────────────────────────┐
│               Daemon Lifecycle                          │
│                                                         │
│  synergos-core start                                    │
│       │                                                 │
│       ▼                                                 │
│  ┌──────────┐   ┌──────────┐   ┌──────────────────┐   │
│  │ Config   │──▶│ EventBus │──▶│ IPC Server       │   │
│  │ Load     │   │ Init     │   │ Listen           │   │
│  └──────────┘   └──────────┘   └────────┬─────────┘   │
│                                          │             │
│       ┌──────────────────────────────────┘             │
│       ▼                                                │
│  ┌──────────────────────────────────────────────┐      │
│  │         Ready (常駐状態)                      │      │
│  │                                               │      │
│  │  ・IPC コマンド受付                           │      │
│  │  ・プロジェクト管理                           │      │
│  │  ・ネットワーク接続管理                       │      │
│  │  ・ファイル転送制御                           │      │
│  │  ・イベント配信                               │      │
│  └──────────────────────────────────────────────┘      │
│       │                                                │
│       ▼  (Shutdown コマンド or シグナル)                │
│  ┌──────────────────────────────────────────────┐      │
│  │  Graceful Shutdown                            │      │
│  │  1. 新規コマンド受付停止                      │      │
│  │  2. アクティブ転送の完了待ち                   │      │
│  │  3. ピアへの Bye 送信                         │      │
│  │  4. IPC ソケットクリーンアップ                │      │
│  └──────────────────────────────────────────────┘      │
└───────────────────────────────────────────────────────┘
```

### 4.3 内部 EventBus

```rust
// synergos-core 内部の EventBus（Ars EventBus とは独立）

/// 型安全な内部イベントバス
pub struct CoreEventBus {
    /// イベント種別ごとのブロードキャストチャンネル
    channels: DashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl CoreEventBus {
    /// イベントを発行
    pub async fn emit<E: CoreEvent>(&self, event: E);
    /// イベントを購読
    pub fn subscribe<E: CoreEvent>(&self) -> broadcast::Receiver<E>;
}

/// コアイベント trait
pub trait CoreEvent: Clone + Send + Sync + 'static {
    fn event_name() -> &'static str;
}
```

### 4.4 IPC サーバー

```rust
/// クロスプラットフォーム IPC サーバー
pub struct IpcServer {
    /// イベントバス参照
    event_bus: Arc<CoreEventBus>,
    /// 接続中のクライアント
    clients: DashMap<ClientId, ClientConnection>,
}

impl IpcServer {
    /// プラットフォームに応じた IPC リスナーを起動
    pub async fn start(event_bus: Arc<CoreEventBus>) -> Result<Self> {
        // Linux/macOS: Unix Domain Socket
        // Windows: Named Pipe
    }

    /// クライアントからのコマンドをディスパッチ
    async fn dispatch(&self, cmd: IpcCommand) -> IpcResponse { /* ... */ }

    /// 購読中のクライアントにイベントをプッシュ
    async fn push_event(&self, client_id: &ClientId, event: IpcEvent) { /* ... */ }
}
```

### 4.5 CLI コマンドハンドラ

```
synergos-core start              # デーモン起動
synergos-core stop               # デーモン停止
synergos-core status             # 状態確認

synergos-core project open <id> <path>   # プロジェクト登録
synergos-core project list               # プロジェクト一覧
synergos-core project close <id>         # プロジェクト解除

synergos-core peer list <project>        # ピア一覧
synergos-core peer connect <peer>        # ピア接続

synergos-core transfer list              # 転送一覧
synergos-core transfer cancel <id>       # 転送取消
synergos-core publish <project> <files>  # 更新公開

synergos-core network status             # ネットワーク状態
```

### 4.6 サービスレイヤ

Exchange・Presence・Conflict は従来 `ars-plugin-synergos` にあったビジネスロジックを
`synergos-core` に移動したもの。Ars への依存を除去し、独立して動作する。

## 5. Ars Plugin Layer (`ars-plugin-synergos`)

### 5.1 概要

`synergos-core` デーモンへの**薄い IPC アダプタ**として機能する Ars プラグインです。
従来はネットワーク基盤を直接利用していましたが、新アーキテクチャでは IPC 経由で
`synergos-core` と通信し、Ars EventBus とのブリッジのみを担います。

### 5.2 プラグインインターフェース

```rust
use ars_core::module::{ModuleInfo, ModuleScope, ProjectContext, ProjectModule};
use ars_core::event_bus::EventBus;
use synergos_ipc::{IpcClient, IpcCommand, IpcResponse, IpcEvent};

pub struct SynergosPlugin {
    /// synergos-core への IPC クライアント
    ipc: Option<IpcClient>,
    /// Ars EventBus ハンドル
    event_bus: Option<EventBus>,
}

#[async_trait]
impl ProjectModule for SynergosPlugin {
    fn info(&self) -> ModuleInfo {
        ModuleInfo {
            id: "plugin-synergos",
            name: "Synergos",
            scope: ModuleScope::Project,
            depends_on: &[],  // 依存なし — オプトアウト原則
            emits: || vec![
                TypeId::of::<PeerConnected>(),
                TypeId::of::<PeerDisconnected>(),
                TypeId::of::<FileTransferProgress>(),
                TypeId::of::<FileTransferCompleted>(),
            ],
            subscribes: || vec![
                TypeId::of::<ResourceImported>(),
            ],
        }
    }

    async fn on_project_open(
        &mut self,
        ctx: &ProjectContext,
        event_bus: &EventBus,
    ) -> Result<()> {
        self.event_bus = Some(event_bus.clone());

        // 1. synergos-core デーモンに IPC 接続
        let ipc = IpcClient::connect().await?;

        // 2. プロジェクトを登録
        ipc.send(IpcCommand::ProjectOpen {
            project_id: ctx.project_id.clone(),
            root_path: ctx.root_path.clone(),
        }).await?;

        // 3. イベント購読を開始し、Ars EventBus にブリッジ
        let subscription = ipc.send(IpcCommand::Subscribe {
            events: vec![EventFilter::All],
        }).await?;

        let bus = event_bus.clone();
        let ipc_clone = ipc.clone();
        tokio::spawn(async move {
            Self::bridge_events(ipc_clone, bus).await;
        });

        self.ipc = Some(ipc);
        Ok(())
    }

    async fn on_project_close(&mut self) -> Result<()> {
        if let Some(ipc) = self.ipc.take() {
            ipc.send(IpcCommand::ProjectClose {
                project_id: self.project_id.clone(),
            }).await.ok();
        }
        Ok(())
    }
}

impl SynergosPlugin {
    /// IPC イベントを Ars EventBus イベントに変換して配信
    async fn bridge_events(ipc: IpcClient, bus: EventBus) {
        while let Ok(event) = ipc.recv_event().await {
            match event {
                IpcEvent::PeerConnected { peer_id, route, rtt_ms } => {
                    bus.emit(PeerConnected { peer_id, route, rtt_ms, .. }).await;
                }
                IpcEvent::TransferProgress { .. } => {
                    bus.emit(FileTransferProgress { .. }).await;
                }
                // ... 他イベントも同様にブリッジ
            }
        }
    }
}
```

## 6. GUI Application (`synergos-gui`)

### 6.1 概要

`synergos-gui` は `synergos-core` デーモンに IPC 接続する**スタンドアロンの GUI アプリケーション**です。
Ars には一切依存せず、独立したアプリとして起動・動作します。

git と SourceTree の関係と同様に、GUI がなくても core は完全に機能し、
GUI は core のフロントエンドとしてのみ動作します。

### 6.2 技術選定

| 技術 | 理由 |
|------|------|
| **egui + eframe** | Rust ネイティブ。Win/Linux/Mac 対応。軽量・高速。即時モードGUI |
| **MessagePack over IPC** | synergos-core との通信プロトコル |

### 6.3 画面構成

```
┌────────────────────────────────────────────────────────────┐
│  Synergos                                     ─ □ ×       │
├────────────────────────────────────────────────────────────┤
│  ┌─ Projects ──┐  ┌─ Network Overview ────────────────┐   │
│  │ ▶ Project A │  │ Status: Connected                  │   │
│  │   Project B │  │ Route: IPv6 Direct                 │   │
│  │   Project C │  │ Peers: 3/4 | Latency: 12ms        │   │
│  └─────────────┘  │ Bandwidth: ▓▓▓▓▓░░ 245 Mbps       │   │
│                    └───────────────────────────────────┘   │
│  ┌─ Peers ─────────────────────────────────────────────┐  │
│  │ ● Alice    │ IPv6   │ 8ms  │ 120 Mbps │ Active     │  │
│  │ ● Bob      │ Tunnel │ 45ms │ 80 Mbps  │ Idle       │  │
│  │ ● Charlie  │ IPv6   │ 12ms │ 245 Mbps │ Active     │  │
│  └─────────────────────────────────────────────────────┘  │
│  ┌─ Transfers ─────────────────────────────────────────┐  │
│  │ ↑ scene.bin      │ 250MB │ ████░░ 72% │ 85 MB/s   │  │
│  │ ↓ textures.pak   │ 45MB  │ ██░░░░ 35% │ 42 MB/s   │  │
│  │ ✓ config.json    │ 12KB  │ Done       │           │  │
│  └─────────────────────────────────────────────────────┘  │
│  ┌─ Conflicts ─────────────────────────────────────────┐  │
│  │ ⚠ main.rs  │ Alice vs Bob │ v4 fork │ [Resolve]   │  │
│  └─────────────────────────────────────────────────────┘  │
├────────────────────────────────────────────────────────────┤
│  Core: Running (PID 12345) │ Uptime: 2h 15m              │
└────────────────────────────────────────────────────────────┘
```

## 7. Exchange（転送制御）（synergos-core 内部）

ファイルリソースの転送を制御する Ars Plugin Layer のコンポーネントです。
Network Foundation Layer の QUIC ストリームを使ってファイルを送受信します。

- **配置場所**: `ars-plugin-synergos/src/exchange/`

```rust
pub struct TransferRequest {
    pub transfer_id: TransferId,
    pub resource_id: String,
    pub file_path: PathBuf,
    pub file_size: u64,
    pub checksum: Blake3Hash,
    pub priority: TransferPriority,
}

pub enum TransferPriority {
    /// ユーザーが明示的に要求した転送
    Interactive,
    /// バックグラウンド同期
    Background,
    /// プリフェッチ（帯域に余裕がある場合のみ）
    Prefetch,
}
```

**転送最適化:**

| 機能 | 説明 |
|------|------|
| チャンク分割 | 大ファイルを固定サイズチャンク（デフォルト 256 KiB）に分割し、QUIC ストリームで並列転送 |
| 差分転送 | Blake3 のローリングハッシュで変更チャンクのみ転送（rsync類似） |
| 優先度キュー | Interactive > Background > Prefetch の順で帯域を配分 |
| 輻輳制御 | QUIC の輻輳制御（BBR）に加え、アプリ層で帯域上限を設定可能 |
| 再開可能 | 接続断からの再接続時、転送済みチャンクからレジューム |

### 4.5 Presence Service（ピア状態管理）

接続可能なピアの発見と状態管理を行う Ars Plugin Layer のコンポーネントです。

- **配置場所**: `ars-plugin-synergos/src/presence/`

```rust
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub endpoints: Vec<Route>,
    pub last_seen: Instant,
    pub project_id: String,
}

/// Presence の通知方式（差し替え可能）
pub enum PresenceBackend {
    /// Cloudflare Workers KV を利用した登録・発見
    CloudflareKV { account_id: String, namespace_id: String },
    /// 共有 WebSocket サーバー経由（ars-collab を流用）
    CollabRelay { server_url: String },
    /// mDNS (同一LAN内のゼロコンフィグ発見)
    Mdns,
}
```

### 4.6 イベント定義

```rust
use ars_core::event::ArsEvent;

#[derive(Debug, Clone)]
pub struct PeerConnected {
    pub peer_id: PeerId,
    pub display_name: String,
    pub route: RouteKind,
    pub rtt_ms: u32,
}

impl ArsEvent for PeerConnected {
    fn source_module(&self) -> &'static str { "plugin-synergos" }
    fn category(&self) -> &'static str { "collab" }
}

#[derive(Debug, Clone)]
pub struct PeerDisconnected {
    pub peer_id: PeerId,
    pub reason: DisconnectReason,
}

impl ArsEvent for PeerDisconnected {
    fn source_module(&self) -> &'static str { "plugin-synergos" }
    fn category(&self) -> &'static str { "collab" }
}

#[derive(Debug, Clone)]
pub struct FileTransferProgress {
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub resource_id: String,
    pub bytes_transferred: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
}

impl ArsEvent for FileTransferProgress {
    fn source_module(&self) -> &'static str { "plugin-synergos" }
    fn category(&self) -> &'static str { "transfer" }
}

#[derive(Debug, Clone)]
pub struct FileTransferCompleted {
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub resource_id: String,
    pub file_path: PathBuf,
    pub checksum: Blake3Hash,
}

impl ArsEvent for FileTransferCompleted {
    fn source_module(&self) -> &'static str { "plugin-synergos" }
    fn category(&self) -> &'static str { "transfer" }
}
```

### 4.7 他モジュールからの Optional 購読例

Synergos がインストールされていない場合、`subscribe` は `None` を返し、コードパスがスキップされます。

```rust
// ars-collab 側（Synergos がある場合のみ高速パスを利用）
async fn on_project_open(&mut self, ctx: &ProjectContext, bus: &EventBus) -> Result<()> {
    if let Some(mut rx) = bus.subscribe::<FileTransferCompleted>().await {
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                // Synergos 経由で転送完了 → ロック解除・通知
            }
        });
    }
    // Synergos がなくても通常の WebSocket 同期で動作
    Ok(())
}
```

## 8. Clio との設計的対応

Synergos は Clio のネットワーク基盤設計パターンを踏襲します。

| Clio コンポーネント | Synergos 対応 | レイヤ | 役割 |
|-------------------|--------------|--------|------|
| Depot（共用デポ） | **Peer Registry** | Plugin | 接続可能なピアとそのエンドポイントを管理 |
| Acquirer（取得レンジ） | **Conduit** | Net Foundation | 接続経路の探索とフィルタリング |
| Metadata Store | **Presence Service** | Plugin | ピアのメタデータ（状態・能力・帯域） |
| Resolver（リソース解決） | **Route Resolver** | Net Foundation | 最適な接続経路の選定（レイテンシ・帯域・安定性でスコアリング） |
| Preference Adapter | **Transfer Policy** | Plugin | ユーザーの転送設定（帯域制限・優先度）を反映 |

## 9. セキュリティ

### 6.1 認証・認可

```
┌──────────────────────────────────────────────┐
│              Authentication Flow              │
│                                               │
│  1. Ars Auth で認証済みユーザーを確認          │  ← Plugin Layer
│  2. プロジェクトの共有設定を検証              │  ← Plugin Layer
│  3. Cloudflare Access で Tunnel 認証          │  ← Net Foundation
│  4. QUIC ハンドシェイク時に mTLS で相互認証    │  ← Net Foundation
│  5. 接続後はピアごとの Permission で制御       │  ← Plugin Layer
└──────────────────────────────────────────────┘
```

| レイヤ | 方式 | 目的 |
|--------|------|------|
| Tunnel | Cloudflare Access (JWT) | Tunnel への不正アクセス防止 |
| Transport | TLS 1.3 (mTLS) | 通信の暗号化・ピア認証 |
| Application | プロジェクト共有トークン | プロジェクトレベルの認可 |

### 6.2 ファイル転送のセキュリティ

- **チェックサム検証**: 全チャンクに Blake3 ハッシュ。改竄検知
- **パス検証**: プロジェクトルート外への書き込みを拒否（パストラバーサル防止）
- **サイズ制限**: 転送可能なファイルサイズの上限を設定で管理
- **レート制限**: ピアごとの転送レートを制限可能

## 10. 設定

### 7.1 プロジェクト設定

```toml
# <project_root>/.ars/synergos.toml

# --- Network Foundation Layer の設定 ---

[tunnel]
api_token_ref = "keychain:cloudflare-api-token"
hostname = ""
max_concurrent_streams = 100
idle_timeout_ms = 30000
enable_0rtt = true

[mesh]
doh_endpoint = "https://dns.example.com/dns-query"
dns_servers = ["2001:db8::1"]
probe_timeout_ms = 3000

[[mesh.turn_servers]]
uri = "turn:turn.example.com:3478?transport=udp"
username = ""
credential_ref = "keychain:turn-credential"
auth_method = "ephemeral_rest"
token_endpoint = "https://turn.example.com/api/credentials"

[mesh.stun_servers]
servers = ["stun:stun.example.com:3478"]

# --- Ars Plugin Layer の設定 ---

[transfer]
chunk_size = 262144  # 256 KiB
max_concurrent_transfers = 8
bandwidth_limit = 0
max_file_size = 0

[presence]
backend = "cloudflare_kv"
[presence.cloudflare_kv]
account_id = ""
namespace_id = ""
```

### 7.2 グローバル設定

```toml
# ~/.ars/synergos.toml

[global]
enabled = true
prefer_ipv6_on_mobile = true
allow_background_transfers = true
```

## 11. ディレクトリ構成

```
Synergos/
├── DESIGN.md                          # 本設計書
├── README.md                          # プロジェクト概要
├── LICENSE
├── Cargo.toml                         # Workspace root
│
├── synergos-net/                      # ═══ Network Foundation Layer ═══
│   ├── Cargo.toml                     #   Ars非依存の汎用ネットワークライブラリ
│   ├── proto/
│   │   └── synergos.proto             #   Protocol Buffers 定義
│   └── src/
│       ├── lib.rs                     #   公開API: SynergosNet, NetEventHandler
│       ├── config.rs                  #   NetConfig, MeshConfig, TunnelConfig
│       ├── types.rs                   #   PeerId, TransferId, Route 等
│       ├── error.rs                   #   エラー型定義
│       ├── conduit/                   #   接続管理
│       ├── tunnel/                    #   Cloudflare Tunnel 制御
│       ├── mesh/                      #   IPv6 Direct / TURN / STUN
│       ├── quic/                      #   QUIC セッション管理
│       ├── dht/                       #   Kademlia 分散ハッシュテーブル
│       ├── gossip/                    #   Gossipsub パブサブメッシュ
│       ├── catalog/                   #   ファイルカタログ管理
│       └── chain/                     #   更新チェーン（ブロック履歴）
│
├── synergos-ipc/                      # ═══ IPC Protocol Layer ═══
│   ├── Cargo.toml                     #   共有IPCプロトコル型定義
│   └── src/
│       ├── lib.rs                     #   公開API
│       ├── command.rs                 #   IpcCommand 定義
│       ├── response.rs                #   IpcResponse 定義
│       ├── event.rs                   #   IpcEvent 定義
│       ├── transport.rs               #   IPC トランスポート抽象化
│       └── client.rs                  #   IpcClient（クライアント側接続ユーティリティ）
│
├── synergos-core/                     # ═══ Core Daemon Layer ═══
│   ├── Cargo.toml                     #   常駐型デーモンバイナリ
│   └── src/
│       ├── main.rs                    #   エントリポイント（CLI解析・デーモン起動）
│       ├── daemon.rs                  #   デーモンライフサイクル管理
│       ├── event_bus.rs               #   内部 EventBus（Ars非依存）
│       ├── ipc_server.rs              #   IPC サーバー（Unix Socket / Named Pipe）
│       ├── cli.rs                     #   CLI コマンドハンドラ
│       ├── project.rs                 #   プロジェクト管理
│       ├── exchange/                  #   ファイル転送制御（旧 ars-plugin から移動）
│       │   └── mod.rs
│       ├── presence/                  #   ピア状態管理（旧 ars-plugin から移動）
│       │   └── mod.rs
│       └── conflict/                  #   コンフリクト管理（旧 ars-plugin から移動）
│           └── mod.rs
│
├── synergos-gui/                      # ═══ GUI Application ═══
│   ├── Cargo.toml                     #   スタンドアロンGUIバイナリ
│   └── src/
│       ├── main.rs                    #   エントリポイント
│       ├── app.rs                     #   アプリケーション状態・メインループ
│       ├── connection.rs              #   synergos-core への IPC 接続管理
│       └── ui/                        #   UI コンポーネント
│           ├── mod.rs
│           ├── overview.rs            #   ネットワーク概要パネル
│           ├── peers.rs               #   ピア一覧パネル
│           ├── transfers.rs           #   転送状況パネル
│           ├── conflicts.rs           #   コンフリクト管理パネル
│           └── settings.rs            #   設定パネル
│
├── ars-plugin-synergos/               # ═══ Ars Plugin Layer (薄IPCアダプタ) ═══
│   ├── Cargo.toml                     #   ars-core + synergos-ipc に依存
│   └── src/
│       ├── lib.rs                     #   ProjectModule 実装（IPC経由）
│       ├── events.rs                  #   ArsEvent 定義
│       └── bridge.rs                  #   IpcEvent → Ars EventBus ブリッジ
│
└── tests/
    ├── net_integration/               # Network Foundation 統合テスト
    ├── core_integration/              # Core Daemon 統合テスト
    └── ipc_integration/               # IPC 通信テスト
```

## 12. 依存クレート

### 12.1 Workspace Cargo.toml

```toml
[workspace]
members = [
    "synergos-net",
    "synergos-ipc",
    "synergos-core",
    "synergos-gui",
    "ars-plugin-synergos",
]
resolver = "2"
```

### 12.2 synergos-net/Cargo.toml（変更なし）

```toml
[package]
name = "synergos-net"
version = "0.1.0"
edition = "2021"
description = "P2P network foundation with QUIC, Cloudflare Tunnel, and IPv6 Direct/TURN"

[dependencies]
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
quinn = "0.11"
rustls = { version = "0.23", features = ["ring"] }
prost = "0.13"
prost-types = "0.13"
blake3 = "1"
serde = { version = "1", features = ["derive"] }
hickory-resolver = { version = "0.24", features = ["dns-over-https-rustls"] }
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }
bytes = "1"
dashmap = "6"
crc32fast = "1"
thiserror = "2"

[build-dependencies]
prost-build = "0.13"
```

### 12.3 synergos-ipc/Cargo.toml（新規）

```toml
[package]
name = "synergos-ipc"
version = "0.1.0"
edition = "2021"
description = "Shared IPC protocol types for Synergos cross-platform communication"

[dependencies]
serde = { version = "1", features = ["derive"] }
rmp-serde = "1"              # MessagePack シリアライズ
tokio = { version = "1", features = ["io-util", "net"] }
thiserror = "2"
tracing = "0.1"

# クロスプラットフォーム IPC
[target.'cfg(unix)'.dependencies]
# Unix Domain Socket は tokio::net に含まれる

[target.'cfg(windows)'.dependencies]
tokio = { version = "1", features = ["io-util", "net"] }
```

### 12.4 synergos-core/Cargo.toml（新規）

```toml
[package]
name = "synergos-core"
version = "0.1.0"
edition = "2021"
description = "Synergos core daemon — resident application with EventBus and IPC server"

[[bin]]
name = "synergos-core"
path = "src/main.rs"

[dependencies]
synergos-net = { path = "../synergos-net" }
synergos-ipc = { path = "../synergos-ipc" }
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
blake3 = "1"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
dashmap = "6"
uuid = { version = "1", features = ["v4"] }
mdns-sd = "0.11"
crc32fast = "1"
thiserror = "2"
```

### 12.5 synergos-gui/Cargo.toml（新規）

```toml
[package]
name = "synergos-gui"
version = "0.1.0"
edition = "2021"
description = "Synergos standalone GUI application — network monitor and transfer manager"

[[bin]]
name = "synergos-gui"
path = "src/main.rs"

[dependencies]
synergos-ipc = { path = "../synergos-ipc" }
eframe = "0.31"              # egui フレームワーク（Win/Linux/Mac対応）
egui = "0.31"                # 即時モードGUI
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
```

### 12.6 ars-plugin-synergos/Cargo.toml（変更: synergos-net → synergos-ipc）

```toml
[package]
name = "ars-plugin-synergos"
version = "0.1.0"
edition = "2021"
description = "Ars plugin — thin IPC adapter to synergos-core daemon"

[dependencies]
# IPC プロトコル（synergos-net への直接依存は不要に）
synergos-ipc = { path = "../synergos-ipc" }

# Ars コア
ars-core = { git = "https://github.com/LUDIARS/Ars", path = "crates/ars-core" }

tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
thiserror = "2"
```

## 13. 実装ロードマップ

### Phase 1: Network Foundation 骨格（完了）

| Step | 作業 | クレート |
|------|------|---------|
| 1-1 | `types.rs` + `config.rs` — 共通型と設定 | synergos-net |
| 1-2 | `tunnel/cloudflared.rs` — cloudflared プロセス制御 | synergos-net |
| 1-3 | `quic/` — QUIC セッション管理 (quinn) | synergos-net |
| 1-4 | `conduit/` — 接続ライフサイクル + Route Discovery | synergos-net |

### Phase 2: Core Daemon + IPC 基盤

| Step | 作業 | クレート |
|------|------|---------|
| 2-1 | IPC プロトコル型定義（Command/Response/Event） | synergos-ipc |
| 2-2 | IPC トランスポート（Unix Socket / Named Pipe） | synergos-ipc |
| 2-3 | Core デーモン骨格（起動・停止・シグナルハンドリング） | synergos-core |
| 2-4 | 内部 EventBus 実装 | synergos-core |
| 2-5 | IPC サーバー実装 | synergos-core |
| 2-6 | CLI コマンドハンドラ | synergos-core |

### Phase 3: サービスレイヤ移植

| Step | 作業 | クレート |
|------|------|---------|
| 3-1 | Exchange（転送制御）を core に移植 | synergos-core |
| 3-2 | Presence（ピア状態管理）を core に移植 | synergos-core |
| 3-3 | Conflict（コンフリクト管理）を core に移植 | synergos-core |

### Phase 4: Network Foundation 拡張

| Step | 作業 | クレート |
|------|------|---------|
| 4-1 | `mesh/resolver.rs` — FQDN → IPv6 解決 (DoH) | synergos-net |
| 4-2 | `mesh/turn.rs` — TURN/STUN クライアント | synergos-net |
| 4-3 | `mesh/probe.rs` — IPv6 到達性プローブ | synergos-net |
| 4-4 | `conduit/route.rs` — Route Migration | synergos-net |

### Phase 5: GUI アプリケーション

| Step | 作業 | クレート |
|------|------|---------|
| 5-1 | IPC クライアント接続・イベント受信 | synergos-gui |
| 5-2 | ネットワーク概要パネル | synergos-gui |
| 5-3 | ピア一覧パネル | synergos-gui |
| 5-4 | 転送状況パネル | synergos-gui |
| 5-5 | コンフリクト管理パネル | synergos-gui |
| 5-6 | 設定パネル | synergos-gui |

### Phase 6: Ars Plugin リファクタ + 統合

| Step | 作業 | クレート |
|------|------|---------|
| 6-1 | Ars Plugin を薄い IPC アダプタに書き換え | ars-plugin-synergos |
| 6-2 | IpcEvent → Ars EventBus ブリッジ | ars-plugin-synergos |
| 6-3 | E2E 統合テスト | tests/ |
| 6-4 | クロスプラットフォームビルド検証 | 全クレート |

## 14. 補足: オプトアウト原則の保証

Synergos がインストールされていない場合の Ars の動作:

| 機能 | Synergos あり | Synergos なし |
|------|-------------|-------------|
| ローカル編集 | 通常動作 | 通常動作 |
| ファイル保存 | 通常動作 | 通常動作 |
| WebSocket コラボ | Synergos 高速パスを併用 | WebSocket のみで動作 |
| ファイル共有 | QUIC 高速転送 | 手動（Git / 外部ツール） |
| モバイル接続 | IPv6 Direct | 不可（Web版を代替利用） |
| プレゼンス | Synergos + ars-collab 両方 | ars-collab のみ |
| 専用GUI | synergos-gui で独立操作可能 | — |

**実装上の保証:**
1. `depends_on: &[]` — 他モジュールは Synergos を依存先に含まない
2. `EventBus::subscribe()` が `None` を返す — Synergos のイベントは存在しないだけ
3. Ars 本体のコードに Synergos への直接参照を含まない
4. `ars-plugin-synergos` は `ars-core` + `synergos-ipc` のみに依存（synergos-net への直接依存なし）
5. `synergos-core` は Ars に一切依存しない — 独立した常駐アプリケーションとして動作
6. `synergos-gui` は Ars に一切依存しない — 独立したGUIアプリケーションとして動作
7. `synergos-net` は Ars に一切依存しない — モバイル等の非Arsクライアントから単体利用可能

## 15. 実装詳細設計

### 15.1 IPFS ファイルストリーム統合

ファイル転送には IPFS (InterPlanetary File System) のコンテンツアドレッシングとストリーミングを活用します。

```
┌─────────────────────────────────────────────────────────────┐
│                    File Transfer Pipeline                     │
│                                                               │
│  ローカルファイル                                             │
│       │                                                       │
│       ▼                                                       │
│  ┌──────────┐    ┌──────────────┐    ┌───────────────────┐   │
│  │ Chunker  │───▶│ IPFS DAG     │───▶│ QUIC Multiplexed │   │
│  │          │    │ Builder      │    │ Streams           │   │
│  │ 可変長   │    │              │    │                   │   │
│  │ チャンク  │    │ CID生成      │    │ ストリーム占有率  │   │
│  │ 分割     │    │ Merkle DAG   │    │ に基づく配分      │   │
│  └──────────┘    └──────────────┘    └───────────────────┘   │
│                                                               │
│  受信側:                                                      │
│  CID ベースで重複排除 → 既に保持するブロックはスキップ         │
│  Merkle DAG の検証 → 改竄検知                                 │
└─────────────────────────────────────────────────────────────┘
```

**IPFS 統合の利点:**

| 機能 | 説明 |
|------|------|
| **コンテンツアドレッシング** | CID (Content Identifier) によりファイルをハッシュで識別。同一内容は一度だけ転送 |
| **Merkle DAG** | ファイルをブロックのDAGとして構造化。部分検証・並列取得が可能 |
| **重複排除** | 既に保持しているブロック (CID) はスキップ。差分転送が自然に実現 |
| **ストリーミング** | DAG の先頭から順にストリーム。全体のダウンロード完了前に利用開始可能 |
| **Pinning** | 重要なリソースをローカルにピン留め。GCによる削除を防止 |

```rust
// synergos-net 内の IPFS 統合
pub struct IpfsBlock {
    pub cid: Cid,
    pub data: Bytes,
    pub links: Vec<Cid>,  // Merkle DAG の子ノード
}

pub struct IpfsFileStream {
    /// ファイル全体のルート CID
    pub root_cid: Cid,
    /// 全ブロック数
    pub total_blocks: u32,
    /// ブロックサイズ（可変長チャンク: Rabin fingerprint）
    pub block_sizes: Vec<u32>,
    /// 総ファイルサイズ
    pub file_size: u64,
}

/// IPFS ブロック交換プロトコル（Bitswap 簡易版）
pub trait BlockExchange: Send + Sync {
    /// 相手が保持するブロック一覧を問い合わせ
    async fn get_wantlist(&self, peer: &PeerId) -> Result<Vec<Cid>>;
    /// 指定 CID のブロックを送信
    async fn send_block(&self, peer: &PeerId, block: &IpfsBlock) -> Result<()>;
    /// ブロックの受信をリクエスト
    async fn request_block(&self, peer: &PeerId, cid: &Cid) -> Result<IpfsBlock>;
}
```

### 15.2 ファイルサイズ別ストリーム占有率制御

ファイルサイズに応じて QUIC ストリームの占有率（帯域配分）を動的に制御します。

```
┌─────────────────────────────────────────────────────────┐
│              Stream Bandwidth Allocation                  │
│                                                           │
│  Total Available Bandwidth (スピードテストで計測)         │
│  ════════════════════════════════════════════             │
│                                                           │
│  ┌─── Large (≥100MB) ───┐┌── Medium (1-100MB) ──┐┌─S─┐  │
│  │     60% 占有          ││    30% 占有          ││10%│  │
│  │                       ││                      ││   │  │
│  │  専用ストリーム ×N     ││  共有ストリーム ×M   ││共有│  │
│  │  チャンク 1 MiB       ││  チャンク 256 KiB    ││64K│  │
│  │  並列度: 高           ││  並列度: 中          ││低 │  │
│  └───────────────────────┘└──────────────────────┘└───┘  │
│                                                           │
│  S = Small (<1MB): 小ファイルは即時転送                   │
└─────────────────────────────────────────────────────────┘
```

```rust
/// ファイルサイズクラス
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileSizeClass {
    /// 100MB 以上: 大容量ファイル
    Large,
    /// 1MB 〜 100MB: 中サイズファイル
    Medium,
    /// 1MB 未満: 小ファイル
    Small,
}

impl FileSizeClass {
    pub fn classify(size: u64) -> Self {
        const MB: u64 = 1024 * 1024;
        match size {
            s if s >= 100 * MB => Self::Large,
            s if s >= MB => Self::Medium,
            _ => Self::Small,
        }
    }
}

/// ストリーム占有率設定
pub struct StreamAllocation {
    /// 帯域配分比率 (合計 100)
    pub bandwidth_ratio: BandwidthRatio,
    /// サイズクラスごとのチャンクサイズ
    pub chunk_sizes: ChunkSizeConfig,
    /// サイズクラスごとの最大同時ストリーム数
    pub max_streams: StreamLimits,
}

pub struct BandwidthRatio {
    pub large: u8,    // デフォルト: 60
    pub medium: u8,   // デフォルト: 30
    pub small: u8,    // デフォルト: 10
}

pub struct ChunkSizeConfig {
    pub large: u32,   // デフォルト: 1 MiB (1048576)
    pub medium: u32,  // デフォルト: 256 KiB (262144)
    pub small: u32,   // デフォルト: 64 KiB (65536)
}

pub struct StreamLimits {
    pub large: u16,   // スピードテスト結果に基づき動的決定
    pub medium: u16,
    pub small: u16,
}

/// 帯域スケジューラ
pub struct BandwidthScheduler {
    allocation: StreamAllocation,
    /// 現在の利用可能帯域 (bps)
    available_bandwidth: AtomicU64,
    /// サイズクラスごとの現在使用帯域
    class_usage: DashMap<FileSizeClass, AtomicU64>,
}

impl BandwidthScheduler {
    /// 転送リクエストに帯域を割り当て
    /// クラスの占有率上限に達している場合はキューに入れて待機
    pub async fn acquire(&self, class: FileSizeClass, requested_bps: u64) -> BandwidthLease {
        // ...
    }

    /// 帯域リースの返却
    pub fn release(&self, lease: BandwidthLease) {
        // ...
    }

    /// 帯域配分の動的再計算
    /// Large 転送がない場合、Medium/Small に再配分する
    pub fn rebalance(&self) {
        // ...
    }
}
```

### 15.3 スピードテストに基づくコネクション数決定

接続時にスピードテストを実施し、結果に基づいて最適なコネクション数とストリーム数を決定します。

```
┌─────────────────────────────────────────────────────────┐
│                  Speed Test Flow                         │
│                                                           │
│  接続確立直後                                             │
│       │                                                   │
│       ▼                                                   │
│  ┌──────────────┐                                        │
│  │ Probe Phase  │  3 段階のプローブ                      │
│  │              │                                        │
│  │ 1. Latency   │  QUIC Ping × 5 → RTT 中央値           │
│  │ 2. Bandwidth │  64 KiB × 10 パケット → スループット    │
│  │ 3. Capacity  │  並列ストリーム × 4 → 実効帯域          │
│  └──────┬───────┘                                        │
│         │                                                 │
│         ▼                                                 │
│  ┌──────────────┐                                        │
│  │ Calibration  │  結果に基づきパラメータ決定             │
│  │              │                                        │
│  │ bandwidth → max_connections                           │
│  │ latency  → chunk_size 調整                            │
│  │ capacity → stream 並列度                              │
│  └──────────────┘                                        │
└─────────────────────────────────────────────────────────┘
```

```rust
/// スピードテスト結果
pub struct SpeedTestResult {
    /// 往復遅延 (RTT) の中央値 (ms)
    pub rtt_median_ms: u32,
    /// RTT のジッター (ms)
    pub rtt_jitter_ms: u32,
    /// ダウンロード帯域 (bps)
    pub download_bps: u64,
    /// アップロード帯域 (bps)
    pub upload_bps: u64,
    /// 実効並列ストリーム容量
    pub effective_streams: u16,
    /// テスト実施時刻
    pub tested_at: Instant,
    /// 使用した接続経路
    pub route: RouteKind,
}

/// コネクションパラメータの算出
pub struct ConnectionCalibrator;

impl ConnectionCalibrator {
    /// スピードテスト結果からパラメータを算出
    pub fn calibrate(result: &SpeedTestResult) -> CalibratedParams {
        let bandwidth = result.download_bps.min(result.upload_bps);

        CalibratedParams {
            // 帯域に応じたコネクション数
            // 10 Mbps 未満: 1, 10-100 Mbps: 2, 100 Mbps+: 4
            max_connections: match bandwidth {
                b if b < 10_000_000 => 1,
                b if b < 100_000_000 => 2,
                _ => 4,
            },

            // Large ファイル用の並列ストリーム数
            large_streams: (result.effective_streams / 2).max(1),
            // Medium ファイル用
            medium_streams: (result.effective_streams / 4).max(1),
            // Small ファイル用
            small_streams: (result.effective_streams / 8).max(1),

            // RTT に応じたチャンクサイズ調整
            // 高レイテンシ → 大きいチャンク（往復回数削減）
            chunk_size_multiplier: if result.rtt_median_ms > 100 { 2.0 }
                                   else if result.rtt_median_ms > 50 { 1.5 }
                                   else { 1.0 },

            // 帯域制限の初期値（測定帯域の 80%）
            initial_bandwidth_limit: (bandwidth as f64 * 0.8) as u64,
        }
    }
}

pub struct CalibratedParams {
    pub max_connections: u16,
    pub large_streams: u16,
    pub medium_streams: u16,
    pub small_streams: u16,
    pub chunk_size_multiplier: f64,
    pub initial_bandwidth_limit: u64,
}
```

### 15.4 帯域優先接続 (Bandwidth-Aware Peer Selection)

ネットワーク帯域が大きいユーザーに優先的に接続し、全体のスループットを最大化します。

```
┌───────────────────────────────────────────────────────┐
│           Bandwidth-Aware Peer Selection               │
│                                                         │
│  Discovered Peers          Sorted by Bandwidth          │
│                                                         │
│  ┌────────┐               ┌────────┐  ← 優先接続       │
│  │ Peer A │  100 Mbps     │ Peer C │  500 Mbps         │
│  │ Peer B │   10 Mbps     │ Peer A │  100 Mbps         │
│  │ Peer C │  500 Mbps     │ Peer D │   50 Mbps         │
│  │ Peer D │   50 Mbps     │ Peer B │   10 Mbps         │
│  └────────┘               └────────┘  ← 後回し         │
│                                                         │
│  接続確立後も定期的にスピードテストを再実施               │
│  帯域変動に応じて接続優先度を動的に変更                   │
└───────────────────────────────────────────────────────┘
```

```rust
/// ピアの帯域情報
pub struct PeerBandwidthProfile {
    pub peer_id: PeerId,
    /// 最新のスピードテスト結果
    pub speed_test: SpeedTestResult,
    /// 帯域の移動平均 (直近 5 回分)
    pub avg_bandwidth_bps: u64,
    /// 接続の安定性スコア (0.0 - 1.0)
    pub stability_score: f64,
    /// 総合スコア（帯域 × 安定性）
    pub composite_score: f64,
}

/// ピア選択戦略
pub struct PeerSelector {
    /// 帯域プロファイル（スコア降順）
    profiles: RwLock<Vec<PeerBandwidthProfile>>,
    /// 最大同時接続ピア数
    max_peers: u16,
    /// スピードテスト再実施間隔
    retest_interval: Duration,
}

impl PeerSelector {
    /// 新しいピアを発見した際の処理
    pub async fn on_peer_discovered(&self, peer: &PeerEndpoint) -> PeerAction {
        // 1. 軽量プローブ (Ping × 3) で初期 RTT を計測
        // 2. 初期スコアを推定
        // 3. 現在の接続ピアの最低スコアと比較
        // 4. 新ピアのスコアが上回る場合 → 接続切り替え候補に
    }

    /// 定期スコア再計算
    pub async fn recalculate_scores(&self) {
        // 各ピアに対してスピードテストを再実施
        // スコアが逆転した場合は接続優先度を入れ替え
    }

    /// 接続すべきピアを優先度順に返す
    pub fn ranked_peers(&self) -> Vec<PeerId> {
        // composite_score 降順
    }
}
```

### 15.5 接続方式の自動判定 (IPv6 / Tunnel)

接続時に IPv6 到達性と Cloudflare Tunnel の可用性を自動判定し、最適な経路を選択します。

```
┌─────────────────────────────────────────────────────────┐
│              Route Auto-Detection Flow                    │
│                                                           │
│  ピア発見                                                 │
│       │                                                   │
│       ├─────────────────────────────┐                     │
│       │  (並列実行)                 │                     │
│       ▼                             ▼                     │
│  ┌──────────────┐           ┌──────────────┐             │
│  │ IPv6 Probe   │           │ Tunnel Probe │             │
│  │              │           │              │             │
│  │ 1. AAAA解決  │           │ 1. Tunnel    │             │
│  │ 2. ICMPv6    │           │    ヘルス確認 │             │
│  │    到達確認   │           │ 2. QUIC      │             │
│  │ 3. QUIC接続  │           │    ハンドシェ │             │
│  │    試行      │           │    イク試行   │             │
│  └──────┬───────┘           └──────┬───────┘             │
│         │                          │                      │
│         ▼                          ▼                      │
│  ┌──────────────────────────────────────┐                │
│  │         Route Decision Matrix        │                │
│  │                                      │                │
│  │  IPv6 OK + Tunnel OK → IPv6 優先     │                │
│  │  IPv6 OK + Tunnel NG → IPv6          │                │
│  │  IPv6 NG + Tunnel OK → Tunnel        │                │
│  │  IPv6 NG + Tunnel NG → Relay/Error   │                │
│  └──────────────────────────────────────┘                │
│                                                           │
│  選択後もバックグラウンドで代替経路をモニタリング          │
│  経路断 → 自動フォールバック (< 3秒)                      │
└─────────────────────────────────────────────────────────┘
```

```rust
/// 経路自動判定
pub struct RouteDetector {
    mesh: Arc<Mesh>,
    tunnel: Arc<TunnelManager>,
}

impl RouteDetector {
    /// 指定ピアへの最適経路を判定
    pub async fn detect(&self, peer: &PeerEndpoint) -> DetectionResult {
        // IPv6 と Tunnel のプローブを並列実行
        let (ipv6_result, tunnel_result) = tokio::join!(
            self.probe_ipv6(peer),
            self.probe_tunnel(peer),
        );

        DetectionResult {
            ipv6: ipv6_result,
            tunnel: tunnel_result,
            recommended: Self::decide(&ipv6_result, &tunnel_result),
        }
    }

    fn decide(ipv6: &ProbeResult, tunnel: &ProbeResult) -> RouteKind {
        match (ipv6.reachable, tunnel.reachable) {
            (true, _) => RouteKind::Direct,      // IPv6 最優先
            (false, true) => RouteKind::Tunnel,   // Tunnel フォールバック
            (false, false) => RouteKind::Relay,   // WebSocket リレー
        }
    }
}

pub struct ProbeResult {
    pub reachable: bool,
    pub rtt_ms: Option<u32>,
    pub error: Option<String>,
}
```

### 15.6 QUIC マルチキャスト・ブロードキャスト

複数のコネクションに対して同一データを効率的に配信するための仕組みです。
QUIC は本来ユニキャストプロトコルのため、アプリケーション層でブロードキャストを実現します。

```
┌───────────────────────────────────────────────────────────┐
│              QUIC Application-Level Broadcast               │
│                                                             │
│  送信元                                                     │
│       │                                                     │
│       ▼                                                     │
│  ┌──────────────┐                                          │
│  │ Broadcast    │   1つのデータを N 本のストリームに複製    │
│  │ Dispatcher   │                                          │
│  └──┬───┬───┬──┘                                          │
│     │   │   │                                               │
│     ▼   ▼   ▼                                               │
│  ┌──┐ ┌──┐ ┌──┐    各ピアへの QUIC ストリーム               │
│  │A │ │B │ │C │                                             │
│  └──┘ └──┘ └──┘                                             │
│                                                             │
│  最適化:                                                    │
│  ・ゼロコピー送信（同一バッファを全ストリームで共有）         │
│  ・帯域が最も広いピアから順に送信開始                        │
│  ・遅いピアには送信レートを個別に制限                        │
│  ・ブロードキャスト対象のフィルタリング（サブスクリプション） │
└───────────────────────────────────────────────────────────┘
```

```rust
/// ブロードキャストディスパッチャ
pub struct BroadcastDispatcher {
    /// 接続中のピアとそのストリーム
    peers: DashMap<PeerId, PeerConnection>,
    /// ブロードキャスト用バッファプール
    buffer_pool: Arc<BufferPool>,
}

impl BroadcastDispatcher {
    /// 全接続ピアにデータをブロードキャスト
    pub async fn broadcast(&self, data: &Bytes, opts: BroadcastOptions) -> BroadcastResult {
        let targets = self.select_targets(&opts);

        // ゼロコピー: Arc<Bytes> で全ストリームに同一バッファを共有
        let shared = Arc::new(data.clone());

        // 帯域順に送信（速いピアから）
        let mut tasks = Vec::with_capacity(targets.len());
        for peer in targets {
            let buf = Arc::clone(&shared);
            tasks.push(tokio::spawn(async move {
                peer.send_stream.write_all(&buf).await
            }));
        }

        // 全送信の完了を待機（タイムアウト付き）
        let results = futures::future::join_all(tasks).await;
        BroadcastResult::from(results)
    }

    /// ファイル全体をブロードキャスト（IPFS ブロック単位）
    pub async fn broadcast_file(
        &self,
        file_stream: &IpfsFileStream,
        opts: BroadcastOptions,
    ) -> BroadcastResult {
        // 各ブロックを順次ブロードキャスト
        // 受信済みブロック (CID) を持つピアにはスキップ
    }

    fn select_targets(&self, opts: &BroadcastOptions) -> Vec<&PeerConnection> {
        let mut targets: Vec<_> = self.peers.iter()
            .filter(|p| opts.filter.matches(&p.peer_id))
            .collect();
        // 帯域降順でソート
        targets.sort_by(|a, b| b.bandwidth_bps.cmp(&a.bandwidth_bps));
        targets
    }
}

pub struct BroadcastOptions {
    /// 送信先フィルタ（全員 / 指定ピアのみ / サブスクリプショングループ）
    pub filter: BroadcastFilter,
    /// タイムアウト
    pub timeout: Duration,
    /// 遅いピアを待つか（false = 速いピアの完了で即 return）
    pub wait_for_all: bool,
}

pub enum BroadcastFilter {
    All,
    Peers(Vec<PeerId>),
    Group(String),
}
```

### 15.7 ネットワーク状況モニタリング

リアルタイムでネットワーク状況・コネクション数・ファイル送信状況を監視する機能です。

```
┌───────────────────────────────────────────────────────────┐
│                  Network Monitor Dashboard                 │
│                                                             │
│  ┌─ Network Overview ────────────────────────────────┐     │
│  │ Route: IPv6 Direct | Bandwidth: 245 Mbps          │     │
│  │ Active Connections: 3/4 | Latency: 12ms           │     │
│  └───────────────────────────────────────────────────┘     │
│                                                             │
│  ┌─ Connections ─────────────────────────────────────┐     │
│  │ # │ Peer        │ Route  │ RTT  │ BW     │ State │     │
│  │ 1 │ Alice       │ IPv6   │ 8ms  │ 120Mb  │ ● OK  │     │
│  │ 2 │ Bob         │ Tunnel │ 45ms │ 80Mb   │ ● OK  │     │
│  │ 3 │ Charlie     │ IPv6   │ 12ms │ 245Mb  │ ● OK  │     │
│  └───────────────────────────────────────────────────┘     │
│                                                             │
│  ┌─ Active Transfers ────────────────────────────────┐     │
│  │ # │ File            │ Size  │ Progress │ Speed    │     │
│  │ 1 │ scene.bin       │ 250MB │ ████░ 72%│ 85 MB/s │     │
│  │ 2 │ textures.pak    │ 45MB  │ ██░░░ 35%│ 42 MB/s │     │
│  │ 3 │ config.json     │ 12KB  │ ████████ │ Done     │     │
│  └───────────────────────────────────────────────────┘     │
│                                                             │
│  ┌─ Bandwidth History (last 60s) ────────────────────┐     │
│  │ 250 ┤                          ╭──╮                │     │
│  │ 200 ┤              ╭───────────╯  ╰──╮            │     │
│  │ 150 ┤    ╭────────╯                   ╰───        │     │
│  │ 100 ┤───╯                                         │     │
│  │  50 ┤                                             │     │
│  │   0 ┼────────────────────────────────────────     │     │
│  │     0s          20s          40s          60s     │     │
│  └───────────────────────────────────────────────────┘     │
└───────────────────────────────────────────────────────────┘
```

```rust
/// ネットワークモニター（synergos-net レイヤ）
pub struct NetworkMonitor {
    /// メトリクスの収集間隔
    interval: Duration,
    /// スナップショット履歴（直近 N 件）
    history: RwLock<VecDeque<NetworkSnapshot>>,
    /// リアルタイム通知用チャンネル
    subscribers: broadcast::Sender<NetworkSnapshot>,
}

/// ネットワーク状況のスナップショット
#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub timestamp: Instant,
    pub overview: NetworkOverview,
    pub connections: Vec<ConnectionStatus>,
    pub transfers: Vec<TransferStatus>,
}

#[derive(Debug, Clone)]
pub struct NetworkOverview {
    /// 現在の接続経路種別
    pub primary_route: RouteKind,
    /// 総利用可能帯域 (bps)
    pub total_bandwidth_bps: u64,
    /// 現在使用中帯域 (bps)
    pub used_bandwidth_bps: u64,
    /// アクティブコネクション数
    pub active_connections: u16,
    /// 最大コネクション数
    pub max_connections: u16,
    /// 平均レイテンシ (ms)
    pub avg_latency_ms: u32,
    /// パケットロス率 (0.0 - 1.0)
    pub packet_loss_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub peer_id: PeerId,
    pub display_name: String,
    pub route: RouteKind,
    pub rtt_ms: u32,
    pub bandwidth_bps: u64,
    pub state: ConnectionState,
    /// この接続での総転送量
    pub bytes_transferred: u64,
    /// 接続時間
    pub connected_since: Instant,
}

#[derive(Debug, Clone)]
pub struct TransferStatus {
    pub transfer_id: TransferId,
    pub file_name: String,
    pub file_size: u64,
    pub size_class: FileSizeClass,
    pub bytes_transferred: u64,
    pub speed_bps: u64,
    pub direction: TransferDirection,
    pub peer_id: PeerId,
    pub state: TransferState,
}

#[derive(Debug, Clone, Copy)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone)]
pub enum TransferState {
    Queued,
    Active { progress_pct: f32 },
    Paused { reason: String },
    Completed,
    Failed { error: String },
}

impl NetworkMonitor {
    /// モニタリング開始
    pub async fn start(&self) -> Result<()> {
        // interval ごとにスナップショットを収集
        // subscribers に通知
    }

    /// 最新のスナップショットを取得
    pub fn current(&self) -> NetworkSnapshot { /* ... */ }

    /// 履歴を取得（直近 N 件）
    pub fn history(&self, count: usize) -> Vec<NetworkSnapshot> { /* ... */ }

    /// リアルタイム更新を購読
    pub fn subscribe(&self) -> broadcast::Receiver<NetworkSnapshot> {
        self.subscribers.subscribe()
    }
}
```

**Ars Plugin Layer でのモニター統合:**

```rust
/// Ars EventBus 経由でモニタリングデータを配信
#[derive(Debug, Clone)]
pub struct NetworkStatusUpdated {
    pub snapshot: NetworkSnapshot,
}

impl ArsEvent for NetworkStatusUpdated {
    fn source_module(&self) -> &'static str { "plugin-synergos" }
    fn category(&self) -> &'static str { "monitor" }
}

// Plugin Layer で NetworkMonitor を EventBus にブリッジ
async fn bridge_monitor(monitor: &NetworkMonitor, bus: &EventBus) {
    let mut rx = monitor.subscribe();
    while let Ok(snapshot) = rx.recv().await {
        bus.emit(NetworkStatusUpdated { snapshot }).await;
    }
}
```

### 15.8 設定（追加分）

```toml
# synergos.toml に追加

# --- IPFS 設定 ---
[ipfs]
# ローカル IPFS ブロックストアのパス
block_store_path = ".ars/synergos/blocks"
# ブロックストアの最大サイズ (bytes, 0 = 無制限)
max_store_size = 0
# GC 閾値（ストア使用率がこの値を超えたらピンなしブロックを削除）
gc_threshold = 0.8
# Rabin fingerprint チャンク分割の設定
rabin_min = 65536       # 64 KiB
rabin_avg = 262144      # 256 KiB
rabin_max = 1048576     # 1 MiB

# --- ストリーム占有率設定 ---
[stream_allocation]
# 帯域配分比率（合計 100）
large_ratio = 60
medium_ratio = 30
small_ratio = 10

# --- スピードテスト設定 ---
[speed_test]
# 接続時にスピードテストを実施するか
enabled = true
# スピードテストの再実施間隔 (秒)
retest_interval_secs = 300
# プローブパケット数
probe_count = 10

# --- ピア選択設定 ---
[peer_selection]
# 帯域スコアの重み (0.0 - 1.0)
bandwidth_weight = 0.7
# 安定性スコアの重み (0.0 - 1.0)
stability_weight = 0.3
# スコア再計算間隔 (秒)
recalculate_interval_secs = 60

# --- モニタリング設定 ---
[monitor]
# スナップショット収集間隔 (ミリ秒)
snapshot_interval_ms = 1000
# 履歴保持数
history_size = 3600
# 帯域履歴のグラフ用サンプリング間隔 (秒)
graph_sample_interval_secs = 1

# --- DHT/Gossipsub 設定 ---
[dht]
# Kademlia k-bucket サイズ
k_bucket_size = 20
# ルーティングテーブル更新間隔 (秒)
routing_refresh_secs = 60
# ピアのアクティブ情報 TTL (秒)
peer_ttl_secs = 120

[gossipsub]
# Gossip メッシュの目標ピア数
mesh_n = 6
# メッシュの下限（これを下回ると新規ピアを追加）
mesh_n_low = 4
# メッシュの上限（これを超えると刈り込み）
mesh_n_high = 12
# ハートビート間隔 (ミリ秒)
heartbeat_interval_ms = 1000
# メッセージキャッシュ保持数（重複排除用）
message_cache_size = 1000

# --- カタログ設定 ---
[catalog]
# チャンクあたりの最大ファイル数
chunk_max_files = 256
# ファイル更新履歴の保持件数
history_depth = 10
# カタログ同期間隔 (秒)
sync_interval_secs = 30

# --- コンフリクト設定 ---
[conflict]
# コンフリクト通知のリトライ間隔 (秒)
notify_retry_secs = 30
# ホットスタンバイ情報の保持期間 (秒)
hot_standby_ttl_secs = 86400
```

## 16. ピアネットワーク — DHT + Gossipsub メッシュ

### 13.1 概要

ピアノードの発見・管理には **DHT (分散ハッシュテーブル)** を採用し、プロジェクト単位のメッシュネットワーク形成には **Gossipsub** プロトコルを参考にした Pub/Sub を用いる。

ユーザーのアクティブ情報は各ノードが直接保持するのではなく、**他のピアから Gossip で受信した情報**として伝播する。自分自身の情報も他者のビューを通じて共有される。

```
┌────────────────────────────────────────────────────────────┐
│           Project-Scoped Mesh Network                       │
│                                                              │
│   DHT (Kademlia)                                            │
│   ┌──────────────────────────────────────────────┐          │
│   │  グローバルピアレジストリ                     │          │
│   │  Key: PeerId → Value: PeerEndpoint            │          │
│   │  各ノードが k-bucket で近傍ピアを管理         │          │
│   └──────────────────────────────────────────────┘          │
│          │                                                   │
│          ▼                                                   │
│   Gossipsub (プロジェクト単位の Topic)                       │
│   ┌──────────────────────────────────────────────┐          │
│   │  Topic: "project/<project_id>"               │          │
│   │                                               │          │
│   │      ┌───┐    ┌───┐    ┌───┐                 │          │
│   │      │ A │◄──►│ B │◄──►│ C │  ← メッシュ     │          │
│   │      └─┬─┘    └─┬─┘    └───┘                 │          │
│   │        │        │                             │          │
│   │      ┌─▼─┐    ┌─▼─┐                          │          │
│   │      │ D │◄──►│ E │         ← ファンアウト    │          │
│   │      └───┘    └───┘                           │          │
│   └──────────────────────────────────────────────┘          │
│                                                              │
│   各ノードは以下を Gossip で交換:                            │
│   ・自身のアクティブ状態                                     │
│   ・カタログ更新通知                                         │
│   ・ファイル更新要求 / 送信通知                              │
│   ・コンフリクト通知                                         │
└────────────────────────────────────────────────────────────┘
```

### 13.2 DHT (Kademlia)

```rust
// synergos-net レイヤ

/// DHT ノード
pub struct DhtNode {
    /// 自身のノードID（PeerId から導出）
    pub node_id: NodeId,
    /// Kademlia ルーティングテーブル
    routing_table: RoutingTable,
    /// ノード情報ストア
    store: DashMap<NodeId, PeerRecord>,
}

/// ルーティングテーブル（k-bucket 方式）
pub struct RoutingTable {
    /// 160-bit 空間を距離ベースの k-bucket に分割
    buckets: Vec<KBucket>,
    /// k-bucket サイズ（デフォルト: 20）
    k: usize,
}

pub struct KBucket {
    pub entries: Vec<BucketEntry>,
    pub last_refreshed: Instant,
}

pub struct BucketEntry {
    pub node_id: NodeId,
    pub peer_id: PeerId,
    pub endpoints: Vec<Route>,
    pub last_seen: Instant,
    pub rtt_ms: Option<u32>,
}

/// DHT に保存するピアレコード
pub struct PeerRecord {
    pub peer_id: PeerId,
    pub display_name: String,
    pub endpoints: Vec<Route>,
    /// 参加中のプロジェクト一覧
    pub active_projects: Vec<String>,
    /// レコード発行時刻
    pub published_at: Instant,
    /// TTL（この時間を過ぎたら失効）
    pub ttl: Duration,
}

impl DhtNode {
    /// ピアを検索（Kademlia FIND_NODE）
    pub async fn find_peer(&self, peer_id: &PeerId) -> Option<PeerRecord> { /* ... */ }

    /// 自身のレコードを DHT に公開
    pub async fn announce(&self, record: PeerRecord) -> Result<()> { /* ... */ }

    /// 特定プロジェクトに参加しているピアを検索
    pub async fn find_project_peers(&self, project_id: &str) -> Vec<PeerRecord> { /* ... */ }

    /// ルーティングテーブルのリフレッシュ
    pub async fn refresh_routing_table(&self) { /* ... */ }
}
```

### 13.3 Gossipsub メッシュ

プロジェクト単位の Topic に基づくメッシュネットワークを構成する。

```rust
// synergos-net レイヤ

/// Gossipsub ノード
pub struct GossipNode {
    /// メッシュピア（フルメッセージを交換する相手）
    mesh: DashMap<TopicId, Vec<PeerId>>,
    /// ファンアウトピア（メッシュ外だが Gossip するキューする相手）
    fanout: DashMap<TopicId, Vec<PeerId>>,
    /// メッセージキャッシュ（重複排除用）
    message_cache: MessageCache,
    /// パラメータ
    params: GossipsubParams,
}

pub struct GossipsubParams {
    /// メッシュの目標ピア数
    pub mesh_n: usize,      // デフォルト: 6
    /// メッシュ下限
    pub mesh_n_low: usize,  // デフォルト: 4
    /// メッシュ上限
    pub mesh_n_high: usize, // デフォルト: 12
    /// ハートビート間隔
    pub heartbeat_interval: Duration,
}

/// Gossip メッセージの種類
#[derive(Debug, Clone)]
pub enum GossipMessage {
    /// ピアのアクティブ状態
    PeerStatus {
        peer_id: PeerId,
        status: PeerActivityStatus,
        /// この情報を最初に発信したピア
        origin: PeerId,
        /// ホップ数（伝播距離）
        hops: u8,
    },
    /// カタログ更新通知
    CatalogUpdate {
        project_id: String,
        root_crc: u32,
        update_count: u64,
        updated_chunks: Vec<ChunkId>,
    },
    /// ファイル更新要求（受信側が「欲しい」と宣言）
    FileWant {
        requester: PeerId,
        file_id: FileId,
        version: u64,
    },
    /// ファイル送信通知（送信側が「送りたい」と宣言）
    FileOffer {
        sender: PeerId,
        file_id: FileId,
        version: u64,
        size: u64,
        crc: u32,
    },
    /// コンフリクト通知
    ConflictAlert {
        file_id: FileId,
        conflicting_nodes: Vec<PeerId>,
        their_versions: Vec<u64>,
    },
}

#[derive(Debug, Clone)]
pub struct PeerActivityStatus {
    pub peer_id: PeerId,
    pub display_name: String,
    pub state: ActivityState,
    /// 最終アクティブ時刻
    pub last_active: Instant,
    /// 作業中のファイル（任意）
    pub working_on: Option<Vec<FileId>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ActivityState {
    /// アクティブに作業中
    Active,
    /// アイドル（接続はしている）
    Idle,
    /// 離席
    Away,
    /// オフライン（他ピアのキャッシュ情報）
    Offline,
}

/// メッセージキャッシュ（重複排除）
pub struct MessageCache {
    /// メッセージID → 受信時刻
    seen: DashMap<MessageId, Instant>,
    /// キャッシュサイズ上限
    max_size: usize,
}

impl GossipNode {
    /// Topic を購読（プロジェクト参加時）
    pub async fn subscribe(&self, topic: TopicId) -> Result<()> { /* ... */ }

    /// Topic から退出（プロジェクト離脱時）
    pub async fn unsubscribe(&self, topic: TopicId) -> Result<()> { /* ... */ }

    /// メッセージをメッシュに配信
    pub async fn publish(&self, topic: &TopicId, message: GossipMessage) -> Result<()> {
        // 1. メッセージID を生成しキャッシュに登録
        // 2. メッシュピアにフルメッセージを送信
        // 3. ファンアウトピアには IHAVE を送信
    }

    /// ハートビート処理（定期実行）
    async fn heartbeat(&self) {
        // 1. メッシュサイズが mesh_n_low 未満 → GRAFT でピア追加
        // 2. メッシュサイズが mesh_n_high 超過 → PRUNE で刈り込み
        // 3. IHAVE/IWANT 交換で見逃したメッセージを補完
        // 4. メッセージキャッシュの古いエントリを削除
    }
}
```

### 13.4 ピア情報の伝播モデル

ユーザーの情報は「他人からもらう情報」として伝播する。

```
┌──────────────────────────────────────────────────────┐
│          Peer Information Propagation                  │
│                                                        │
│  Node A が「自分はアクティブ」と発信:                  │
│                                                        │
│  A ──(PeerStatus)──► B                                │
│                       │                                │
│                       ├──(PeerStatus, hops+1)──► C    │
│                       │                                │
│                       └──(PeerStatus, hops+1)──► D    │
│                                                        │
│  C が B から受信した A の情報を D に転送:              │
│  C ──(PeerStatus, hops+2)──► D                        │
│                                                        │
│  D は B からの直接情報 (hops=1) と                     │
│  C 経由の情報 (hops=2) の両方を受信。                  │
│  hops が小さい方を採用。                               │
│                                                        │
│  → 各ノードは他者のビューを通じてネットワーク全体の    │
│    アクティブ状態を把握する                             │
└──────────────────────────────────────────────────────┘
```

## 17. ファイル更新モデル — ユーザー起点更新 + チェーン

### 14.1 更新トリガーの原則

ファイル更新は **ユーザーが「更新したい」と明示的に操作したタイミング** でのみ行う。
自動保存で帯域を消費しないための設計判断。

```
┌───────────────────────────────────────────────────────────┐
│                  Update Trigger Flow                        │
│                                                             │
│  ユーザー操作                                               │
│       │                                                     │
│       ▼                                                     │
│  「更新を公開」ボタン / コマンド                            │
│       │                                                     │
│       ├─ テキスト分解可能 ──► git diff 生成 ──► チェーン書込 │
│       │                                                     │
│       └─ バイナリ ──────────► IPFS チャンク ──► チェーン書込 │
│                                                             │
│       │                                                     │
│       ▼                                                     │
│  Gossipsub: FileOffer メッセージ配信                        │
│       │                                                     │
│       ▼                                                     │
│  受信側が「欲しい」と判断 → FileWant をチェーンに書込       │
│       │                                                     │
│       ▼                                                     │
│  マッチング（Want ⇔ Offer）成立 → 転送開始                  │
│                                                             │
│  ※ 自動保存はローカルのみ。ネットワークには流さない         │
└───────────────────────────────────────────────────────────┘
```

### 14.2 テキストファイルの差分管理

テキストで分解可能なファイル（ソースコード、JSON、YAML、TOML 等）は git の diff アルゴリズムで差分を生成し、チェーンに書き込む。

```rust
// ars-plugin-synergos レイヤ

/// テキスト差分
pub struct TextDiff {
    pub file_id: FileId,
    pub base_version: u64,
    pub new_version: u64,
    /// unified diff 形式
    pub patch: String,
    /// パッチの CRC
    pub patch_crc: u32,
    /// 適用後のファイル全体 CRC
    pub result_crc: u32,
}

/// ファイル種別の判定
pub enum FileContentType {
    /// テキスト分解可能 → diff でチェーン書込
    Text,
    /// バイナリ → IPFS ブロック転送
    Binary,
}

impl FileContentType {
    pub fn detect(path: &Path, content: &[u8]) -> Self {
        // 1. 拡張子による判定 (.rs, .json, .yaml, .toml, .md, etc.)
        // 2. 内容のバイナリ判定（NUL バイトの有無）
    }
}
```

### 14.3 チェーン（更新履歴の直列管理）

ファイルごとに直近 N 件の更新履歴をブロックチェーン類似の直列構造で管理する。

```
┌─────────────────────────────────────────────────────────┐
│              File Update Chain                            │
│                                                           │
│  ┌──────────┐   ┌──────────┐   ┌──────────┐             │
│  │ Block 0  │──►│ Block 1  │──►│ Block 2  │ (HEAD)      │
│  │          │   │          │   │          │             │
│  │ ver: 1   │   │ ver: 2   │   │ ver: 3   │             │
│  │ prev: ∅  │   │ prev: h0 │   │ prev: h1 │             │
│  │ author:A │   │ author:B │   │ author:A │             │
│  │ hash: h0 │   │ hash: h1 │   │ hash: h2 │             │
│  │ type:full│   │ type:diff│   │ type:diff│             │
│  │ data:... │   │ data:... │   │ data:... │             │
│  └──────────┘   └──────────┘   └──────────┘             │
│                                                           │
│  ルール:                                                  │
│  ・ツリーは必ず直列（分岐しない）                         │
│  ・直近 N 件を保持（古いブロックは GC）                   │
│  ・各ブロックは前ブロックのハッシュを prev に持つ         │
│  ・分岐が発生 → コンフリクト状態                          │
└─────────────────────────────────────────────────────────┘
```

```rust
// synergos-net レイヤ（チェーン構造はネットワーク基盤の一部）

/// チェーンブロック
#[derive(Debug, Clone)]
pub struct ChainBlock {
    /// ブロックのハッシュ (Blake3)
    pub hash: Blake3Hash,
    /// 前ブロックのハッシュ（先頭ブロックは None）
    pub prev_hash: Option<Blake3Hash>,
    /// バージョン番号（単調増加）
    pub version: u64,
    /// 作成者
    pub author: PeerId,
    /// 作成時刻
    pub timestamp: u64,
    /// 更新内容
    pub payload: ChainPayload,
}

#[derive(Debug, Clone)]
pub enum ChainPayload {
    /// テキストファイルの差分
    TextDiff {
        patch: String,
        result_crc: u32,
    },
    /// バイナリファイルの IPFS CID 参照
    BinaryCid {
        cid: Cid,
        file_size: u64,
        crc: u32,
    },
    /// フルスナップショット（初回 or 定期的なベースライン）
    FullSnapshot {
        cid: Cid,
        file_size: u64,
        crc: u32,
    },
}

/// ファイルごとの更新チェーン
pub struct FileChain {
    pub file_id: FileId,
    /// 直近 N 件のブロック（古い順）
    blocks: VecDeque<ChainBlock>,
    /// 保持件数上限
    max_depth: usize,
    /// HEAD ブロックのハッシュ
    pub head: Option<Blake3Hash>,
}

impl FileChain {
    /// 新しいブロックを追加
    pub fn append(&mut self, block: ChainBlock) -> Result<(), ChainError> {
        // prev_hash が現在の HEAD と一致するか検証
        if block.prev_hash != self.head {
            return Err(ChainError::Fork {
                expected: self.head.clone(),
                got: block.prev_hash.clone(),
            });
        }
        self.blocks.push_back(block.clone());
        self.head = Some(block.hash);
        // 上限超過分を削除
        while self.blocks.len() > self.max_depth {
            self.blocks.pop_front();
        }
        Ok(())
    }

    /// 指定バージョンからの差分ブロック列を取得
    pub fn blocks_since(&self, version: u64) -> Vec<&ChainBlock> {
        self.blocks.iter().filter(|b| b.version > version).collect()
    }

    /// HEAD の CRC を取得
    pub fn head_crc(&self) -> Option<u32> {
        self.blocks.back().map(|b| match &b.payload {
            ChainPayload::TextDiff { result_crc, .. } => *result_crc,
            ChainPayload::BinaryCid { crc, .. } => *crc,
            ChainPayload::FullSnapshot { crc, .. } => *crc,
        })
    }
}

#[derive(Debug)]
pub enum ChainError {
    /// チェーンの分岐を検出（コンフリクト）
    Fork {
        expected: Option<Blake3Hash>,
        got: Option<Blake3Hash>,
    },
}
```

### 14.4 Want / Offer チェーン — 重複受け取りブロック

受信側は「欲しいファイル」、送信側は「送りたいファイル」をそれぞれチェーンに書き込むことで、転送の意図を明確にし、重複転送を防止する。

```
┌───────────────────────────────────────────────────────────┐
│            Want / Offer Ledger (転送台帳)                   │
│                                                             │
│  ┌─── Want Chain ───────────────────────────────────┐      │
│  │ 受信者が「このファイルの version N が欲しい」    │      │
│  │                                                   │      │
│  │  { requester: B, file: f1, ver: 3 }              │      │
│  │  { requester: C, file: f2, ver: 5 }              │      │
│  │  { requester: D, file: f1, ver: 3 } ← 重複      │      │
│  └───────────────────────────────────────────────────┘      │
│                                                             │
│  ┌─── Offer Chain ──────────────────────────────────┐      │
│  │ 送信者が「このファイルの version N を送りたい」   │      │
│  │                                                   │      │
│  │  { sender: A, file: f1, ver: 3, size: 10MB }     │      │
│  │  { sender: A, file: f2, ver: 5, size: 500KB }    │      │
│  └───────────────────────────────────────────────────┘      │
│                                                             │
│  マッチングロジック:                                        │
│  ・Want と Offer の (file_id, version) が一致 → 転送開始    │
│  ・同一 (file_id, version) への Want 重複 → 2件目以降は無視 │
│  ・Offer が先行している場合 → Want 到着時に即座にマッチ     │
│  ・Want が先行している場合 → Offer 到着時にマッチ           │
│  ・マッチ済みエントリは Fulfilled 状態に遷移                │
└───────────────────────────────────────────────────────────┘
```

```rust
// synergos-net レイヤ

/// Want エントリ
#[derive(Debug, Clone)]
pub struct WantEntry {
    pub requester: PeerId,
    pub file_id: FileId,
    pub version: u64,
    pub requested_at: u64,
    pub state: LedgerEntryState,
}

/// Offer エントリ
#[derive(Debug, Clone)]
pub struct OfferEntry {
    pub sender: PeerId,
    pub file_id: FileId,
    pub version: u64,
    pub file_size: u64,
    pub crc: u32,
    pub offered_at: u64,
    pub state: LedgerEntryState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LedgerEntryState {
    /// 待機中（マッチ相手を待っている）
    Pending,
    /// マッチ済み（転送中）
    Matched,
    /// 転送完了
    Fulfilled,
    /// 取り消し
    Cancelled,
}

/// Want/Offer 転送台帳
pub struct TransferLedger {
    wants: DashMap<(FileId, u64), Vec<WantEntry>>,
    offers: DashMap<(FileId, u64), OfferEntry>,
}

impl TransferLedger {
    /// Want を登録（重複チェック付き）
    pub fn register_want(&self, want: WantEntry) -> LedgerAction {
        let key = (want.file_id.clone(), want.version);

        // 重複チェック: 同じ requester が同じ (file, version) を既に Want していたら無視
        if let Some(existing) = self.wants.get(&key) {
            if existing.iter().any(|w| w.requester == want.requester) {
                return LedgerAction::Duplicate;
            }
        }

        // Offer が既にあるか確認
        if let Some(offer) = self.offers.get(&key) {
            if offer.state == LedgerEntryState::Pending {
                return LedgerAction::Match {
                    sender: offer.sender.clone(),
                    file_size: offer.file_size,
                };
            }
        }

        self.wants.entry(key).or_default().push(want);
        LedgerAction::Queued
    }

    /// Offer を登録
    pub fn register_offer(&self, offer: OfferEntry) -> Vec<LedgerAction> {
        let key = (offer.file_id.clone(), offer.version);
        let mut actions = Vec::new();

        // 待機中の Want をすべてマッチ
        if let Some(wants) = self.wants.get(&key) {
            for want in wants.iter().filter(|w| w.state == LedgerEntryState::Pending) {
                actions.push(LedgerAction::Match {
                    sender: offer.sender.clone(),
                    file_size: offer.file_size,
                });
            }
        }

        self.offers.insert(key, offer);
        actions
    }
}

pub enum LedgerAction {
    /// キューに追加された
    Queued,
    /// 重複のため無視
    Duplicate,
    /// マッチ成立 → 転送開始
    Match { sender: PeerId, file_size: u64 },
}
```

## 18. カタログシステム

### 15.1 構造

プロジェクトのファイル群を階層的に管理するカタログシステム。

```
┌─────────────────────────────────────────────────────────────┐
│                      Catalog Structure                        │
│                                                               │
│  ┌──────────────────────────────────────────────────┐        │
│  │              Root Catalog                         │        │
│  │              (プロジェクトに 1 つ)                 │        │
│  │                                                   │        │
│  │  project_id: "proj-001"                          │        │
│  │  update_count: 42      ← 累積更新件数            │        │
│  │  chunks: [                                        │        │
│  │    { chunk_id: c0, crc: 0xA1B2C3D4, updated: T1 }│        │
│  │    { chunk_id: c1, crc: 0xE5F6A7B8, updated: T2 }│        │
│  │    { chunk_id: c2, crc: 0x12345678, updated: T3 }│        │
│  │  ]                                                │        │
│  └───────────┬──────────┬──────────┬─────────────────┘        │
│              │          │          │                           │
│         ┌────▼───┐ ┌───▼────┐ ┌───▼────┐                    │
│         │Chunk c0│ │Chunk c1│ │Chunk c2│                    │
│         │        │ │        │ │        │                    │
│         │ files: │ │ files: │ │ files: │ ← 追加順で配置     │
│         │ [f0,f1,│ │ [f4,f5,│ │ [f8,f9]│                    │
│         │  f2,f3]│ │  f6,f7]│ │        │                    │
│         │        │ │        │ │        │                    │
│         │各ファイル│ │各ファイル│ │各ファイル│                    │
│         │のCRC   │ │のCRC   │ │のCRC   │                    │
│         │と状態  │ │と状態  │ │と状態  │                    │
│         └────────┘ └────────┘ └────────┘                    │
│                                                               │
│  別管理: 各ファイルの更新履歴 (FileChain)                     │
│  ┌─────────────────────────────────────┐                     │
│  │ f0: [Block0] → [Block1] → [Block2] │  直近 N 件          │
│  │ f1: [Block0] → [Block1]            │                     │
│  │ ...                                 │                     │
│  └─────────────────────────────────────┘                     │
└─────────────────────────────────────────────────────────────┘
```

### 15.2 データ構造

```rust
// synergos-net レイヤ

/// ルートカタログ（プロジェクトに 1 つ）
#[derive(Debug, Clone)]
pub struct RootCatalog {
    pub project_id: String,
    /// 累積更新件数（単調増加）
    pub update_count: u64,
    /// チャンクのインデックス
    pub chunks: Vec<ChunkIndex>,
    /// カタログ全体の CRC（chunks の CRC から算出）
    pub catalog_crc: u32,
    /// 最終更新時刻
    pub last_updated: u64,
}

/// チャンクインデックス（ルートカタログ内のエントリ）
#[derive(Debug, Clone)]
pub struct ChunkIndex {
    pub chunk_id: ChunkId,
    /// このチャンク内ファイルの CRC を合成した値
    pub crc: u32,
    /// このチャンクの最終更新時刻
    pub last_updated: u64,
}

/// チャンク（ファイル群のまとまり）
#[derive(Debug, Clone)]
pub struct Chunk {
    pub chunk_id: ChunkId,
    /// ファイルエントリ（追加順）
    pub files: Vec<FileEntry>,
    /// チャンクあたりの最大ファイル数
    pub max_files: usize,
}

/// チャンク内のファイルエントリ
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub file_id: FileId,
    /// ファイルパス（プロジェクトルート相対）
    pub path: String,
    /// 現在のファイル CRC
    pub crc: u32,
    /// ファイルの状態
    pub state: FileState,
    /// ファイルサイズ
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileState {
    /// 最新（全ノードで一致）
    Synced,
    /// ローカルで変更あり（未公開）
    LocalModified,
    /// リモートで更新あり（未取得）
    RemoteUpdated,
    /// コンフリクト状態
    Conflict,
    /// 削除済み
    Deleted,
}

/// カタログマネージャ
pub struct CatalogManager {
    root: RwLock<RootCatalog>,
    chunks: DashMap<ChunkId, Chunk>,
    /// 各ファイルの更新チェーン
    chains: DashMap<FileId, FileChain>,
}

impl CatalogManager {
    /// ファイルを追加（次の空きチャンクに配置）
    pub fn add_file(&self, path: &str, crc: u32, size: u64) -> FileId { /* ... */ }

    /// ファイル更新をチェーンに記録 + カタログ CRC 更新
    pub fn record_update(&self, file_id: &FileId, block: ChainBlock) -> Result<(), ChainError> {
        // 1. FileChain にブロック追加
        // 2. FileEntry の CRC を更新
        // 3. 所属 Chunk の CRC を再計算
        // 4. RootCatalog の update_count を +1
        // 5. RootCatalog の catalog_crc を再計算
        todo!()
    }

    /// リモートカタログとの差分を検出
    pub fn diff_catalog(&self, remote: &RootCatalog) -> CatalogDiff { /* ... */ }
}
```

### 15.3 カタログベースの更新検知

```
┌───────────────────────────────────────────────────────────┐
│              Catalog-Based Update Detection                  │
│                                                             │
│  1. Gossip で RootCatalog の update_count を受信            │
│     │                                                       │
│     ▼                                                       │
│  2. ローカル update_count と比較                            │
│     │                                                       │
│     ├── 一致 → 更新なし（何もしない）                       │
│     │                                                       │
│     └── 不一致 → 3. チャンク CRC を比較                     │
│                    │                                        │
│                    ▼                                        │
│              4. CRC が異なるチャンクのみ詳細比較             │
│                    │                                        │
│                    ▼                                        │
│              5. チャンク内の各ファイル CRC を比較            │
│                    │                                        │
│                    ├── ファイル CRC 不一致                   │
│                    │   └── 更新履歴 (FileChain) を確認      │
│                    │       │                                │
│                    │       ├── カタログの更新情報との差分のみ │
│                    │       │   → 最新ファイルで更新          │
│                    │       │                                │
│                    │       └── ローカル差分あり + ツリー競合  │
│                    │           → コンフリクト状態            │
│                    │                                        │
│                    └── ファイル CRC 一致 → スキップ          │
└───────────────────────────────────────────────────────────┘
```

```rust
/// カタログ差分
pub struct CatalogDiff {
    /// 更新が必要なファイル
    pub updates: Vec<FileUpdateAction>,
    /// コンフリクトが発生したファイル
    pub conflicts: Vec<ConflictInfo>,
}

pub enum FileUpdateAction {
    /// リモートの最新版で更新
    ApplyRemote {
        file_id: FileId,
        blocks: Vec<ChainBlock>,
    },
    /// 新規ファイルの取得
    FetchNew {
        file_id: FileId,
        cid: Cid,
    },
    /// ファイル削除
    Remove {
        file_id: FileId,
    },
}
```

## 19. コンフリクト管理

### 16.1 コンフリクトの検出

ローカルで変更があり、かつリモートのチェーンとツリーが分岐している場合にコンフリクトが発生する。

```
┌───────────────────────────────────────────────────────────┐
│              Conflict Detection                             │
│                                                             │
│  ローカルチェーン:                                          │
│  [B0] → [B1] → [B2] → [B3_local]                          │
│                                                             │
│  リモートチェーン:                                          │
│  [B0] → [B1] → [B2] → [B3_remote]                         │
│                                                             │
│  B2 までは共通だが B3 で分岐 → コンフリクト                 │
│                                                             │
│  ┌──────────────────────────────────────────┐              │
│  │ ConflictInfo                              │              │
│  │                                           │              │
│  │ file_id: f1                               │              │
│  │ common_ancestor: B2 (version 3)           │              │
│  │ local_head: B3_local (version 4, author A)│              │
│  │ remote_head: B3_remote (version 4, authorB│)             │
│  │ state: Active                             │              │
│  └──────────────────────────────────────────┘              │
└───────────────────────────────────────────────────────────┘
```

### 16.2 コンフリクト状態の管理

```rust
// ars-plugin-synergos レイヤ

/// コンフリクト情報
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub file_id: FileId,
    /// 共通の祖先ブロック
    pub common_ancestor: ChainBlock,
    /// ローカルの HEAD
    pub local_head: ChainBlock,
    /// リモートの HEAD
    pub remote_head: ChainBlock,
    /// コンフリクトに関与するノード
    pub involved_nodes: Vec<PeerId>,
    /// コンフリクト検出時刻
    pub detected_at: u64,
    /// 状態
    pub state: ConflictState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConflictState {
    /// アクティブ（未解決）
    Active,
    /// 解決済み（どちらかを採用）
    Resolved { chosen: ConflictResolution },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConflictResolution {
    /// ローカル版を採用
    KeepLocal,
    /// リモート版を採用
    AcceptRemote,
    /// 手動マージ済み
    ManualMerge,
}

/// コンフリクトマネージャ
pub struct ConflictManager {
    /// アクティブなコンフリクト
    conflicts: DashMap<FileId, ConflictInfo>,
    /// ホットスタンバイ情報（オフラインノード向け）
    hot_standby: DashMap<PeerId, Vec<ConflictNotification>>,
    /// 通知リトライキュー
    notify_queue: RwLock<Vec<PendingNotification>>,
}

/// コンフリクト通知
#[derive(Debug, Clone)]
pub struct ConflictNotification {
    pub conflict: ConflictInfo,
    pub notified_at: u64,
    pub target_peer: PeerId,
}

/// 保留中の通知（オフラインノード向け）
#[derive(Debug, Clone)]
pub struct PendingNotification {
    pub notification: ConflictNotification,
    pub retry_count: u32,
    pub next_retry_at: u64,
}

impl ConflictManager {
    /// コンフリクトを検出・登録
    pub fn detect_conflict(
        &self,
        file_id: &FileId,
        local_chain: &FileChain,
        remote_block: &ChainBlock,
    ) -> Option<ConflictInfo> {
        // local_chain の HEAD と remote_block の prev_hash が異なる場合
        // → 分岐点（共通祖先）を特定してコンフリクト生成
        todo!()
    }

    /// コンフリクトを関与ノードに通知
    pub async fn notify_conflict(
        &self,
        conflict: &ConflictInfo,
        gossip: &GossipNode,
        topic: &TopicId,
    ) {
        // 1. Gossipsub で ConflictAlert を配信
        gossip.publish(topic, GossipMessage::ConflictAlert {
            file_id: conflict.file_id.clone(),
            conflicting_nodes: conflict.involved_nodes.clone(),
            their_versions: vec![
                conflict.local_head.version,
                conflict.remote_head.version,
            ],
        }).await.ok();

        // 2. 関与ノードがオフラインの場合はホットスタンバイに保存
        for peer in &conflict.involved_nodes {
            if !self.is_peer_online(peer).await {
                self.hot_standby
                    .entry(peer.clone())
                    .or_default()
                    .push(ConflictNotification {
                        conflict: conflict.clone(),
                        notified_at: now(),
                        target_peer: peer.clone(),
                    });
            }
        }
    }

    /// ピアがオンラインに復帰した際にホットスタンバイ情報を配信
    pub async fn flush_hot_standby(&self, peer: &PeerId, gossip: &GossipNode) {
        if let Some((_, notifications)) = self.hot_standby.remove(peer) {
            for notif in notifications {
                // 保持期限内のもののみ送信
                gossip.publish(
                    &TopicId::from(&notif.conflict.file_id),
                    GossipMessage::ConflictAlert {
                        file_id: notif.conflict.file_id.clone(),
                        conflicting_nodes: notif.conflict.involved_nodes.clone(),
                        their_versions: vec![
                            notif.conflict.local_head.version,
                            notif.conflict.remote_head.version,
                        ],
                    },
                ).await.ok();
            }
        }
    }

    /// コンフリクト状態でも更新を許可（ただし通知が飛ぶ）
    pub async fn update_during_conflict(
        &self,
        file_id: &FileId,
        block: ChainBlock,
        gossip: &GossipNode,
        topic: &TopicId,
    ) -> Result<()> {
        // 1. 更新自体は許可（チェーンに追加）
        // 2. コンフリクト状態は維持
        // 3. 関与ノードに「コンフリクト中に更新があった」旨を通知
        gossip.publish(topic, GossipMessage::ConflictAlert {
            file_id: file_id.clone(),
            conflicting_nodes: self.conflicts.get(file_id)
                .map(|c| c.involved_nodes.clone())
                .unwrap_or_default(),
            their_versions: vec![block.version],
        }).await?;
        Ok(())
    }
}
```

### 16.3 コンフリクト解決フロー

```
┌───────────────────────────────────────────────────────────┐
│              Conflict Resolution Flow                       │
│                                                             │
│  コンフリクト検出                                           │
│       │                                                     │
│       ▼                                                     │
│  ① Gossipsub で ConflictAlert を全メッシュに配信            │
│       │                                                     │
│       ├── 対象ノードがオンライン                             │
│       │   → 即座に通知を受信                                │
│       │                                                     │
│       └── 対象ノードがオフライン                             │
│           → ホットスタンバイに保存                           │
│           → ノード復帰時に配信                               │
│       │                                                     │
│       ▼                                                     │
│  ② コンフリクト状態は全ノードに周知される                    │
│     ファイル状態: FileState::Conflict                        │
│       │                                                     │
│       ▼                                                     │
│  ③ コンフリクト中も更新は可能                               │
│     → ただし更新のたびに通知が飛ぶ                          │
│       │                                                     │
│       ▼                                                     │
│  ④ 解決操作（ユーザーが選択）                               │
│     ├── KeepLocal: ローカル版を採用し、チェーンを進める      │
│     ├── AcceptRemote: リモート版を取り込む                   │
│     └── ManualMerge: 手動マージ後に新ブロックとして記録      │
│       │                                                     │
│       ▼                                                     │
│  ⑤ 解決ブロックをチェーンに追加                             │
│     Gossipsub で解決を通知 → 全ノードの状態を Synced に      │
└───────────────────────────────────────────────────────────┘
```
