# 再設計プロジェクト 引き継ぎ資料 (Synergos / Curare)

- 作成日: 2026-10-23
- 対象: Synergos main @ eeeb650 / Curare main @ f6d9153
- 位置づけ: FORMAT_SPEC §6 の作業ドキュメント。後任 (人 / AI セッション) がこの文書だけで作業を再開できることを目的とする。
- 同内容の文書: Curare 側 `spec/plan/redesign-analysis/HANDOFF.md` (repo 固有節だけ異なる)。

---

## 0. 結論 (先に)

- **依頼**: Synergos + Curare を「AI 開発・ゲーム開発の課題解決 / セキュリティ (脆弱性と侵害の歯止め) / 転送速度と効率」の 3 条件を同時に満たす仕組みへ再設計するため、足りない要素と再設計点を洗い出し、テーマ別の設計書を各ブランチにコミットする。侵害封じ込め機構は特に深く調査する。追加指示として、実装可能な部分は並列で実装しレビューする。
- **到達点**: 解析 (コード読解マップ 13 本・外部調査 4 本) は完了。発見の洗い出しは 15 観点中 6 観点で生データまで。反証・重要度判定、設計書 6 本、実装 3 件は**未着手**。
- **停止理由**: 2026-10-16 08:55 JST の停止指示と、同時刻に API 週次上限に達して並列エージェントが全停止したため。
- **次の一手**: §5 の順に、発見の残りと反証 → 設計書執筆 → 実装。C++ 移行計画 (`spec/plan/cpp-migration.md`) と衝突しないよう、設計書は「Rust 現行実装の欠落」と「C++ 移行後も残る契約上の欠落」を分けて書く (§6)。
- **判断結果 (2026-10-23)**: §7 の 6 点は §7.1 で確定。実装 3 件は Rust へ入れず C++ のリスペックとして作り直す。

---

## 1. 現在地

| 項目 | 状態 | 場所 |
|---|---|---|
| CI 復旧 (rustfmt / clippy / macOS・Windows テスト / RustSec) | **main に取り込み済み** (Revisor 経由、eeeb650)。GitHub PR #71 は同内容で未クローズ | PR #71 (`fix/ci-green`, head af6a9f2) |
| 解析スナップショット | **main に取り込み済み** (eeeb650 でパス正規化と find-verify の判定修正つき)。GitHub PR #72 は未クローズ | `spec/plan/redesign-analysis/` |
| Curare 側解析スナップショット | GitHub PR 未マージ (チェック全緑) | Curare PR #12 (`design/analysis-2026-10-15`) |
| Security audit の残り 1 件 | 未解決 (判断待ち、§7-5) | PR #71 のコメント |
| 発見 (findings) | 6/15 観点の生データ。**未検証** | `findings/*.json` |
| 設計書 6 本 | 未着手 | 予定: 各 repo `spec/plan/redesign-{security,transfer-efficiency,ai-game-dev}.md` |
| 実装 3 件 | 未着手 (仕様は §4) | — |
| 残作業報告 (人間向けの要約ドキュメント) | 作成済み | https://claude.ai/code/artifact/6a009053-a2b9-4b11-b86c-569762171160 |

注意: PR #71 / #72 の内容は Revisor が squash して main に入れたため、GitHub 上の 2 PR は役目を終えている。クローズするかはオーナー判断 (§7-2)。

---

## 2. 成果物の所在 (`spec/plan/redesign-analysis/`)

