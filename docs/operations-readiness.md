# 運用開始に向けた完成度と残作業 (2026-08-15 棚卸し)

「2 台で動かして、日常的にアセットを揃える」を運用開始の定義として、
コードを読んで判定した現状と残作業。**運用テスト (OPERATIONAL-TEST.md) の前提になる**。

## 1. 現状の完成度 (機能別)

| 領域 | 状態 | 根拠 / 備考 |
|---|---|---|
| ノード identity / QUIC 相互認証 (S1) | ✅ できている | ed25519 → 自己署名証明書、`expected_peer_id` ピンニング |
| 別マシンとの接続 (`peer add-url` / `/peer-info`) | ✅ できている | HTTP GET → QUIC connect、unspecified 告知時は URL ホストに置換 |
| **招待トークンで別マシンから join** | ✅ **現行実装** | 従来は発行 daemon のメモリ内でしか有効でなく、別マシンでは常に `invalid invite token` だった。署名付き自己完結トークン (`syn1.`) に変更 |
| publish → FileOffer → auto-pull → QUIC 転送 | ✅ できている | `two_node_full_e2e` / `auto_pull_e2e` で配線ごと検証済み |
| **同じファイルの 2 回目以降の更新が伝搬** | ✅ **現行実装** | version が常に 1 固定で、受信側が「持っている」と判定して 2 回目以降を取りに行かなかった。`.synergos/manifest.json` で単調増加に |
| **再 publish の Want が gossip 重複として捨てられる** | ✅ **現行実装** | FileWant を常に version 0 で流していたため 2 回目以降が内容同一 = 同じ MessageId になり自ノード/相手で drop されていた。要求バージョンを載せる |
| **daemon 再起動後の継続** | ✅ **現行実装** | Offer 台帳がメモリのみで、publisher 再起動後は FileWant に応答できなかった。マニフェストから復元 |
| **Windows ↔ Linux 混在のパス** | ✅ **現行実装** | FileId が `\` 区切りのまま流れ、Linux 側に `sub\a.bin` という 1 ファイルができていた。`/` に正規化 |
| **受信の一時ファイル** | ✅ **現行実装** | OS temp → プロジェクトへ `rename` していたため別ドライブ (C: temp → D: 作業) で失敗。`<root>/.synergos/incoming/` に変更 |
| **Windows で `[::]:port` に IPv4 が届かない** | ✅ **現行実装** | Windows 既定 v6-only。IPV6_V6ONLY を落としてデュアルスタック bind |
| publish 時に大ファイルを RAM に全読み | ✅ **現行実装** | CRC のために `fs::read` していた → ストリーミング CRC |
| ネットワーク由来 FileId のパス脱出 (`..`) | ✅ **現行実装** | 受信側 resolver で `..` / 絶対パス / `.synergos/` を拒否 |
| 転送の整合性検証 | ✅ できている | 64 KiB フレームごと blake3 + 全体 blake3 |
| CatalogUpdate → CatalogSyncService | ⚠️ 骨格のみ | RootCatalog を Bitswap で取ってくるが**実ファイル同期には使われていない** (実データは FileOffer 経路)。Phase 2 (履歴ノード) では使わない。将来の最適化用に残置 |
| Bitswap / ContentStore | ⚠️ メモリのみ | `MemoryContentStore`。再起動で消える。Phase 2 の履歴ノード保管庫は別実装 (`.synergos/history/`)、ContentStore は据え置き |
| 差分転送 (変わった部分だけ) | ❌ 未実装 | 全体転送。設計は [versioning-design.md](versioning-design.md) §3 |
| 削除 / リネームの伝搬 | ❌ 未実装 | manifest に tombstone を持たせれば可 (P1) |
| ディレクトリ単位 publish / 自動監視 | ❌ 未実装 | `publish <id> <files...>` のみ。`--all` (作業ツリー走査) は小さい追加 (P1) |
| 競合検出・退避 | ⚠️ 一部 | 同じ version で size / CRC が違う Offer は `ConflictManager` へ登録し、ローカル版を保持。親版に基づく分岐検出・別名退避・解決 UI は未実装。設計 §4 |
| Cloudflare Tunnel 経由 | ❌ 方針転換 | UDP 不通。インターネット越しの接続は Cloudflare Mesh または直接到達できる UDP 経路を使う |
| Cloudflare Mesh 実運用 | ⚠️ 未着手 | dashboard 設定・エンロール・到達性確認は人手。実環境向け手順の整備が必要 |
| synergos-relay (WS 中継) | ✅ 単体はある | クライアント側の relay 経路は `force_relay_only` の設定と conduit のみ。2 台運用では不要 |
| TURN | ❌ 未実装 | Mesh で代替 |
| GUI (egui) / Tauri | ⚠️ 表示・基本操作 | 運用は CLI で足りる。invite の `--url` 相当は GUI 未対応 (config 側で `peer_info_advertised_url` を入れれば同じ結果) |
| Ars プラグイン | ⚠️ 薄い | 今回のスコープ外 |

## 2. 残作業 (優先順)

### P0 — 2 台運用の成立に必須

すべて §1 の「現行実装」行。加えて:

- [x] `docs/two-node-operations.md` (2 台手順)
- [x] `docs/versioning-design.md` (VC との兼ね合い)
- [x] E2E テスト `versioned_republish_e2e` (v1 → 再 publish v2 → 据え置き → manifest → 再起動復元)
- [ ] **実機 2 台での通し確認** — コードとテストで裏は取ったが、別マシン間で `invite → join → publish → 受信 → 再 publish → 再受信 → 両 daemon 再起動 → 再 publish` の一連は**まだ実機で回していない**。手順どおりに回して詰まった点を two-node-operations.md §5 に追記する

### P1 — 日常運用で早々に困る

| # | 作業 | 規模 | メモ |
|---|---|---|---|
| 1 | `project publish <id> --all` (作業ツリー走査、`.synergos/` と `.gitignore` 相当の除外) | 小 | 今は毎回ファイル列挙が要る |
| 2 | 削除 / リネームの伝搬 (manifest tombstone + `FileOffer{deleted:true}`) | 中 | 消したファイルが他ノードに残り続ける |
| 3 | `project status <id>` (manifest vs 作業ツリー: 変更・未 publish・未取得) | 小 | 「今 publish すべきものは何か」を人が把握できるように |
| 4 | 受信側の欠落復旧: manifest にあるのに実ファイルが無い/壊れている場合の再取得 (`project verify <id>`) | 小 | 手で消したファイルを再取得できない |
| 5 | Cloudflare Mesh のセットアップ手順と接続診断の整備 | 中 | LAN 外運用の前提。製品外設定を含むため実環境で確認する |
| 6 | 転送の再開 (resume) / 並列度制御 | 中 | 数 GB を切断で最初からやり直す。CHUNK/offset を Offer に載せれば可 |
| 7 | invite の GUI / Tauri 対応 (`peer_info_url` を渡す) | 小 | CLI は済み |

### P2 — 履歴ノードとチーム規模 (versioning-design.md Phase 2〜3、neco 決定 2026-08-16: 差分管理はしない)

| # | 作業 | 規模 |
|---|---|---|
| 1 | `[history]` 設定 (enabled / projects / root / 保持ポリシー) + 保管庫 (objects + index.json、atomic write、meta sidecar) | 中 |
| 2 | 受信/publish 完了時に履歴ノードが objects へ格納 (ハードリンク or コピー) + 旧版 FileWant への応答 | 中 |
| 3 | `project checkout` / `project restore --version` / `history ls` / `history gc` | 中 |
| 4 | 競合検出 (version 分岐) → 退避 + ConflictAlert 発火 | 中 |
| 5 | manifest / state 分離 (git コミット用と node ローカル用) | 小 |
| 6 | プロジェクト参加 ACL (招待トークンを認可境界にする: ホスト側で参加要求を検証) | 中 |
| 7 | 4 台以上でのメッシュ形成 (GRAFT/PRUNE の実配線。今は全 QUIC 接続へ flood) | 中 |
| 8 | Cloudflare Mesh の実セットアップと常駐運用 | 運用作業 |

## 3. 判断が要るところ

1. **2 台の実機はどれか** — 手順書は LAN と Mesh の両方を書いたが、実機確認はどちらの構成でやるか
   (LAN なら今日から、Mesh なら Cloudflare 側の設定が先)
2. ~~versioning-design の案 B で進めてよいか~~ → **neco 決定 (2026-08-16): 案 D 履歴ノードフラグ**。
   残る判断は着手順 (P1 を先に潰すか、P2 #1〜#3 を先にやるか)
