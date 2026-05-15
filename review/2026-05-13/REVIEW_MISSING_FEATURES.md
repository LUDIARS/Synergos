# REVIEW_MISSING_FEATURES — 2026-05-13

評価: **B**

## M-01 [HIGH] STUN/TURN 未配線

`mesh/stun.rs` (287 行) は単独存在、 `conduit/mod.rs` から未呼出。 `MeshConfig::turn_servers` は受けるだけ。 Tunnel UDP 不可環境で完全 NAT 越え経路が WSS Relay のみに。 推奨: Conduit に組込み。

## M-02 [MEDIUM] `relay_endpoint_configured` 固定 false

`daemon.rs:113`。 `NetConfig::relay_endpoint: Option<String>` 追加で解消。

## M-03 [MEDIUM] CatalogManager 永続化なし

`daemon.rs:230` で in-memory のみ。 daemon 再起動後に余分 sync。 推奨: `catalog/<project_id>.bin`。

## M-04 [MEDIUM] IPC Subscribe フィルタ粗

`EventFilter` は category 単位のみ。 推奨: `TransferOfProject { project_id }` 追加。

## M-05 [LOW] Relay room token (V-05 表裏)

`RelayConfig::require_token` + HMAC-SHA256。

## M-06 [LOW] cloudflared version log

`tunnel/mod.rs:101`。 起動前 `cloudflared --version` を tracing。

## 完成度ヒート

| 機能 | % |
|---|---|
| QUIC S1+HLO1 | 100 |
| HTTPS bootstrap | 95 |
| IPv4 fallback | 100 |
| auto-pull #57 | 95 |
| bootstrap_urls #56 | 100 |
| CatalogSync #25/26 | 90 |
| WSS Relay | 80 |
| Tunnel supervisor | 90 |
| TURN/STUN 利用 | 20 |
| Identity 暗号 #41 | 100 |
| Tauri GUI | 75 |
| ars-plugin | 60 |
| auto_promote | 95 |