| パス | 内容 | 信頼度 |
|---|---|---|
| `maps/sy-*.md` (7 本) | Synergos 読解マップ: transport / discovery / content / core-versioning / history・hooks・IPC / control・custody / clients・docs。各ファイルに 実装済み統制・弱点・性能特性・設定既定値・テスト範囲・文書と実装の差 (file:line 付き) | 高。ただし **51ba2c6 時点**。07af492 で `synergos-net/src/quic/mod.rs` 等が変わったので該当行は再確認 |
| `research/breach-containment.md` | 侵害封じ込め機構の調査。NIST SP 800-207、Biscuit / CAEP、短寿命資格、S3 Object Lock / tombstone、エンベロープ暗号と crypto-shredding、Sigstore / 改ざん検知ログ、C2PA、SLSA / GitHub Actions 堅牢化、サンドボックス、ASVS 5.0 V5、Maya scriptNode ワーム、実事例 12 件、**P0〜P2 の機構カタログ** (§8) | 高 (出典 URL 付き。取得できなかった出典は §9 に明記) |
| `research/transfer-efficiency.md` | CDC / delta / 圧縮 / QUIC 調整 / 再開 / P2P / 遅延取得、ワークロード別の判断表 | 高 |
| `research/vcs-game-ai.md` | Perforce・Unity VCS・Git LFS・Xet・lakeFS・DVC 等の機構と、取り入れる / 真似しない点 | 高 |
| `research/ai-dev-data.md` | データセット / モデルの版管理・系譜・ライセンス・GPU アクセス、作らずに統合すべきもの | 高 |
| `findings/*.json` (6 本) | security (authz-boundaries, transport-crypto) / transfer (wire-path, dedup-delta-resume) / ai-game (gamedev-workflow, ai-dataset-model) の生データ。id・重大度・根拠・シナリオ・再設計案 | **未検証** (反証・重要度判定前)。設計書へ転記前に file:line を再確認 |
| `workflows/find-verify.js` | 発見 → 反証 → 重要度判定。テーマ単位 (`args.theme`) | main で判定修正済み (`tests/find-verify.test.mjs`) |
| `workflows/design.js` | 3 観点のアーキテクト → 統合ブリーフ (契約 ID 台帳つき) → repo 別の設計書執筆 → 批評 2 系統 → 改訂 | 未実行 |
| `workflows/implement.js` | 実装 (worktree 単位) → 独立検証 → 修正 → 再検証 | 未実行 |
| `workflows/TEMPLATE.md` | 設計書の章立て (結論 / 背景・ゴール・非ゴール / 現状分析 / 4 軸比較 / 再設計 / 実装ステップ・ロールバック / 検証 / 未決) | 確定 |

---

## 3. 暫定の主要発見 (critical / high、未検証)

反証前なので「候補」として扱う。id は `findings/*.json` と対応する。

### セキュリティ (侵害封じ込め)

| id | 要旨 | repo |
|---|---|---|
| SEC-CR-01 / SEC-AZ-02 | HLO1 Hello が TLS セッションに束縛されず ts_ms も未検査で、再生によって他ノードになりすませる。accept() が既存接続を上書きする | Sy |
| SEC-AZ-01 | データプレーンにプロジェクト会員資格の検査が一切ない。鍵を持ち到達できれば全プロジェクトの会員扱い | Sy |
| SEC-AZ-04 / AZ-05 / AZ-06 | gossip・DHT・bitswap が署名はあっても認可がない (任意 CID を誰にでも返す等) | Sy |
| SEC-AZ-03 / CR-03 | 招待トークンが無期限・多回使用・失効不可の bearer | Sy |
| SEC-AZ-07 / CR-02 | bootstrap の信頼アンカーが平文 HTTP の TOFU | Sy |
| SEC-AZ-08 / CR-14 | relay は room_id だけが秘密の平文 WS、送信者詐称可 | Sy |
| SEC-AZ-09 / AZ-10 | IPC は同一 uid なら全権 (資格取得失敗時に fail-open)、hook は peer が上書きできる repo 相対スクリプトを全環境変数つきで実行 | Sy |
| SEC-AZ-11 / AZ-12 | control は全組織共通の静的 admin bearer。ノード登録簿はデータプレーンに反映されず revoke が届かない | Sy / Cu |
| SEC-AZ-13 / CR-04 | custody は 1 つの秘密で bearer 認証・受領証 HMAC・capability URL 署名を兼ね、検証側 (Curare) も同じ秘密を持つため受領証を偽造できる | 両方 |
| SEC-AZ-17 | 失効が伝播しない (発行済み capability、24h admission、peer セッション、relay、custody) | 両方 |
| SEC-CR-06 / CR-09 | 平文経路の許容、鍵ローテーション / 失効の仕組みが皆無 | 両方 |
| SEC-AZ-14 / AZ-15 | Curare: リクエスト毎に Cernere WS を張りキャッシュ・レート制限なし、接続断を 401 扱い。作成者が永久に全権で職務分離なし | Cu |

### 転送速度と効率

