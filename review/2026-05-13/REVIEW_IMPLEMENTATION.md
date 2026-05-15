# REVIEW_IMPLEMENTATION — 2026-05-13

評価: **A**

## I-01 [A] QUIC + rustls custom verifier

`quic/mod.rs:458-505`。 rustls 0.23 `with_custom_certificate_verifier` を connect 毎構築、 ED25519 のみ許可 (`verifier.rs:110`)。 `Once::call_once` で CryptoProvider 二重 install を遮断。

## I-02 [A] HLO1 双方向認識

`quic/hello.rs:108-173`。 connect 直後 bidi で SignedHello、 accept 直後 4096B 上限+ed25519 検証+`blake3[:20]==peer_id` 再導出、 失敗即 close。

## I-03 [A-] IPv4 dual-stack fallback

`peer_bootstrap.rs:180-239`。 advertised specific → primary、 DNS 異ファミリ末尾。 unspecified → DNS を prefer family で並べ替え。 unit+E2E (`:274`)。 Win+dual-stack 問題を吸収。

## I-04 [A] Daemon 並行性

`daemon.rs:62-431`。 7 並行タスク+任意 peer_info+bootstrap、 abort handle 全保持。 shutdown は project_close→presence→transfer cancel→conduit/quic/tunnel 順。

## 良い点

IPC 段階 read (`transport.rs:131`)、 gossip GC O(n log k) (`gossip/node.rs:42`)、 transfer 二段 hash 検証、 Win SID RAII (`ipc_server.rs:310`)、 unsafe は OS API のみ。

## 軽微

`peer_bootstrap.rs:228` `.copied().copied()` 冗長、 `daemon.rs:113` TODO。
