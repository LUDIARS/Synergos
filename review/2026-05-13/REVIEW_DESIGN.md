# REVIEW_DESIGN — 2026-05-13

評価: **A**

## D-01 [A] 4 レイヤ分離

`net→ipc→core→gui/tauri` + `ars-plugin`。 `NetworkHandles` (`daemon.rs:43`) と `ServiceContext` (`ipc_server.rs:36`) で集約。 git↔SourceTree モデル (DESIGN §2.2) と一致。

## D-02 [A] 拡張 magic stream

`daemon.rs:625-689` で 4 既存 magic 以外を `PeerStreamReceivedEvent` で上位通知、 1 MiB cap。 Tessera/Susurrus の足場。

## D-03 [A-] auto_promote

`promotion.rs:60-86` で IPv6/UPnP/cloudflared/relay 並列 probe。 `force_relay_only` > `auto_promote`。 `daemon.rs:113` `relay_endpoint_configured: false` 固定 TODO。

## D-04 [A] peer-info bootstrap

HTTPS GET → QUIC connect 2 段で Tunnel UDP 不可制約を解決。 `bootstrap_urls` (`config.rs:43`) で起動時 auto-attach、 open 全 project に attach (`daemon.rs:345`)。 TURN/STUN 配線は M-01。