| id | 要旨 | repo |
|---|---|---|
| TRN-WP-01 / WP-02 | quinn が既定値 (ストリーム窓 1.25 MB で 100 ms RTT なら約 100 Mbps 上限)、keepalive なしで 30 秒無通信で切断 | Sy |
| TRN-WP-03 / WP-06 / WP-07 | 1 ファイル 1 ストリーム直列、送信側で 3〜6 回読み、受信側で 2 回書き 4 回読み (100 GB で 600 GB の I/O) | Sy |
| TRN-WP-04 / DD-03 / DD-09 / WP-15 | 再開・Range・複数ソースなし。custody は 30 分上限の単発 PUT | Sy |
| TRN-DD-01 / WP-12 / DD-07 | 2026-08-16 の「全体転送・CDC なし」決定で反復保存の大容量ファイルは重複排除 0%。AI チェックポイントで 50〜60% の削減余地 | 両方 |
| TRN-DD-05 | 内容同一性が CRC32 + サイズ。転送時に計算済みの BLAKE3 を捨てている | Sy |
| TRN-DD-10 / DD-12 / DD-13 | custody verify が毎回全体再ハッシュ、Curare upload は単発 presigned PUT、download は Range 再開なし | 両方 |
| TRN-DD-16 / DD-19 / WP-16 | 10^4〜10^6 ファイルの木で 1 ファイル毎のオーバーヘッドが支配的。100 GB チェックポイントを通せる経路がない。NAT 越えのデータ経路なし | 両方 |

### AI 開発・ゲーム開発

| id | 要旨 | repo |
|---|---|---|
| AGD-GW-01 | 非マージ可能バイナリの排他ロックがない | 両方 |
| AGD-GW-02 | auto-pull が未 publish のローカル編集を上書きする | Sy |
| AGD-GW-04 | Synergos manifest / 履歴ノードと Curare の版が ID を共有しない 2 系統 | 両方 |
| AGD-GW-06 / GW-07 / GW-10 | 部分ワークスペース、エンジン / DCC 統合、オフライン復帰がない | 両方 |
| AGD-GW-17 / AI-02 / AI-15 | 500 / 500 / 1000 の固定上限、manifest 全体書き直しなどスタジオ規模に届かない | 両方 |
| AGD-AI-01 / AI-03 / AI-04 | データセット集約がない、`reproducible` は拒否のみでキャッシュではない、系譜が平坦 | Cu |
| AGD-AI-05 / AI-06 / AI-11 | 権利・同意・AI 利用可否の記録なし、消去不能、取り込み時の検査・隔離なし | Cu |
| AGD-AI-07 / AI-08 | チェックポイント級の大容量階層と学習クラスタ向けアクセス面がない | 両方 |

---

## 4. 実装 3 件の仕様 (未着手、そのまま着手可能)

いずれも 2026-10-15 に worktree を作ったが、API 上限でコミット 0 のまま停止した。**C++ 移行 (§6) との関係を先に決めること** (Rust 現行実装に入れるか、C++ 側の契約として取り込むか)。

### 4.1 HLO2: Hello の TLS セッション束縛 (Synergos, `synergos-net/src/quic/`)

対象: SEC-CR-01 / SEC-AZ-02 (critical)。07af492 で `quic/mod.rs` が変わっているので着手前に再読すること。

- 署名対象に `quinn::Connection::export_keying_material(&mut [u8; 32], b"synergos-hlo2 v1", &[])` (両側で独立に計算) を含め、他接続で捕獲した Hello が検証を通らないようにする。`ts_ms` は ±5 分の鮮度窓 (定数 + doc コメント)。
- マジックを `HLO2` にし、HLO1 は明示エラーで拒否 (前例: 版番号なし転送の拒否)。互換性破壊なので全ノード同時更新を docs に明記。
- accept をタスク化し、Hello 待ちで accept ループが止まる HoL ブロックを解消。保留ハンドシェイク上限 (既定 64、`QuicConfig` で設定可、超過は warn して close)、`Incoming` のアドレス検証 (retry) を使う。
- 同一 PeerId の再接続: 旧接続が閉じていれば置換、生きていれば置換したうえで旧接続を専用 close code で明示的に閉じ、warn ログ (PeerId::short と両アドレス)。閉じた接続を `open_stream` / `raw_connection` が選ばないこと。
- テスト: 往復成功 / 他セッションの Hello 再生を拒否 / 古い ts_ms 拒否 / HLO1 拒否 / Hello を送らない客が他客の accept を 1 秒以上止めない / PeerId 重複で旧接続が閉じる / 上限超過の拒否。
- 文書: DESIGN.md §3.5 と HLO1 に言及する docs を HLO2 に更新。

