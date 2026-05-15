# Synergos AI Review — 2026-05-13

リポジトリ: E:/Document/Ars/Synergos
対象 commit: 088b31a (peer stream extension hook for upper services)
基準: AIFormat 5 テンプレ (DESIGN / VULNERABILITY / IMPLEMENTATION / MISSING_FEATURES / QUALITY)
レビュー範囲: 5 crate + Tauri GUI (synergos-net / -core / -ipc / -gui / -relay / -tauri / ars-plugin-synergos)

## 全体評価

| 観点 | 評価 | 概要 |
|------|------|------|
| 設計 (DESIGN) | A | 4 レイヤ分離 (net/ipc/core/gui) が明確、 git↔SourceTree モデル、 Ars 非依存の `synergos-core` |
| 脆弱性 (VULNERABILITY) | B | TLS は ed25519 SPKI ピンニング + HLO1 相互認証で S1 解決済。 残課題は `/peer-info` 無認証 + 自動 advertise の IP echo 依存 |
| 実装 (IMPLEMENTATION) | A | quinn + rustls + custom verifier の組み合わせが正しい、 IPv6/IPv4 dual-candidate fallback も健全 |
| 不足機能 (MISSING) | B | TURN/STUN 実利用、 relay ↔ direct hybrid、 catalog の persistence、 IPC subscribe フィルタ細分化が未着手 |
| 品質 (QUALITY) | A | テスト 60+、 CI 3 OS matrix、 unsafe は libc/Win32 SID 取得に限定、 `RUSTFLAGS=-D warnings` |

## 集計

- 件数: design 4 / vulnerability 5 / implementation 4 / missing 6 / quality 3 = 計 22
- 重大度 high: 2 件 (`/peer-info` 無認証公開、 IPv4 echo MITM ありえる)
- 重大度 medium: 7 件
- 重大度 low: 13 件
- 自動修正候補: 0 件 (列挙のみ、 SCM 修正禁止ルール準拠)

## weighted_score

- design 0.30 × A(95) + vuln 0.30 × B(80) + impl 0.20 × A(92) + missing 0.10 × B(78) + quality 0.10 × A(90)
- = 28.5 + 24.0 + 18.4 + 7.8 + 9.0 = **87.7 / 100**

## 主要所見

1. **QUIC TLS は健全。 ed25519 SPKI 直接ピン留め + HLO1 双方向 Hello で S1 解決**
   - `synergos-net/src/quic/verifier.rs:38` `check_peer_binding` で SPKI を抜いて `blake3[:20]` 照合
   - `synergos-net/src/quic/hello.rs:108` クライアント側自己署名 Hello、 サーバ側は accept 直後の `recv_hello` で同じ検証
   - 緩和パス (旧 `DevOnlySkipVerify`) は完全削除済 (`synergos-net/src/quic/mod.rs:19` コメントで明記)
2. **`/peer-info` servlet は無認証で公開する設計** (`synergos-core/src/peer_info_server.rs:200`)
   - `peer_id` + `quic_endpoint` + `synergos_version` を匿名 GET で返す
   - 設計コメントは「将来 Tower middleware で認証層を後付け」と書くが現状は空欄
   - peer_id と endpoint だけで偽サーバを spoof する経路は無いが (TLS で弾く) 、 列挙 + バージョン情報リーク (CVE 紐付け) のリスクあり
3. **`auto_advertise_addr` が HTTPS IP echo に依存** (`peer_info_server.rs:107-132`)
   - `icanhazip.com` / `ipify.org` / `ident.me` の HTTPS 応答を信用して advertise する
   - TLS で integrity は守られるが、 これらの ECHO サービスは攻撃面を上流に拡大 (DNS hijack で偽 IP を返されると拡散する)
4. **Cloudflare Tunnel UDP 不通問題への対処は ハイブリッド + relay-only auto promotion で完結**
   - `synergos-net/src/promotion.rs` で起動時 probe → IPv6 / UPnP / cloudflared / relay の組合せで `RelayOnly` を自動判定
   - bootstrap は HTTPS (`/peer-info`)、 データプレーンは直結 QUIC、 これで `feedback_cloudflare_tunnel_udp.md` の制約を吸収
5. **IPv4 fallback は端正な実装** (`synergos-core/src/peer_bootstrap.rs:180-239`)
   - advertised が specific なら primary に、 異ファミリの DNS 解決を末尾に積む
   - `[::]` advertise は DNS 解決結果を v6 → v4 の順に並べる
   - `feedback_synergos_windows_dualstack_quic.md` の制約 (Win dual-stack で v4 dest を取りこぼす) を緩和

詳細は各 REVIEW_* ファイルを参照。