### 4.2 custody 強化 (Synergos, `synergos-custody/`)

対象: SEC-AZ-13 / SEC-CR-05 の一部、control-custody マップの W2・W3・W12〜W15・W25・W29。Curare は URL を不透明文字列として扱うので互換 (要再確認: `curare-server/src/storage/synergos-storage.ts`)。

- capability URL に 128 bit nonce `n=` を加えて署名対象に含め、単回使用化 (PUT は公開成功で消費し、中断時は同じ URL で再試行可。GET の扱いは spec に明記)。消費済み nonce は期限付きで保持。
- reclaim: 未受付 ID は 404 で墓標を作らない。`.data` は同一ボリュームの trash へ atomic rename し、保持期間 (env `SYNERGOS_CUSTODY_RECLAIM_RETENTION_SECS`、既定 7 日) 後に背景タスクが消去。墓標による再受付拒否は維持。
- 監査イベント (admit / verify / download 発行 / PUT / GET / reclaim / purge): operation_id、object、byte_length、outcome、principal (admin | capability)、可能なら remote_addr。秘密・署名は出さない。synergos-control の既定ログフィルタに `synergos_custody=info` を追加。
- IP 別トークンバケット (admin 600/分、transfer 120/分、env で変更可) → 429 + Retry-After。同時 PUT 上限 32、総 in-flight 64 GiB → 503 + Retry-After。
- `GET /v1/custody/health` を無認証の最小契約 (`ok, service, version, durability`、I/O なし、RULE_SRE §2) にし、詳細は認証付きの別 endpoint へ。
- 期限切れで未アップロードの admission メタデータを同じ保持期間で GC。
- テストと `spec/feature/opaque-custody.md` 更新。

### 4.3 Curare 認証強化 (`curare-server/src/auth/` ほか)

対象: SEC-AZ-14、cu-auth-access マップの性能・可用性指摘。

- actor キャッシュ (SHA-256(token) キー、TTL 60 秒、拒否の負キャッシュ 15 秒、LRU 10,000、single-flight、`invalidateUser`)。生トークンは保持しない。管理系の member.list 再検証はキャッシュしない。
- Bearer の事前検査 (JWT 形式 3 セグメント、長さ 32〜4096)。不正なら Cernere に接続せず 401。
- IP 別レート制限 (照会 60/分、認証失敗 20/分) と Cernere への circuit breaker (連続 5 失敗で 10 秒 fail-fast)。`X-Forwarded-For` は `CURARE_TRUST_PROXY=1` のときだけ信頼。
- Cernere WS の close 理由の分類: 資格エラー → 401、通信断・1006/1011/1012・タイムアウト → 503、応答不正 → 503。Cernere のソース (`LUDIARS/Cernere` の `server/src`) で実際の close code を確認して根拠を書く。
- production では `CERNERE_URL` を https 必須にして起動時 fail-fast。
- `/api/health` を I/O なしの `{ok, service: "curare", version}` にし、依存確認は `/api/readiness` へ分離 (エラー文言は返さない)。version は `EXCUBITOR_SERVICE_VERSION → CURARE_VERSION → npm_package_version → 'unknown'`。
- `spec/feature/auth-session-cache.md` 新設、`.env.example` 追記、`spec/domains/asset-access.domain.json` に新規ファイル (テスト含む) を登録。

---

## 5. 再開手順

### 5.1 環境

- Curare のビルドには `LUDIARS/Cernere` を Curare と同階層に clone し、`packages/id-cache` で `npm install && npm run build` (CI と同じ配置)。
- Rust は CI の stable と同じ toolchain で整形・clippy を確認する (2026-10-16 時点で rustc 1.99.1。ローカルの既定 toolchain が古いと rustfmt / clippy の差分を見落とす)。
- `CARGO_TARGET_DIR` を共有すると 26 GB 程度まで膨らむ。ディスク枯渇時は `debug/incremental` を消すか `CARGO_INCREMENTAL=0`。
- `synergos-tauri` は GTK 開発ライブラリがない環境ではビルド不可なので `--exclude synergos-tauri`。admin-ui は別 workspace で `wasm32-unknown-unknown` ターゲットが要る。

### 5.2 作業順

1. **発見の残り 9 観点**: `workflows/find-verify.js` を `args: {"theme": "security"}` / `"transfer"` / `"ai-game"` で実行。残り観点は security: integrity-dataloss / blast-radius / supply-chain-config / dos-abuse-injection / detection-response-ops、transfer: topology-relay / e2e-flow-metadata、ai-game: data-model-fit / ops-platform-scale。既存 6 観点の JSON は再利用し、スクリプト冒頭で読み込んでスキップするよう改修すると費用を抑えられる。全観点を反証・判定に通し、`findings/<theme>-verified.json` を得る。
2. **設計書**: `workflows/design.js` を `args: {"theme": ..., "sySha": "<main>", "cuSha": "<main>"}` でテーマ毎に実行。出力 2 本を各 repo の `spec/plan/redesign-<theme>.md` に置く。章立ては `workflows/TEMPLATE.md`、選択肢比較は HARNESS §3.5 の 4 軸 (AI 学習量 / 作業コスト / 解決度 / 主目的との一致度)。repo 間の契約は `SPEC-SY-*` / `SPEC-CU-*` / `SPEC-CR-*` の同一 ID を両文書で使う。
3. **実装**: §4 の 3 件を `workflows/implement.js` で (タスク定義に §4 の本文をそのまま渡せる)。各タスクは独立 worktree、テスト同梱、独立検証を通してからテーマブランチへ統合。
4. **レビュー → 統合**: RULE.md §6.5 に従い Revisor 経由 (§7-2)。

### 5.3 並列実行の注意

2026-10-15 は 3 テーマ × 15 観点 + 実装 3 件を同時に走らせて API 週次上限に当たり、全エージェントが途中で止まった。テーマを 1 つずつ流すか、上限回復直後に起動する。

---

## 6. C++ 移行計画との関係

`spec/plan/cpp-migration.md` (2026-10-17) により、Synergos は C++20 への段階移行を始めている (第 1 段は IPC フレーミング・版メタデータ・受領証署名・排他的公開のプリミティブ、`spec/feature/native-contracts.md`)。同計画は「既存の互換契約を保つ」ことを完了条件にしているが、§3 のセキュリティ発見の多くは**契約そのものの欠落** (HLO1 の再生可能性、会員資格検査の不在、custody の秘密兼用、失効の非伝播) である。

設計書では次を分けて書くこと:

- **契約の変更が必要なもの** (例: HLO1 → HLO2、capability URL の nonce、受領証の非対称署名化、会員資格の認可): C++ 移行の互換対象を更新する必要があるので、cpp-migration.md の「Compatibility contracts」へ反映する変更提案として書く。
- **実装の非効率** (例: quinn 既定値、多重読み書き): C++ 側では最初から避けられるので、ネイティブ実装の要件として渡す。
- Rust 現行実装へ先に入れるかどうかは判断事項 (§7-6)。

---

## 7. 判断待ち事項

1. **ブランチと PR の構成**: 3 テーマ × 2 repo = 6 本か、repo ごとに 1 本へまとめるか。
2. **GitHub PR か Revisor か**: RULE.md §6.5 は GitHub PR を禁じ Revisor 経由を求める。実際に PR #71 / #72 は Revisor が main へ取り込んだ。今後は Revisor に統一し、#71 / #72 / Curare #12 は役目を終えたらクローズするか。
3. **2026-08-16 の CDC 取り下げ決定の再検討**: ゲーム用メディアは現決定を維持し、AI チェックポイント / データセットだけチャンク化 (Xet 型) を再検討する案を設計書に載せてよいか。判断者: neco。
4. **Cernere への依頼**: 短寿命 capability token (Biscuit 等) の発行鍵公開、SSF/CAEP によるセッション失効通知、step-up 認証。侵害封じ込めの P0 機構の多くはこれが前提。
5. **Security audit の残り 1 件** (RUSTSEC-2026-0194 / 0195、quick-xml 0.30.0): synergos-gui → eframe 0.31 → accesskit → atspi → zbus_xml 4.0.0 経由で、上流に修正版がない。(a) `.cargo/audit.toml` で理由付き除外、(b) 赤のまま許容。判断者: セキュリティ担当。
6. **実装の着地点**: §4 の 3 件を Rust 現行実装に入れるか、C++ 移行の契約として取り込むか。優先順の提案は HLO2 → custody → Curare 認証。

### 7.1 判断結果 (2026-10-23、判断者 neco)

| # | 決定 | 作業への反映 |
|---|---|---|
| 1 | **6 本** (3 テーマ × 2 repo) | 設計書は `design/<theme>` ブランチを repo ごとに切り、テーマ単位で Revisor へ出す |
| 2 | **Revisor に統一** | GitHub PR #71 / #72 / #73 と Curare #12 はクローズ。資料は Revisor 経由で取り込む (本資料もその経路) |
| 3 | **CDC を再検討する**。ただし別モジュールとして切り離す判断もありうる | transfer-efficiency 設計書で「Synergos 本体に入れる案」と「チャンク保管を独立モジュールにする案」を 4 軸で比較し、切り離しが正解なら境界と契約を書く |
| 4 | **Cernere へ依頼する。経緯のログも残す** | 短寿命 capability token の発行鍵公開・SSF/CAEP 失効通知・step-up 認証を Cernere のタスクとして登録し、依頼内容と根拠 (本資料 §3 の SEC-AZ-17 等) を記録する |
| 5 | **quick-xml 0.30 は「依存自体が必要か」から再検討** | 除外 / 許容の二択ではなく、synergos-gui (eframe → accesskit → atspi → zbus_xml) の依存経路が要るかを先に判断する。C++ 移行で GUI が置き換わるなら依存ごと外す |
| 6 | **C++ でリスペックして作り直す** | §4 の 3 件は Rust 現行実装へ入れない。HLO2 / custody 強化 / 認証強化は C++ 移行の契約 (`spec/plan/cpp-migration.md` の Compatibility contracts、`spec/feature/native-contracts.md`) として仕様を書き直し、C++ 側で実装する。§6 の「契約の変更」と「実装の非効率」の区別はそのまま設計書に使う |

---

## 8. 参考

- 残作業報告 (人間向け要約): https://claude.ai/code/artifact/6a009053-a2b9-4b11-b86c-569762171160
- GitHub: LUDIARS/Synergos PR #71 (CI 復旧、main 取り込み済み)、#72 (解析、main 取り込み済み)、LUDIARS/Curare PR #12 (解析)
- 組織ルール: AIFormat の RULE.md / HARNESS.md / FORMAT_SPEC.md / RULE_SRE.md / RULE_DATA_SCHEMA.md

### 7.2 追加判断 (2026-10-23 夕、判断者 neco)

| # | 決定 | 作業への反映 |
|---|---|---|
| 7 | **外向きのセキュアな中継経路 (DERP 型 relay、両端が outbound で 443 に接続) は C++ 版で作る** | SPEC-SY-relay-auth / SPEC-SY-relay-v2 は C++ 移行の契約として実装する。現行 Rust には入れない。それまで拠点⇔外部環境は Tailscale (synergos-node を tailnet IP にだけ bind) でつなぐ |
| 8 | **名簿と会員は Cernere が担保する** | プロジェクト会員 (誰がどのプロジェクトで read/publish/history か) とノード名簿 (どのノードが誰のものか・どこにいるか) の正本は Cernere。synergos-control は Cernere が署名した会員証・名簿を配布・キャッシュ・検証するだけで、独自に会員を決めない。membership LCT の発行鍵と名簿の署名鍵は SPEC-CR-* 契約として Cernere に置く |
| 9 | **Curare はデザイナー向けの版管理で Synergos のハコ** | 転送・同期の本体は Synergos 単体。Curare 経由は版・承認が要る場面だけ |
| 10 | **Curare を使わないプロジェクトの正式な最新版は、Cernere が publish 役割と認めた会員の署名で決める** | D-A02 (AI・ゲーム開発 設計書) の再判断。Curare 紐付けプロジェクトは版・レビュー・承認を Curare の ref 権威が担い、P2P だけのプロジェクトは Cernere 会員 (SPEC-CR-membership-issuer) が署名した head (SPEC-SY-replication-stream の member-signed head) で同期する。オブジェクトモデルと changeset は共通で、`acceptor_sig` は Curare 紐付けのときだけ要る |
