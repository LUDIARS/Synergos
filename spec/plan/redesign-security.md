# Synergos 再設計 — セキュリティ (脆弱性と侵害の歯止め)

- 作成日: 2026-10-23
- 対象: Synergos main @ d404f20
- 関連テーマ: 本書は 3 テーマのうち security (脆弱性と侵害の歯止め) の Synergos 側正本。同 repo の他テーマは `spec/plan/redesign-transfer-efficiency.md` (転送速度と効率。チャンク保管モジュールの境界 SPEC-SY-chunk-store を持つ) と `spec/plan/redesign-ai-game-dev.md` (AI 開発・ゲーム開発のワークフロー。`project_ref` と Curare space の対応を持つ)。相手 repo の同名文書は「Curare: spec/plan/redesign-security.md (LUDIARS/Curare main)」。repo をまたぐ参照は相対パスを使わず、この「repo 名: repo 相対パス」の形にそろえる (Curare 文書にも同じ書き方を申し送る)。契約 ID・機構 ID・目標番号 (G / E)・共通の問い (Q1〜Q10) は両文書で同一。
- 根拠資料: 本 repo の `docs/versioning-design.md`、`spec/feature/opaque-custody.md`、`spec/feature/native-contracts.md`、`spec/plan/cpp-migration.md`、`docs/hooks.md`、`docs/mesh-operations.md`、`docs/mesh-two-node-checklist.md`、`docs/site-operations.md`。解析成果 (すべて repo 相対): `spec/plan/redesign-analysis/HANDOFF.md` (§6 は「契約の変更」と「実装の非効率」の区別、§7.1 は 2026-10-23 のオーナー決定 6 点。決定 6 により本書の Synergos 実行時変更はすべて C++ 移行の互換契約 / ネイティブ要件として書き、現行 Rust 実装へは入れない)、検証済み発見 `spec/plan/redesign-analysis/findings/security-verified.json` (kept[] 136 件のうち Synergos 関係 117 件)・`spec/plan/redesign-analysis/findings/transfer-verified.json` (kept[] 79 件)・`spec/plan/redesign-analysis/findings/ai-game-verified.json` (kept[] 77 件)、読解マップ `spec/plan/redesign-analysis/maps/sy-*.md`、調査ノート `spec/plan/redesign-analysis/research/breach-containment.md`・`spec/plan/redesign-analysis/research/transfer-efficiency.md`。検証済み発見 3 本は d404f20 の main に無い (main の `findings/` は未検証の生データ 6 本だけ) ので、本書と同じ変更で `spec/plan/redesign-analysis/findings/` に追加する。相手 repo の決定として Curare: `spec/feature/codex-cloud-access.md` (Curare ローカル main 744f013、2026-10-23 neco 判断) を参照する。外部資料は §8。

## 0. 結論 (先に)

- **足りないもの**: Synergos の資格はすべて大域・静的・共有・失効不能で、データプレーンには認可が存在しない。ed25519 鍵を持ち到達できれば全プロジェクトの会員になる (SEC-AZ-01, SEC-BR-01)。control は全組織共通の静的 bearer 1 本 (SEC-AZ-11, SEC-BR-04)。custody は 1 つの対称秘密で認証・URL 署名・受領証署名を兼ね、検証側の Curare も同じ値を持つ (SEC-CR-04, SEC-BR-05)。破壊的変更は旧バイトを残さず全メンバーへ伝播する (SEC-IN-01)。失効・不変コピー・流出検知・マージ前の供給網検査がどれも無い (SEC-BR-12, SEC-DR-09, SEC-DR-04, SEC-SC-01)。
- **作り直すもの**: (1) すべての境界を PEP にし、1 系統の短寿命・範囲限定・署名付きトークン LCT v1 で検証する。(2) 侵害前提の封じ込め単位 (プロジェクト / サービス主体 + 用途 / 操作 ID / プロジェクト KEK) を置き、署名付き失効バンドルで束ねて全 PEP を fail-closed にする。(3) Revisor のマージ前ゲート・濫用方針表・イベント網羅表・起動時 fail-fast で基線を機械検査し、後退を止める。
- **着地点**: HLO2・custody 強化・認証強化は Rust に入れない。`cpp-migration.md` の "Compatibility contracts" 改訂と `native-contracts.md` の新 `[id:]` 節として仕様化し、C++ 側で実装する (§4.37)。Rust 側は設定だけの暫定補償 (SECURITY-M34) に留める。
- **順序**: P0 文書・ゲート・暫定設定 → P1 共通 codec と golden vector → P2 Curare 実行時 (Curare 文書) → P3 C++ custody / control → P4 C++ peer データプレーン → P5 データ保護 → P6 検知と配布完全性。
- **会員資格と名簿の権威 (オーナー決定 8、2026-10-23 夕)**: プロジェクト会員とノード名簿の正本は Cernere で、membership LCT・node card・ProjectRoster・RosterStatus・OfflineLease は Cernere だけが署名する (SPEC-CR-membership-issuer、SPEC-CR-node-roster、§4.34)。synergos-control は配布・キャッシュ・検証だけを行い、会員・所有・到達性を決めず、その発行鍵をどのフェーズでも (暫定でも) 持たない。enrollment の承認は Cernere (step-up)、失効は Cernere から SSF/CAEP で流れる。
- **relay の着地 (オーナー決定 7)**: 外向きの DERP 型 relay は C++ 版でだけ作る。C++ relay が受入ゲートを通るまで、拠点と外部環境は Tailscale でつなぎ、`synergos-node` は tailnet IP にだけ bind する (§4.6、SECURITY-M34)。Curare はデザイナー向けの版管理で Synergos のハコであり、バイトの経路でも会員資格の権威でもない (オーナー決定 9)。
- **他テーマとの整合**: 転送効率文書が本書の契約に求めた追補 X1〜X5 (relay の QUIC 経路、node card の `topo`、upload_id 単位の PUT、新 stream 種別への会員資格ゲート、SPEC-SY-chunk-access の無条件化) は本書で受け入れて契約文に反映した (§4.0.2)。
- **現状のまま運用した場合の最悪シナリオ**: フィッシングされたアーティスト機 1 台が、ランサムで暗号化したファイルと `.git/hooks` ペイロードを全メンバーの作業ツリーへ送り込む (SEC-IN-01, SEC-DA-05, SEC-BR-01)。同時に Curare ホストから読んだ受領証秘密で、全組織の custody 実体の GET URL をオフライン・無記録で作る (SEC-BR-05, SEC-DR-04)。不変コピーが無いので復旧できない (SEC-DR-09)。

## 1. 背景・ゴール・非ゴール

### 1.1 背景

現状設計の要点 (既存 spec の引用):

- `docs/versioning-design.md:10-11`: 「差分管理はしない」「転送は Phase 1 のまま全体転送 (blake3 検証) … CDC チャンク重複排除案は neco 決定 (2026-08-16) で取り下げ」。履歴の実体は「フラグを立てた履歴ノード (history node) が版ごとに丸ごと保持」し、既定は無効 (`docs/versioning-design.md:111`)。
- `spec/feature/opaque-custody.md:3-5`: 「Cu Serverが版・提出・許可・共有ノードの利用方針、Crが本人・所属・組織役割を所有する。Synergosはopaque objectの転送と保管の実行を担当し、制作履歴や承認を作らない。」(2026-09-08 境界)
- 2026-10-23 夕の決定 8・9 で、この境界を次のように締める。Cernere は本人・組織・所属に加えて、**Synergos のプロジェクト会員 (誰がどのプロジェクトで read / publish / history か) とノード名簿 (どのノードが誰のもので、どこに到達できるか) の正本**である。Synergos は転送と同期の本体で、Curare が無くても動く。Curare はデザイナー向けの版・レビュー・承認を持ち、それが要る場面でだけ capability を出してバイトの移動を認可する。Curare はバイトを中継せず、会員資格を輸出・発行・判定しない。
- `spec/feature/opaque-custody.md:9`: 「別listenerを起動しない」。`:12`: 「Clientには用途・期限・実体を限定したHMAC署名URLだけを渡す」。`:17`: 「HMAC-SHA256を計算する」。`:53`: `SYNERGOS_CUSTODY_RECEIPT_SECRET`「Cuと共有する32文字以上の署名鍵」。
- `spec/plan/cpp-migration.md:38-47`: 「Local IPC authorization: same-user checks」「preserve protobuf schemas, QUIC ALPN, peer authentication」「Reject path escapes, symlinks/junctions」「Preserve canonical receipt bytes, HMAC」。
- `synergos-core/src/invite_token.rs:19-21` は自ら「現状 Synergos にはプロジェクト単位の参加 ACL が無い … このトークンは「参加手順の同梱」であって認可境界ではない。認可は Cloudflare Mesh 等のネットワーク層で行う」と記す。

ユーザー要求は、AI 開発・ゲーム開発の課題解決、セキュリティ、転送速度と効率の 3 条件を同時に満たすことである。本書はセキュリティを扱い、転送効率を損なわない上限 (§1.2 の E1〜E7) を守る。

オーナー決定 (2026-10-23) のうち本書を拘束するもの:

| # | 決定 | 本書での扱い |
|---|---|---|
| 1 | 3 テーマ × 2 repo = 6 本の設計書 | 本書はそのうち Synergos の security。相手は Curare: spec/plan/redesign-security.md |
| 2 | 提出経路は Revisor のみ | GitHub Actions を含む GitHub 側の検査は置かない (D-12)。本書も Revisor 経由で取り込む |
| 3 | CDC / チャンク化を再検討。チャンク保管の独立モジュール化もありうる | 判断は転送効率文書 (D-T03 で独立モジュール `synergos-chunkstore` を採用)。本書はその境界のセキュリティ契約 SPEC-SY-chunk-access を持つ (D-11、§4.24) |
| 4 | Cernere 側 (短寿命 capability 署名鍵、SSF/CAEP 失効通知、step-up) は SPEC-CR-* として依頼済み | 本書は依存として参照する。Synergos / Curare には実装しない |
| 5 | quick-xml 0.30 は「依存自体が要るか」から判断。C++ 移行で GUI が置き換わるなら依存ごと外す | 経路を作るのは eframe の `accesskit` feature だけ。P0 で `synergos-gui` をサーバ workspace から `desktop/` workspace へ移し、`accesskit` を外して経路を消す。egui バイナリそのものの撤去は C++ desktop-gui が受入ゲートを通ってから (D-13, SECURITY-M30) |
| 6 | HLO2・custody 強化・認証強化は Rust に入れず C++ 移行契約として再仕様化 | §4 の各機構に区分 C (契約の変更) / I (実装要件) / O (運用・設定) を付ける。§4.37 に改訂案 |
| 7 | 外向きの DERP 型 relay は C++ 版でだけ作る。それまで拠点と外部環境は Tailscale (`synergos-node` は tailnet IP にだけ bind) | SPEC-SY-relay-auth / SPEC-SY-relay-v2 は C++ 移行の契約。現行 Rust には何も足さない (`design/authority-amendment.md` §3) |
| 8 | プロジェクト会員とノード名簿の正本は Cernere。synergos-control は Cernere 署名の会員証・名簿を配布・キャッシュ・検証するだけ | 会員証 (membership LCT)・node card・ProjectRoster・RosterStatus・OfflineLease は Cernere が署名する。control は会員を決めず、その発行鍵を持たない。enrollment の承認は Cernere (step-up)。失効は Cernere から SSF/CAEP で流れる (SPEC-CR-membership-issuer、SPEC-CR-node-roster。Actio `actio:2a7af0fa`、`actio:fd0e4de8`) |
| 9 | Curare はデザイナー向けの版管理で、Synergos のハコ | 転送・同期の本体は Synergos 単体。Curare が関わるのは版・レビュー・承認が要る場面だけ。Curare はバイトの経路でも会員資格の権威でもない |

### 1.2 ゴール (測定可能な到達条件)

両 repo を合わせた全体表。G 番号は両文書で共通で、Curare 文書は番号と要約だけを参照する。「判定点」は目標値をどちらの repo の受入試験で判定するか (Sy = Synergos、Cu = Curare、両方)。「提案値」は neco の確認待ち (Q9)。

| # | ゴール | 目標値 | 判定点 | 主な発見 | 受入試験 |
|---|---|---|---|---|---|
| G1 | 失効の伝播 | Cernere SSF イベント・運用者 freeze・ノード隔離から、QUIC accept と stream 振分け・relay join・custody 償還・Curare API・取得・`/dl` の**全 PEP** での拒否まで **p99 60 秒以内**。300 秒より古い失効バンドルでは新規セッションと償還を拒否。**分断時の例外**: control に届かない PEP では、失効済み peer の**既存**セッションが membership LCT の `exp` (≤3600 秒) まで残る。オフライン貸与 (D-21) を受理したプロジェクトでは貸与の窓まで残る。この残存は D-21 で明示的に受け入れる | 両方 | SEC-AZ-17, SEC-BR-12, SEC-DR-07, SEC-CR-09 | AT-SY-SEC-10, AT-SY-SEC-32, AT-SY-SEC-39, AT-CU-SEC-10 |
| G2 | 資格の寿命 | custody capability ≤300 秒 (ブラウザ URL 形式 ≤120 秒)、membership LCT ≤3600 秒、custody delegation LCT ≤3600 秒、招待は既定 24 時間・最大 7 日・単回、enrollment claim code (Cernere が step-up 後に発行、SPEC-CR-node-roster) は 15 分・単回。magic link は既定 72 時間・最大 14 日、actor キャッシュ ≤60 秒 (この 2 つは Cu) | 両方 | SEC-CR-03, SEC-CR-05, SEC-AZ-23 | AT-SY-SEC-13, AT-SY-SEC-31, AT-CU-SEC-03, AT-CU-SEC-15 |
| G3 | 鍵の分離 | 2 サービスまたは 2 用途で共有される秘密値 **0**。鍵の **100 %** が `kid` を持つ。ローテーションは現行と前世代の kid を併用受理し、flag day を作らない | 両方 | SEC-CR-04, SEC-DR-08, SEC-SC-09 | AT-SY-SEC-27 |
| G4 | テナント分離 | §6 の攻撃試験で組織間・プロジェクト間の読取成功 **0** (侵害ノード 1 台、capability 1 件、履歴ノード 1 台の各条件) | 両方 | SEC-AZ-01, SEC-BR-05, SEC-BR-13 | AT-SY-SEC-02, AT-SY-SEC-08, AT-SY-SEC-38, AT-CU-SEC-02 |
| G5 | ローカル全権の撤廃 | 同一 OS uid であることは daemon 管理権を意味しない。破壊的 IPC コマンドの **100 %** が確認 nonce か step-up を要求 | Sy | SEC-AZ-09, SEC-BR-02 | AT-SY-SEC-05, AT-SY-SEC-34 |
| G6 | 復旧可能性 | 確定・commit 済み・archive 済みの全版が、任意の**単一**資格の侵害後も **30 日以上** (プロジェクト方針での下限 7 日) 残る。受信で置換された旧バイトを **14 日以上** 保持。月次の復旧訓練が合格。Curare DB は RPO ≤15 分・RTO ≤4 時間 (提案値、Cu) | 両方 | SEC-IN-01, SEC-IN-05, SEC-IN-06, SEC-DR-09 | AT-SY-SEC-11, AT-SY-SEC-18, AT-SY-SEC-30, AT-SY-SEC-36, AT-CU-SEC-05, AT-CU-SEC-06 |
| G7 | 検知 | 主体の発行・償還量が 7 日 p95 の **5 倍**超で **60 秒以内**に step-up。canary 接触で **60 秒以内**に freeze。機微なルート・コマンド・stream 種別の **100 %** がイベント種別に対応 | 両方 | SEC-DR-05, SEC-DR-06, SEC-BR-14 | AT-SY-SEC-19, AT-SY-SEC-24, AT-CU-SEC-11, AT-CU-SEC-12, AT-CU-SEC-29 |
| G8 | 濫用 | 外部ハンドラ (HTTP ルート、QUIC stream 種別、gossip topic、IPC コマンド) の **100 %** に AbusePolicy 行。洪水は 429 か stream reset で返し、OOM にしない。不正な bearer の洪水で Cernere 呼出 **0 回** (Cu) | 両方 | SEC-DA-22, SEC-DA-02, SEC-DA-14 | AT-SY-SEC-17, AT-SY-SEC-23, AT-CU-SEC-16 |
| G9 | 供給網と入力境界 | GitHub workflow ファイル **0**。セキュリティ検査の **100 %** が Revisor マージ前ケース (両方)。wire / ファイル decoder の **100 %** に fuzz target (Sy)。取込検査が合成 DCC 試料の **100 %** を検出し、正常ファイルの誤検出 **0** (Cu) | 両方 | SEC-SC-01, SEC-DA-19, SEC-DA-20 | AT-SY-SEC-21, AT-SY-SEC-22, AT-CU-SEC-21 |
| G10 | crypto-shred | 組織の KEK 破棄で、その組織の custody・履歴・複製・MinIO の全 blob が読めなくなる。manifest と監査ハッシュは残る | 両方 | SEC-CR-19, SEC-BR-13 | AT-SY-SEC-20, AT-CU-SEC-20 |
| G11 | 失効基盤の可用性 | control-node を 2 インスタンス以上、別ホスト (うち 1 つは別拠点) で動かし、月間可用性 **99.9 %** 以上。1 インスタンスの停止で PEP の手元バンドルが **60 秒**を超えて古くならない。全 control-node に届かない状態の許容上限は、オフライン貸与の無いプロジェクトで新規セッション **300 秒**・既存セッション **3600 秒** (D-21)。会員証・node card・RosterStatus は Cernere が署名するので、Cernere に届かない間は control-node が二重化されていても membership LCT の更新・enrollment・名簿の変更は止まり、RosterStatus の `exp` (300 秒) を過ぎると新規セッションは拒否される。Cernere 側の可用性・遅延の要求 (RosterStatus 60 秒ごとの再発行、失効 10 秒以内の反映、トークン発行 p99 < 1 秒) は SPEC-CR-membership-issuer / SPEC-CR-node-roster に置く | Sy | SEC-AZ-17 (fail-closed の副作用) | AT-SY-SEC-32 |

転送効率を損なわないための上限 (本書の機構はこれに縛られる。転送の目標値自体は転送効率文書が持つ):

| # | 上限 | 目標 |
|---|---|---|
| E1 | peer stream 認可の費用 | stream あたりの追加往復 **0**。会員検査は接続ごとの CapSet に対するローカル照合。セッション確立は QUIC 1-RTT 1 回で、SESS は最初の stream に載せる。状態を変える操作に **0-RTT を使わない** (内容ハッシュ指定の GET だけ可、レート制限付き) |
| E2 | Curare 経由の clone 認可 | N 実体の認可は Curare 取得呼出 **ceil(N/1000)** 回、custody 管理呼出 **0** 回 (LCT の減衰はローカル)。長時間転送の capability 更新は **300 秒に 1 回以下**。Curare と custody の delegation 更新は組織ごとに 30 分に 1 回以下 |
| E3 | 検証 CPU | 要求あたり Ed25519 検証 ≤3 回 (delegation 連鎖)、受領証あたり ≤1 回。(jti, block-hash) の検証キャッシュで範囲要求の繰返しを O(1) にする |
| E4 | 暗号化のバイト増分 | ≤0.1 %。SPEC-SY-envelope-encryption の既定 segment 64 KiB あたり 16 B の AEAD タグで 0.024 %、実体ごとの封筒ヘッダ ≤256 B を加えても 1 MiB 以上の実体で 0.05 % 未満 |
| E5 | 重複排除率の維持 | プロジェクト内収束チャンク鍵 (D-10) で、暗号化後のプロジェクト内重複排除率を非暗号化 CDC と**同一**に保つ |
| E6 | 旧バイト保持の書込費用 | 置換される旧ファイルは同一ボリュームの `displaced/` への rename。全体コピーは**しない** |
| E7 | 暗号化による読取増幅 | 暗号化済み実体への範囲読取で、復号・認証するバイト数 ≤ 要求バイト数 + 2 × segment (既定 128 KiB)。1 MiB 以上の範囲では **≤2 ×** (既定 segment では ≤1.125 ×)。転送効率文書の Bao 検証範囲 (16 KiB group)、≤1 GiB の範囲要求、約 64 KiB のチャンクと整合させる |

### 1.3 非ゴール

| 範囲 | 扱い | 判断者 |
|---|---|---|
| CDC / チャンク化の採否、チャンク保管の配置 | 転送効率文書 (`spec/plan/redesign-transfer-efficiency.md`) が 4 軸で判断した (D-T03、独立モジュール)。本書はその境界の契約 SPEC-SY-chunk-access (D-11 の不変条件) だけを持つ | neco |
| Curare の認証・権限・監査・取得台帳・webhook・MinIO・DB 役割 | Curare: spec/plan/redesign-security.md が正本。本書は SPEC-CU-* を参照する | neco / プロダクトオーナー |
| Cernere の署名鍵公開・SSF/CAEP 送信・step-up・サービス主体、Synergos のプロジェクト会員の発行 (membership LCT・ProjectRoster・OfflineLease) とノード名簿 (claim code・node card・RosterStatus) | SPEC-CR-* として Cernere へ依頼済み (会員と名簿は SPEC-CR-membership-issuer / SPEC-CR-node-roster、Actio `actio:2a7af0fa`・`actio:fd0e4de8`、オーナー決定 8)。本書は依存として扱い、Synergos は検証だけを行う | Cernere オーナー |
| 現行 Rust 実装へのセキュリティ機能追加 | 行わない (オーナー決定 6)。設定だけの暫定補償 (SECURITY-M34) は行う | neco |
| `project_ref` と Curare space の対応付け | AI・ゲーム開発文書 (`spec/plan/redesign-ai-game-dev.md`) の共有識別子が正本 | neco |
| frame 配置 (範囲・再開) | 転送効率文書の framing 契約が正本。本書は境界値の不変条件 (SPEC-SY-decode-bounds) だけを持つ | neco |
| 公開 DHT の要否 | 本書は「対象外なら削除」を既定とする (Q4、D-23) | neco |
| チャンク保管モジュールの API・chunker・pack 形式 | 転送効率文書の SPEC-SY-chunk-store が正本。本書はその境界の SPEC-SY-chunk-access だけを持つ | neco |

## 2. 現状分析 — 足りない要素と再設計すべき点

検証済み発見 (`spec/plan/redesign-analysis/findings/security-verified.json` の kept[]) のうち、repo が synergos (62 件) または both (55 件) の 117 件を載せる。dropped[] の 3 件は載せない。file:line は main @ d404f20 で確認した。パスは repo 相対。重大度は検証時の判定。「再設計の方向」の M 番号は §4 の SECURITY-Mxx。both の行で Curare 側の根拠が主のものは、Curare: spec/plan/redesign-security.md の §2 が正本行を持つ。

区分の対応: vulnerability = 脆弱性、missing-control = 欠落機構、design-gap / inconsistency = 設計ギャップ、missing-feature = 未実装機能。

### 2.1 濫用・注入 (DA)

| ID | 区分 | 現状 (file:line) | 影響 (AI / ゲーム制作での実害) | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-DA-01 | 脆弱性 | accept が HLO1 の 5 秒待ちで直列化。Retry・保留ハンドシェイク上限・IP 別上限なし (`synergos-net/src/quic/mod.rs:445-465`, `synergos-net/src/quic/hello.rs:31`, `synergos-core/src/daemon.rs:750-753`) | 半開き接続 1 本で履歴ノードやビルドノードが全 peer の受付を止め、checkout と auto-pull が無言で止まる | high | M02 の受付パイプライン (NR-1) + M21 |
| SEC-DA-02 | 脆弱性 | 受信 stream に peer 単位の本数・メモリ・時間予算なし。1 接続 100 本、1〜16 MiB の先行確保 (`synergos-core/src/daemon.rs:806`, `synergos-net/src/config.rs:694-695`, `synergos-net/src/content/bitswap.rs:117-122`) | 1 peer で daemon を OOM させ、全プロジェクトの転送が止まる | high | M21 の予算 (NR-2) + M23 |
| SEC-DA-03 | 脆弱性 | bitswap が 1024 CID × 16 MiB を受け、wantlist を全 decode 後に検査。MemoryContentStore は退避なし (`synergos-net/src/content/bitswap.rs:37-39`, `synergos-net/src/content/bitswap.rs:237,260`, `synergos-net/src/content/store.rs:26-27`) | 大規模プロジェクトのカタログ同期が全メンバーの RAM を枯渇させる | medium | M03 (bitswap を `(project_ref, cid)` 鍵化) + M21 |
| SEC-DA-04 | 脆弱性 | 未要求 TXFR に総量上限・quota・空き容量予約なし。方針判定は本体書込の後 (`synergos-core/src/exchange/mod.rs:700-702`, `synergos-core/src/exchange/mod.rs:730-731`, `synergos-core/src/exchange/mod.rs:746-770`) | 作業ボリュームを埋めて DCC 保存・git・manifest 書込が同時に壊れる。履歴ノードでは全プロジェクトの archive が止まる | high | M13 (SPEC-SY-pull-only-receive と事前予約) |
| SEC-DA-05 | 脆弱性 | `.synergos` 保護が大小文字区別で、`.git/hooks` `.vscode` `.cargo` を任意 peer が書ける (`synergos-core/src/manifest.rs:305-311`, `synergos-core/src/project.rs:349-359`, `synergos-core/src/manifest.rs:322-344`) | 毎日 Unity / Unreal / VS Code で開くプロジェクトが、資産同期経由のコード実行経路になる | high | M13 (SPEC-SY-path-policy) |
| SEC-DA-06 | 脆弱性 | hook は shell 文字列。Windows は `cmd /C` + `raw_arg` で `%SYNERGOS_FILE%` を展開してから解析 (`synergos-core/src/hooks/runner.rs:430-439`, `synergos-core/src/hooks/runner.rs:316-324`, `docs/hooks.md:109`) | 受信ファイル名の `&` で post-receive 変換 (PNG→KTX、PSD 平坦化) がコマンド注入になる | high | M12 (SPEC-SY-sandbox-profile、argv のみ) |
| SEC-DA-07 | 脆弱性 | gossip が無制限の状態を作る。TransferLedger は退避なし、ConflictAlert は無署名で永続、fan-out は (message, peer) ごとに stream (`synergos-net/src/chain/ledger.rs:56-57,113`, `synergos-net/src/gossip/message.rs:86`, `synergos-core/src/conflict/mod.rs:87,343`) | 幻の衝突がリリースを止め、負荷と区別できない | medium | M03 (origin = signer = sender) + M21 (台帳の退避) |
| SEC-DA-08 | 脆弱性 | 数百 B の FileWant 1 件でファイル全体を送信。重複排除・レート・quota なし。履歴ノードでは S3/GDrive 復元と遠隔コピー削除を強制 (`synergos-core/src/exchange/mod.rs:944-951`, `synergos-core/src/exchange/mod.rs:918,929`, `synergos-core/src/history/store.rs:272-280`) | ゲームビルドや AI checkpoint の egress 予算を外部から燃やされる | medium | M03 + M21 (FileWant 予算) + M15 (copy-not-move) |
| SEC-DA-09 | 脆弱性 | relay は既定 64 MiB の WS メッセージ、無認証、`idle_timeout_secs` は未使用、`0.0.0.0` bind (`synergos-relay/src/server.rs:156`, `synergos-relay/src/server.rs:160`, `synergos-relay/src/config.rs:17-18,34`) | 硬い NAT 越しの外注 (約 1 割の組) の経路を匿名 1 クライアントが落とす | medium | M06 + M21。P4 まで非配備 (M34) |
| SEC-DA-10 | 脆弱性 | 無認証の `/peer-info` が要求ごとに外部 IP-echo を最大 6 件、キャッシュなしで取得 (`synergos-core/src/peer_info_server.rs:70-78`, `synergos-core/src/peer_info_server.rs:137-145`, `synergos-core/src/peer_info_server.rs:156-158`) | 増幅と、ノードの存在・IP の漏えい | medium | M05 (node-card で置換) + M22 |
| SEC-DA-11 | 脆弱性 | 招待・`peer add-url`・`bootstrap_urls` から任意ホストへ GET し、攻撃者指定 UDP へ QUIC 発信を繰り返す (`synergos-core/src/peer_bootstrap.rs:193-215`, `synergos-core/src/peer_bootstrap.rs:101-110`, `synergos-core/src/peer_bootstrap.rs:140-142`) | Slack / Discord で流れる招待リンクが作業機をビルドサーバ・NAS・メタデータ endpoint の探査機に変える | medium | M05 + M22 (SPEC-CU-egress-policy の分類器) |
| SEC-DA-12 | 脆弱性 | control の heartbeat が鍵検査前にレジストリ全体を大域 mutex 下で clone し、受理ごとに全 JSON を書いて fsync。全 token にレート制限なし (`synergos-control/src/store/json_store.rs:211-212`, `synergos-control/src/api/heartbeat.rs:58`, `synergos-control/src/api/ui.rs:69`) | 失効の要である control が、事件中に監視対象のノードから飽和させられる | medium | M04 (検証先行) + M21 |
| SEC-DA-13 | 脆弱性 | custody の遅い PUT が実体ロックを最大 30 分保持し、他操作は 10 秒で 503。capability は期限内再生可、同時 PUT と総ディスクの上限なし (`synergos-custody/src/handlers.rs:217-218`, `synergos-custody/src/handlers.rs:234-239`, `synergos-custody/src/store.rs:72`) | 停滞した数 GB のテクスチャ upload 1 本がリリース候補を止める | medium | M09 (jti 単回) + M21 (償還予算) |
| SEC-DA-18 | 脆弱性 | Tauri の `csp: null`、peer 由来文字列が未検証でログへ、IPC の全コマンド debug ログ (`synergos-tauri/src-tauri/tauri.conf.json:25`, `synergos-core/src/peer_bootstrap.rs:151`, `synergos-core/src/ipc_server.rs:504`)。Curare 側の X-Request-Id は Curare 文書 | ログ偽装と XSS 防壁の欠如が漏えい後の調査を弱める | medium | M25 (`client_request_id` の分離) + M32 (CSP) |
| SEC-DA-19 | 設計ギャップ | 手組みの msgpack / JSON 6 形式に版バイトなし、endianness 混在、decode-before-limit、fuzz なし (`synergos-net/src/content/bitswap.rs:237`, `synergos-net/src/quic/hello.rs:155-166`, `synergos-net/src/content/session.rs:218`) | 全作業機が他者のバイトを解析する。panic や二乗 decode が全体停止になり、転送再設計も安全に展開できない | medium | M23 (SPEC-SY-decode-bounds) + M01 |
| SEC-DA-20 | 未実装機能 | 実行可能アセット形式を検査・隔離しない。変換 hook は信頼できないバイトを sandbox なしで解析 (`synergos-core/src/hooks/runner.rs:316`, `docs/hooks.md:105-111`)。判定側は Curare 文書 | 正規の資産経路に乗ったペイロードが DCC 内で実行される。アクセス制御だけでは防げない | high | M24 (判定は Curare、Synergos は強制) + M12 |
| SEC-DA-21 | 脆弱性 | 局所 IPC に接続数・idle / read timeout なし、1 MiB 超の応答で切断、GUI の Transfers タブが毎フレーム接続 (`synergos-core/src/ipc_server.rs:122-123`, `synergos-ipc/src/transport.rs:39,94`, `synergos-ipc/src/client.rs:56`) | 10^4〜10^6 ファイルの通常操作が失敗に見える | low | M11 + M21 |
| SEC-DA-22 | 設計ギャップ | 全制限が frame / request 単位の定数で、主体単位の quota・レート・挙動点数・遥測が無い。登録テストも無い (`maps/sy-content.md` W46、`maps/sy-control-custody.md` W3 に集約。個別の file:line は本表の DA 行) | 人と自動化 (DCC プラグイン、CI、AI データ生成) が同じ基盤を共有し、攻撃が負荷に紛れる | high | M21 (SPEC-CU-abuse-policy の Synergos 行と登録テスト) |

### 2.2 検知・対応・運用 (DR)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-DR-02 | 設計ギャップ | Synergos の admin / custody 操作が監査に残らず帰属できない (`synergos-control/src/api/nodes.rs:293`)。Curare 側の欠落は Curare 文書 | 未公開キャラクター・カットシーン・checkpoint の漏えい範囲を特定できない | high | M25 |
| SEC-DR-03 | 設計ギャップ | custody capability に操作 ID・capability ID が無い (`synergos-custody/src/signature.rs:52-53`) | DCC → Curare → custody → P2P の経路で漏えいを帰属できない | medium | M25 (trace_id / jti / operation_id の相関) |
| SEC-DR-04 | 欠落機構 | custody は受付・受領証・URL 発行・PUT/GET 償還・回収のいずれも記録しない。tombstone は誰が・いつを持たない (`synergos-custody/src/handlers.rs:173-183`, `synergos-custody/src/store.rs:212-219`, `synergos-custody/src/error.rs:19`) | 2026-10-23 から custody が Curare の既定保管先で、全未公開資産が無記録で読まれ・消される | high | M25 (SPEC-SY-custody-journal) |
| SEC-DR-05 | 未実装機能 | peer の FileWant は任意版を返し、主体別の計量・閾値・自動応答が無い (`synergos-core/src/exchange/mod.rs:944-950`) | 量的な流出 (GTA6 / Insomniac 型) に数時間〜数週間気付けない | high | M26 |
| SEC-DR-06 | 未実装機能 | canary・honeytoken が無い (`synergos-control/src/store/mod.rs:31-63`) | 誤検知ほぼゼロの侵害信号が得られない | medium | M26 |
| SEC-DR-07 | 欠落機構 | kill switch・主体 / プロジェクト凍結・データプレーンへ届く隔離・発行済み capability の失効が無い (`synergos-net/src/quic/mod.rs:459-475`, `synergos-custody/src/signature.rs:52-53`, `docs/mesh-operations.md:296-303`) | 1 外注先を即時隔離できず、共有秘密の交換で全パイプラインを止めるしかない | high | M07 |
| SEC-DR-08 | 設計ギャップ | custody token・受領証秘密・control admin token を Curare と共有し、kid なし、Cloudflare token は共通 entry (`docs/site-operations.md:30-33`, `synergos-custody/src/config.rs:10-11`, `synergos-custody/src/handlers.rs:33`) | 1 ホストの侵害で全組織の鍵交換 (flag day) が要る | high | M09 + M20 + M33 |
| SEC-DR-09 | 欠落機構 | バックアップ・不変コピー・復旧訓練なし。破壊は単一ノード上で不可逆 (`docs/site-operations.md:21-24`, `synergos-core/src/ipc_server.rs:1575-1622`) | ランサム時に身代金を拒否する根拠 (健全なバックアップ) が無い | high | M28 (+ M15) |
| SEC-DR-10 | 脆弱性 | control の `/v1/health` が version を返さない (`synergos-control/src/api/mod.rs:100-105`)。RULE_SRE §2 に反する | 障害の切り分けが遅れる | medium | M32 |
| SEC-DR-11 | 欠落機構 | admin UI に CSP / HSTS / Host・Origin 検査なし (`synergos-control/src/api/mod.rs:120-129`, `synergos-control/src/api/ui.rs:79-86`) | admin token を持つ UI に多層防御が無い | medium | M32 (+ M10 の cookie 化) |
| SEC-DR-12 | 欠落機構 | control の admin / heartbeat の 401 を記録せず、ロックアウトなし (`synergos-control/src/auth.rs:17-26`, `synergos-control/src/api/heartbeat.rs:100-103`) | 資格窃取の最初の信号を捨てている | medium | M26 + M21 |
| SEC-DR-13 | 欠落機構 | daemon にセキュリティイベント流が無い。拒否は文脈なしの debug、受理時に遠隔アドレスを記録しない、破壊的 IPC も記録しない (`synergos-core/src/daemon.rs:811-814`, `synergos-core/src/main.rs:29`, `synergos-core/src/daemon.rs:750-754`) | 作業機・レンダーノード・外注機で変換 hook が動くのに、感染範囲を絞れない | medium | M25 (daemon の `security/*.jsonl`) |
| SEC-DR-14 | 設計ギャップ | Vestigium / Concordia 連携なし。Cloudflare のネットワークログは 24 時間 (`docs/mesh-operations.md:339`) | 検知が遅れ、数か月分の組織単位監査を出せない | medium | M25 |
| SEC-DR-15 | 設計ギャップ | dark node reconcile にスケジューラが無く、heartbeat 欠落を警報しない。custody の scrub が無い (`docs/mesh-operations.md:220`, `docs/mesh-operations.md:296-301`, `synergos-control/src/main.rs:57-92`) | 手動検知は外注・レンダーノードの入替えで崩れる。静かなバイト置換が全ビルドと学習を汚す | medium | M26 (+ M15 の scrub) |
| SEC-DR-16 | 未実装機能 | RULE_SRE §1 の `spec/operations.md` と、資格漏えい・侵害ノード・悪性送信・資産漏えい・ランサムの runbook が無い (`docs/mesh-operations.md:227`, `docs/site-operations.md:37-39`) | 発注元の IR 評価を通らず、AI 運用者が辿る手順も無い | medium | M33 |
| SEC-DR-17 | 欠落機構 | custody reclaim は即時 unlink、history purge は全版消去。承認・猶予・記録なし (`synergos-custody/src/store.rs:210-221`, `synergos-core/src/history/store.rs:427-441`) | 妨害が不可逆になる | medium | M16 |

### 2.3 供給網・設定 (SC)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-SC-01 | 欠落機構 | マージ前の供給網ゲートが無い。GitHub Actions は Revisor のマージ後にしか走らず、RULE.md §6.5 は workflow を禁じる (`.github/workflows/ci.yml:3-7`) | ほぼ AI が書くリポで、幻覚・typo-squat の依存名が無検査で入る | high | M29 |
| SEC-SC-02 | 脆弱性 | `LUDIARS/AIFormat/.github/workflows/harness-checks.yml` の可変参照を `permissions:` なしで呼ぶ (`.github/workflows/harness.yml:15-17`) | AIFormat の 1 編集が全リポで資格付きのコード実行になる | high | M29 (workflow 削除で消滅) |
| SEC-SC-03 | 脆弱性 | 可変タグ (`@v4`)・`dtolnay/rust-toolchain@stable`・版なし `cargo install`・prefix 復元の cache (`.github/workflows/ci.yml:21-23`, `.github/workflows/ci.yml:137-138,168-169`, `.github/workflows/native.yml:20,33-35`) | 汚染 cache から改ざんバイナリが出る | medium | M29 |
| SEC-SC-05 | 欠落機構 | `cargo audit` 非ブロッキング、`deny.toml` なし、GUI 依存がサーバと lockfile を共有 (`.github/workflows/ci.yml:157-173`, `Cargo.toml:2-12`) | 露出サービスの依存リスク信号が読めない | medium | M29 + M30 |
| SEC-SC-06 | 脆弱性 | 本番サイトで 866 crate の build.rs / proc-macro を、Vault agent 資格に届く環境でコンパイル (`scripts/site/setup.mjs:7-11`, `Cargo.lock`) | custody token・受領証秘密・Cloudflare token が build 時コードから届く (Codecov 型) | high | M31 |
| SEC-SC-07 | 欠落機構 | 署名・checksum・SBOM・provenance なし。Gatekeeper 回避を教示。バイナリは `0.1.0` を自己申告 (`docs/setup-macos.md:65-67`, `synergos-tauri/src-tauri/tauri.conf.json:28-35`, `synergos-core/src/control_heartbeat.rs:107`) | どのビルドが動いているか分からず、脆弱版の棚卸しができない | high | M31 |
| SEC-SC-08 | 脆弱性 | cloudflared を `releases/latest` から無検証で導入し、PATH 先頭の実体を全環境付きで起動 (`OPERATIONAL-TEST.md:55-57`, `docs/platforms.md:56`, `synergos-net/src/tunnel/mod.rs:336-347`) | 差し替えられた cloudflared が中継される全バイトを見る | medium | M31 (`{version, sha256}` 固定) + M20 (`spawn_child()`) |
| SEC-SC-09 | 設計ギャップ | control が組織共通の Cloudflare API token を持ち、custody / admin 秘密を Curare と値で共有 (`docs/site-operations.md:30-33`, `excubitor.catalog.yaml:35-37`, `scripts/site/lib/control-config.mjs:13-16`) | control 1 侵害が全 LUDIARS サービスの edge 操作へ広がる | high | M10 |
| SEC-SC-10 | 設計ギャップ | 主根拠は Curare の Vault 403/404 = 秘密なし扱い (Curare 文書)。Synergos 側は `scripts/site/lib/control-config.mjs:13-17` が `CF_ACCOUNT_ID`・`CF_API_TOKEN`・`SYNERGOS_CONTROL_ADMIN_TOKEN` の存在だけを起動前に検査し、custody 系の秘密は検査対象外 | 片側の未設定が起動後まで見えない | medium | M20 (SPEC-CU-secrets-manifest) |
| SEC-SC-11 | 脆弱性 | サイト移行 bundle が平文で、運用者指定の SHA-256 だけで検証 (`scripts/site/export-data.mjs:9-10`, `scripts/site/lib/dir-bundle.mjs:59,114`) | 全 custody 実体と登録簿の平文コピーが受取ディレクトリに残る | medium | M28 (SPEC-CU-site-bundle-v2) |
| SEC-SC-12 | 設計ギャップ | hook と cloudflared が daemon 環境を継承、IPC が全コマンドを Debug 出力、秘密の newtype が無い (`synergos-core/src/hooks/runner.rs:315-325`, `synergos-core/src/history/rotation/s3.rs:40-44`, `synergos-core/src/ipc_server.rs:504`) | 第三者の変換ツールが資格読取機になり、debug ログが共有物になる | medium | M12 + M20 (`Secret<T>`, `spawn_child()`) |
| SEC-SC-13 | 設計ギャップ | `rust-toolchain.toml` なし (custody は Rust >= 1.89 を要求) (`synergos-custody/Cargo.toml:5`) | 再現ビルドと provenance の前提が崩れる | medium | M29 (toolchain pin の検査) |
| SEC-SC-14 | 欠落機構 | admin-ui の bundle を版なし `dx` で運用者機上でビルドし、無認証・無検証で配信。Tauri の React フロントは CI で build も audit もされない (`docs/admin-ui.md:31-37`, `docs/admin-ui.md:49-52`, `.github/workflows/ci.yml:115-118`) | 全ノードと Cloudflare 操作を握る管理画面が最も狙われる供給網標的 | medium | M31 (署名付き admin-ui manifest) |
| SEC-SC-16 | 欠落機構 | secret scan なし。文書と API 応答が bearer をコマンドラインで流す (`SETUP.md:74`, `synergos-control/src/api/mesh_setup.rs:168-170`) | AI が大量に書く文書に秘密が貼られる | medium | M29 |
| SEC-SC-17 | 設計ギャップ | 依存追加に admission 方針が無く、bot commit は無署名 (`synergos-net/Cargo.toml` の dependencies、commit fcfb2b7) | エージェントが自由に依存を足し、同じ自動化がマージする | medium | M29 (`deps-approvals.json`、公開から 7 日以上) |
| SEC-SC-18 | 設計ギャップ | コード・ビルド出力・永続データが同じ checkout に同居し、catalog に dev 値 (`scripts/site/lib/paths.mjs:9-19`, `.gitignore:25-26`, `excubitor.catalog.yaml:25,29,33`) | `git clean`・再 clone で custody が消える | medium | M20 (+ M08 の OS サービスディレクトリ) |

### 2.4 通信・暗号 (CR)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-CR-01 | 脆弱性 | HLO1 Hello は再生可能。server nonce・TLS チャネル束縛なし、`ts_ms` 未検査。accept は既存の生きた接続を置換して閉じる (`synergos-net/src/quic/hello.rs:42-49`, `synergos-net/src/quic/hello.rs:93-102`, `synergos-net/src/quic/mod.rs:459-475`) | 外部者が信頼済みアーティスト / CI / 履歴ノードとして版を送り込み、宛先ファイルを受け取る | critical | M02 (SPEC-SY-hlo2-session-binding) |
| SEC-CR-02 | 脆弱性 | bootstrap の信頼アンカーが平文 HTTP の TOFU。`/peer-info` は無署名で、広告された peer_id を pin (`synergos-core/src/peer_bootstrap.rs:196-200`, `synergos-core/src/peer_bootstrap.rs:152-161`, `docs/mesh-two-node-checklist.md:48-54`) | 偽の bootstrap 1 回で外部者が完全な peer になる | high | M05 (SPEC-SY-node-card) |
| SEC-CR-03 | 脆弱性 | 自己完結型招待は既定で無期限、nonce 未消費、多回使用、失効不能、招待先の束縛なし (`synergos-core/src/invite_token.rs:44-47`, `synergos-core/src/ipc_server.rs:982-993`, `synergos-core/src/invite_token.rs:17-21`) | 漏れた招待がプロジェクト資産への恒久ドアになる | high | M05 (SPEC-SY-invite-v2) |
| SEC-CR-04 | 脆弱性 | custody は `receipt_secret` を bearer 比較・受領証 HMAC・capability URL HMAC の 3 用途に使い、Curare と共有 (`synergos-custody/src/handlers.rs:33-34`, `synergos-custody/src/handlers.rs:185-188`, `synergos-custody/src/handlers.rs:125-128`, `synergos-custody/native/signature.cpp:24-26`) | 受領証の検証者が capability を作り、受領証を偽造できる。リリース証拠が検証不能になる | high | M09 (SPEC-SY-custody-receipt-v2 ほか) |
| SEC-CR-05 | 設計ギャップ | URL 内 bearer capability。署名は query、期限内は再生可、検証時の最大寿命・呼出者束縛・アクセスログなし (`synergos-custody/src/handlers.rs:189-194`, `synergos-custody/src/handlers.rs:196-208`) | 一括取得計画は通常の clone 手順なので、1 件の漏えいがリリース全体の dump になる | medium | M09 (SPEC-SY-custody-capability-v2) |
| SEC-CR-06 | 脆弱性 | control / custody に TLS が無く、文書は mesh 露出を指示。heartbeat は mesh への http を許す (`synergos-control/src/main.rs:82-89`, `docs/mesh-operations.md:126-130`, `synergos-core/src/control_heartbeat.rs:56,140`) | 文書推奨の構成で admin・node・custody の bearer が平文で流れる | high | M08 + M20 + M34 |
| SEC-CR-07 | 脆弱性 | identity passphrase・node key・AWS 資格が daemon の環境変数にあり、全子プロセス (cloudflared、hook) が継承 (`synergos-net/src/tunnel/mod.rs:336-347`, `synergos-core/src/hooks/runner.rs:315-324`, `synergos-net/src/identity.rs:40`) | Maya / FBX 変換 hook が daemon の全秘密を読める (Codecov 型) | medium | M12 + M20 |
| SEC-CR-08 | 脆弱性 | ノード識別鍵は既定で平文、非原子的に書いて権限は後付け、`/tmp` や `.` へフォールバック、Windows ACL なし、暗号化形式に KDF 版と AAD なし (`synergos-net/src/identity.rs:125-137`, `synergos-net/src/identity.rs:83-84`, `synergos-net/src/identity.rs:198`) | 共有ワークステーション 1 台での鍵窃取が検知不能な識別子複製になる | medium | M04 (OS keystore、v2 ファイル形式) |
| SEC-CR-09 | 欠落機構 | 鍵ライフサイクルが無い。verifier は有効期間を見ず、accept / connect は失効リストを参照せず、admin / custody / receipt 秘密に kid なし (`synergos-net/src/quic/verifier.rs:70-80`, `synergos-net/src/quic/mod.rs:437-482`, `synergos-control/src/api/heartbeat.rs:88-91`) | ノードの失効手段が「メッシュ再構築」しかない | high | M04 + M07 |
| SEC-CR-10 | 設計ギャップ | 1 つの ed25519 鍵が HLO1・gossip・chain block・招待 JSON の 4 構造に署名し、ドメイン分離と版が無い。gossip の正規バイトは曖昧 (`synergos-net/src/quic/hello.rs:42-49`, `synergos-net/src/gossip/message.rs:110-119`, `synergos-net/src/chain/file_chain.rs:76-85`) | 署名構造を足すたびに衝突面が増える。再設計で署名に頼る範囲が広がる | medium | M01 (SPEC-SY-signed-envelope) |
| SEC-CR-11 | 設計ギャップ | heartbeat は bearer の node key 保持だけを示し、`peer_id` は自己申告・無署名・付け替え可能 (`synergos-core/src/control_heartbeat.rs:103-111`, `synergos-control/src/api/heartbeat.rs:88-91`) | control のレジストリを「誰が資産を出し入れできるか」の根にできない | medium | M04 (SPEC-SY-node-enrollment) |
| SEC-CR-12 | 設計ギャップ | 受領証・capability の独自正規 JSON を Rust・TS・C++ の 3 実装で持ち、規則が分岐。kid / alg / iat / nonce なし、時刻ずれ許容なし、署名を保存しない (`synergos-custody/src/signature.rs:15-16,30`, `synergos-custody/native/canonical.cpp:27,43`, `synergos-custody/src/handlers.rs:118-124`) | 検証の無言の分岐や時刻ずれで全提出が止まる | medium | M01 (JCS) + M09 (receipt v2) |
| SEC-CR-13 | 設計ギャップ | ALPN なし、pin verifier は validity / intermediates / server_name を無視、認証失敗をエラー文字列の部分一致で分類、`enable_0rtt` / `max_udp_payload_size` は未使用 (`synergos-net/src/quic/mod.rs:543-546`, `synergos-net/src/quic/mod.rs:266-271`, `synergos-net/src/quic/verifier.rs:70-80`, `synergos-net/src/config.rs:585-587,696-699`) | 混在版メッシュ (アーティスト機が履歴ノードより古い) が不透明に失敗する | medium | M02 (ALPN `synergos/2`) + NR-7 |
| SEC-CR-14 | 脆弱性 | relay は平文 WS、room_id だけが秘密、join 認証なし、`from_peer_id` 詐称可、中継 payload の E2E 暗号なし (`synergos-relay/src/lib.rs:13-15`, `synergos-relay/src/config.rs:34`, `synergos-relay/src/server.rs:187-200`) | relay 経路が直結経路の全性質を黙って下げる | high | M06 (SPEC-SY-relay-auth) |
| SEC-CR-15 | 設計ギャップ | 静的 admin bearer 1 本、admin token を sessionStorage、Cloudflare token をブラウザ往復、node_key / connector_token を JSON とシェル命令で返す、custody admin が同一 listener (`synergos-control/src/auth.rs:12-28`, `synergos-admin-ui/src/session.rs:6,26`, `synergos-control/src/api/mesh_setup.rs:168-171`) | 組織全体の親鍵がブラウザタブと Curare ホストに露出する | medium | M10 |
| SEC-CR-17 | 脆弱性 | IPC が招待 token や 1 MiB payload を含む全コマンドを debug 出力し、capability 署名が URL query に載る (`synergos-core/src/ipc_server.rs:504`, `synergos-ipc/src/command.rs:11,56-57,209-212`, `synergos-custody/src/handlers.rs:192-194`) | サポート共有のログが資格の第二の流通経路になる | low | M11 + M20 (`Authorization: Capability` ヘッダ化) |
| SEC-CR-18 | 欠落機構 | Tauri GUI は CSP 無効・updater 署名なし・コード署名なし。文書は Gatekeeper 回避と無 pin の cloudflared を教示 (`synergos-tauri/src-tauri/tauri.conf.json:24-26`, `synergos-tauri/src-tauri/Cargo.toml:21-23`, `synergos-tauri/src-tauri/tauri.conf.json:28-35`) | 最も多く導入される面が、最も弱い配布完全性を持つ | medium | M31 + M32 |
| SEC-CR-19 | 未実装機能 | custody・履歴ノード・S3/GDrive 冷蔵層 (生の内容ハッシュ名) に保管時暗号化なし (`synergos-custody/src/store.rs:180-207`, `synergos-core/src/history/rotation/mod.rs:97-98`, `synergos-core/src/history/rotation/s3.rs:66-70`) | 資格窃取 = 資産窃取。離脱時に crypto-shred で消去を証明できない | medium | M27 (SPEC-SY-envelope-encryption) |
| SEC-CR-20 | 脆弱性 | サイト bootstrap が秘密を過剰に伝播し、Cloudflare token entry を共有 (`docs/site-operations.md:30`, `scripts/site/run-control.mjs:21-24`)。主根拠の migration DSN は Curare 文書 | bootstrap で漏れた資格で台帳を改ざんされる | medium | M20 (手順ごとの秘密注入) |
| SEC-CR-21 | 欠落機構 | 移行 bundle は平文で、運用者指定の SHA-256 だけが完全性の根拠 (`scripts/site/export-data.mjs:9-10`, `scripts/site/lib/dir-bundle.mjs:62-64`, `scripts/site/lib/dir-bundle.mjs:118-119`) | 拠点移転・DR の瞬間に全資産史が 1 ファイルになる | medium | M28 |

### 2.5 認可境界 (AZ)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-AZ-01 | 脆弱性 | データプレーンにプロジェクト会員資格の検査が無い。到達できる鍵保持者は全プロジェクトの会員 (`synergos-net/src/quic/mod.rs:459-476`, `synergos-core/src/daemon.rs:811-814`, `synergos-core/src/exchange/mod.rs:703-705`, `synergos-core/src/invite_token.rs:19-21`) | 未公開資産・checkpoint・データセットが mesh 到達だけで流出し、全ノードの作業ツリーが汚染される | critical | M03 (暫定 M34) |
| SEC-AZ-02 | 脆弱性 | HLO1 のクライアント識別が TLS セッションに束縛されず再生可能。再生 Hello が被害者の接続を上書き (`synergos-net/src/quic/hello.rs:42-48`, `synergos-net/src/quic/hello.rs:77-103`, `synergos-net/src/quic/mod.rs:328-335`) | 会員検査・監査帰属・衝突検出が全部破れる | high | M02 |
| SEC-AZ-03 | 設計ギャップ | 招待は bearer・多回・既定無期限で発行ホストが検証せず、参加側が `project_id` を指定してローカルプロジェクトを別名化できる (`synergos-core/src/invite_token.rs:17`, `synergos-core/src/invite_token.rs:44`, `synergos-core/src/ipc_server.rs:1071-1092`) | チャット経由の外注招待で NDA 範囲を絞れない | high | M05 |
| SEC-AZ-04 | 脆弱性 | gossip は署名で認証するが認可しない。topic / 会員検査なし、署名者に束縛されないフィールド、全接続 peer へ fan-out (`synergos-net/src/gossip/node.rs:216-226`, `synergos-net/src/gossip/node.rs:244-250`, `synergos-net/src/gossip/message.rs:77-86`) | 複数顧客・NDA 案件間でファイル名とカタログが漏れ、汚染資産の pull を誘発される | high | M03 |
| SEC-AZ-05 | 脆弱性 | DHT の Announce / FindProjectPeers は無認証で送信者非束縛 (`synergos-net/src/dht/transport.rs:81-85`, `synergos-net/src/dht/node.rs:270-276`, `synergos-net/src/dht/node.rs:261-268`)。本番では休眠 | 発見の汚染と会員情報の開示 (潜在) | medium | M03 (Q4 が「対象外」なら削除) |
| SEC-AZ-06 | 脆弱性 | bitswap サーバは任意 CID を任意の接続 peer に返す。peer 識別・プロジェクト範囲・アクセスログなし (`synergos-core/src/daemon.rs:820-821`, `synergos-net/src/content/bitswap.rs:228`) | ユーザー名入りの絶対パスを含むカタログが非会員へ漏れている。チャンク経路に使えば主要な流出路になる | high | M03 |
| SEC-AZ-07 | 脆弱性 | `/peer-info` 無認証 (文書は `0.0.0.0` を指示)、http bootstrap 許容、TOFU、`peer connect` が presence を捏造 (`synergos-core/src/peer_info_server.rs:194`, `synergos-core/src/peer_info_server.rs:202`, `docs/mesh-two-node-checklist.md:48`) | 導入手順の設定ミス 1 つで他人に会員資格を渡す | high | M05 + M34 |
| SEC-AZ-08 | 脆弱性 | relay の room join は無認証、room_id が唯一の秘密、`from_peer_id` は自己申告、平文 ws で frame を素通し (`synergos-relay/src/server.rs:20-24`, `synergos-relay/src/server.rs:187-200`, `synergos-relay/src/server.rs:209-211`) | relay が平文の盗聴点・注入点になる | high | M06 |
| SEC-AZ-09 | 脆弱性 | IPC は「同一 OS uid = 全権」。資格取得失敗で fail-open、`/tmp` フォールバック、Windows DACL なし、生 frame 注入可 (`synergos-core/src/ipc_server.rs:235-241`, `synergos-core/src/ipc_server.rs:303-306`, `synergos-ipc/src/transport.rs:58`) | Ars プラグインと Curare が daemon 全権を持ち、DCC 供給網の 1 侵害で履歴ノードを消せる | high | M11 |
| SEC-AZ-10 | 脆弱性 | daemon hook が peer に上書きされうる repo 相対スクリプトを全環境で実行。プロジェクト hook の信頼は daemon 全体で、ハッシュ固定なし (`synergos-core/src/hooks/runner.rs:315-324`, `docs/hooks.md:44`, `docs/hooks.md:57`) | 外注が送ったファイルがスタジオ・履歴・CI の全ノードで実行される (Maya scriptNode ワーム型) | high | M12 |
| SEC-AZ-11 | 設計ギャップ | control は全組織・全操作に静的 admin bearer 1 本を使い、Curare も同値を保持。識別・範囲・期限・ローテーション・監査なし (`synergos-control/src/auth.rs:12-28`, `synergos-control/src/api/mod.rs:41-75`, `docs/mesh-operations.md:126-128`) | Curare の不具合の被害半径が全組織に及ぶ | high | M10 |
| SEC-AZ-12 | 欠落機構 | ノードレジストリは観測のみ。peer_id は貼り付けた node key で自己申告し、データプレーンはレジストリを読まず、revoke が peer に届かない (`synergos-control/src/api/heartbeat.rs:88-91`, `synergos-core/src/control_heartbeat.rs:103-111`, `synergos-net/src/quic/mod.rs:437-484`) | レンダーノードの離脱処理が Cloudflare だけの手作業になる | high | M04 + M07 |
| SEC-AZ-13 | 設計ギャップ | Curare → custody は 1 秘密の confused deputy。1 bearer で受付・読取・URL 発行・不可逆回収、同じ秘密で認証・capability 署名・受領証署名。capability に actor・tenant・nonce なし (`synergos-custody/src/handlers.rs:32-33`, `synergos-custody/src/handlers.rs:173-183`, `synergos-custody/src/handlers.rs:184-188`, `synergos-custody/src/contract.rs:8-15`) | Web 層 1 侵害で全組織の機密性と耐久性を失う | high | M09 |
| SEC-AZ-17 | 欠落機構 | 失効が伝播しない。発行済み capability・24 時間 admission・peer セッション・relay room・custody に失効経路なし、SSF/CAEP 受信なし (`synergos-custody/src/handlers.rs:197-207`, `synergos-net/src/quic/mod.rs:459-476`) | 最頻のセキュリティ事象である離脱を即時化できない。P2P 層は peer を忘れない | high | M07 |
| SEC-AZ-18 | 設計ギャップ | 運用者権限が env の allowlist にあり、Curare → daemon の IPC proxy が全組織の daemon 管理者になる (主根拠は Curare 文書。Synergos 側の受け口は同一 uid 全権の IPC: `synergos-core/src/ipc_server.rs:235-241`) | Web 層が履歴ノード破壊コマンドへの最短路になる | medium | M10 + M11 (Curare の IPC token は `read` のみ) |
| SEC-AZ-22 | 欠落機構 | custody・control・daemon IPC が主体単位で何も記録しない (`synergos-custody/src/handlers.rs:149-183`, `synergos-core/src/ipc_server.rs:231-250`) | 資産単位のアクセス証跡を出せず、事後対応も監査対応もできない | medium | M25 |
| SEC-AZ-24 | 脆弱性 | Curare の webhook 許可リストがプロジェクト作成者の自己申告で、co-host の control へ POST できる (主根拠は Curare 文書。標的側は `excubitor.catalog.yaml:27-29` の loopback `127.0.0.1:4250`) | control と custody への confused-deputy の足場になる | medium | M22 (Curare 正本) + M08 (custody-admin の mTLS) |
| SEC-AZ-25 | 設計ギャップ | admin token を JS から読める sessionStorage に置き、UI は CSP なし。Tauri webview は `csp:null` で daemon コマンド面を持つ (`synergos-control/src/api/mod.rs:78-84`, `synergos-admin-ui/src/session.rs:3-6`, `synergos-control/src/api/ui.rs:81-82`) | ブラウザ側の侵害が control 乗っ取りや local daemon 乗っ取りになる | medium | M10 + M32 |

### 2.6 完全性・データ損失 (IN)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-IN-01 | 脆弱性 | 同期が破壊的変更を旧バイトを残さず全メンバーへ伝播。高い版の送信 (ランサム被害者の `publish --all` を含む) が全 auto-pull ノードの作業コピーを置換 (`synergos-core/src/daemon.rs:984-986`, `synergos-core/src/exchange/mod.rs:764-769`, `synergos-core/src/exchange/mod.rs:795`, `synergos-core/src/checkout.rs:86-107`) | PSD・MB/MA・録音・整備済みデータセット・学習済み checkpoint が同期ツリーにしか無く、1 台の侵害で数分のうちに全ワークステーションから消える | critical | M14 (暫定 M34) |
| SEC-IN-02 | 脆弱性 | `.synergos` 保護が大小文字区別の完全一致で、受信時に `.git/` を保護しない (`synergos-core/src/manifest.rs:305-311`, `synergos-core/src/manifest.rs:327-344`, `synergos-core/src/project.rs:349-359`) | Windows / macOS で manifest・版下限・`hooks.toml`・履歴 index / tag を、全 OS で manifest 台帳を持つ git リポジトリを書換・巻戻しされる | high | M13 |
| SEC-IN-03 | 脆弱性 | 履歴 GC が `index.json` を唯一の生存根拠にする。GC はプロセス内 mutex (`synergos-core/src/history/store.rs:118-123` の `lock()`、`:437-438` で取得) だけを取り、OS ファイルロックが無い。lookup 経路 (`synergos-core/src/history/store.rs:267-268`) は無ロック。fsync も無い (`synergos-core/src/history/store.rs:497-509`, `synergos-core/src/history/index.rs:117-143`, `synergos-core/src/history/index.rs:139-140`) | 通常の `gc` が生きている版と sidecar を消す。発覚は旧版が要る時 | high | M15 |
| SEC-IN-04 | 脆弱性 | 保持が攻撃者に操作される。GC は wire 由来の高い版番号を残し、peer が動かせる現 manifest だけを保護し、先着の実体を archive する (`synergos-core/src/history/gc.rs:37-50`, `synergos-core/src/ipc_server.rs:1593-1600`, `synergos-core/src/daemon.rs:701-706`) | git に commit 済みのリリース manifest や学習用スナップショットの存続を peer が左右する | high | M15 |
| SEC-IN-05 | 設計ギャップ | 履歴の破壊操作にブレーキが無い。同一 uid の IPC 1 通で tag と現 manifest を無視して全消去、tag は即時削除・上書き、soft delete・承認・監査なし (`synergos-core/src/history/store.rs:441-471`, `synergos-core/src/ipc_server.rs:1592-1594`, `synergos-core/src/history/tags.rs:210-222`) | 唯一の巻戻し点が 1 回の未認証ローカル呼出で消え、誰がやったかも残らない | high | M15 + M16 |
| SEC-IN-06 | 脆弱性 | 冷蔵層回転は move。任意 peer の旧版要求で archive を復元して遠隔コピーを削除し、削除は復元コピーと index の永続化より先。backend 資格は全削除が可能 (`synergos-core/src/history/rotation/mod.rs:451-469`, `synergos-core/src/history/rotation/mod.rs:445-491`, `synergos-core/src/history/store.rs:272-287`) | コスト目的で退避した旧ビルド・データセットが、通常の peer 通信やクラッシュで消える | high | M15 + M28 |
| SEC-IN-07 | 設計ギャップ | synergos-core に fsync が皆無。作業ファイルを manifest 記録前に置換し、記録失敗は warn のみ、`projects.json` は上書き (`synergos-net/src/transfer/mod.rs:232-233`, `synergos-core/src/manifest.rs:168-169`, `synergos-core/src/exchange/mod.rs:795-831`) | 数 GB のシーン・テクスチャ・checkpoint の破損が数日後にビルドや学習で発覚する | high | M14 (NR-4 の単一 durable-write) |
| SEC-IN-08 | 脆弱性 | daemon がメモリ上の manifest で毎回全書換えし、並行の `git pull/checkout/merge` を無言で巻戻す。OS lock も CAS も無い (`synergos-core/src/project.rs:427-440`, `synergos-core/src/project.rs:484-503`, `synergos-core/src/project.rs:507-516`) | マイルストーン manifest がエラー無しに退行し、QA が誤った版を検証する | medium | M14 (intent journal) |
| SEC-IN-10 | 設計ギャップ | custody reclaim は bearer 1 本による即時・無条件の unlink。custody 側に committed / pinned 状態が無く、未受付 ID にも墓標を作る (`synergos-custody/src/store.rs:210-221`, `synergos-custody/src/handlers.rs:173-181`, `synergos-custody/src/store.rs:79-82`) | 既定保管先の全版本体が、1 資格か 1 回の reclaim 不具合で数秒のうちに消える | high | M16 (SPEC-SY-custody-lifecycle) |
| SEC-IN-11 | 設計ギャップ | durable custody が source checkout 内の gitignore 済み 1 ディレクトリ (`data/site/custody`) で、ノード ID は catalog に固定 (`scripts/site/lib/paths.mjs:9-16`, `scripts/site/run-control.mjs:16`, `.gitignore:26`, `excubitor.catalog.yaml:30`) | `git clean -fdx`・再 clone・ディスク 1 本の故障で全版本体を失い、受領証は durable のまま残る | high | M15 + M20 (+ M08) |
| SEC-IN-12 | 設計ギャップ | 確定後に保管バイトを再検証しない。サイズだけ確認して配信し、`head` は受付時の申告ハッシュを返す (`synergos-custody/src/store.rs:130-140`, `synergos-custody/src/handlers.rs:146`, `synergos-custody/src/handlers.rs:112-113`) | 破損したテクスチャ・重みが確定リリースや学習に入る | medium | M15 (scrub) + M26 |
| SEC-IN-13 | 未実装機能 | バックアップ・復旧の設計が無い。出口は停止型のサイト移行 bundle だけで、作業ツリーと履歴ノードは対象外 (`docs/site-operations.md` のデータ節, `scripts/site/lib/dir-bundle.mjs:117-118`) | 隔離バックアップの有無がランサム被害の分かれ目になる | medium | M28 |
| SEC-IN-15 | 設計ギャップ | checkout / restore が非トランザクション。対象 manifest を内容到着前に書き、大きな pin 取得は 5 分で失効、局所 restore はコピーを再検証しない (`synergos-core/src/checkout.rs:70-74`, `synergos-core/src/checkout.rs:101-108`, `synergos-core/src/exchange/mod.rs:26`) | 混在ツリーで QA やリリース再現が「成功」に見える | medium | M14 (再開可能な検証付き job) |
| SEC-IN-16 | 設計ギャップ | 受信時の archive 失敗は warn のみ。受信と履歴に quota・空き容量予約なし、一時ファイル族を回収しない (`synergos-core/src/exchange/mod.rs:846-853`, `synergos-net/src/config.rs:294-296`, `synergos-core/src/history/store.rs:752-776`) | マイルストーンやデータセット取込の最中に保持保証が黙って失効する | medium | M15 |
| SEC-IN-17 | 脆弱性 | パス封じ込めが文字列の check-then-use で、symlink / junction 差替の TOCTOU 窓がある (`synergos-core/src/manifest.rs:318-321`, `synergos-core/src/exchange/mod.rs:782-795`, `synergos-core/src/history/store.rs:89-115`) | ローカルの hook・スクリプト・git checkout が窓を突ける | low | M13 (handle 相対の操作) |

### 2.7 被害半径 (BR)

| ID | 区分 | 現状 (file:line) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| SEC-BR-01 | 脆弱性 | シナリオ (a) メンバー機 / daemon。侵害 daemon 1 つが全オープンプロジェクトを読み書きし、全 peer の作業ツリーへファイルを押し込む (`synergos-net/src/quic/mod.rs:437-475`, `synergos-core/src/daemon.rs:810-813`, `synergos-core/src/exchange/mod.rs:747,764,795`) | 外注ノート PC 1 台がスタジオ全体のワーム媒介と全流出点になる | critical | M03 + M13 + M14 |
| SEC-BR-02 | 脆弱性 | シナリオ (a-2) 同一 uid の悪性ソフト。DCC プラグインや Tauri webview に注入されたスクリプトが daemon 全権 (任意ディレクトリを開く・publish・purge・frame 注入) (`synergos-core/src/ipc_server.rs:235-241`, `synergos-core/src/ipc_server.rs:862,1575-1578,2048`, `synergos-tauri/src-tauri/tauri.conf.json:25`) | アーティストの権限で動く第三者コード 1 つが、流出と破壊を両方できる | high | M11 |
| SEC-BR-03 | 脆弱性 | シナリオ (b) node / connector token。WARP connector token が全組織の全ノードへの Mesh 到達を与え、reissue でも同じ token、Mesh 到達 = 会員 (`synergos-control/src/api/nodes.rs:263-276`, `synergos-control/src/api/mesh_setup.rs:162-171`, `synergos-control/src/api/heartbeat.rs:88-91`) | スクリプトで作る GPU / レンダー / CI 一時ワーカーの資格漏えいが、スタジオ全体のデータプレーン到達になる | high | M04 (+ M10、M03 で到達 ≠ 会員) |
| SEC-BR-04 | 設計ギャップ | シナリオ (c) control admin token。全組織・全操作の静的 bearer で、再起動なしに交換できず、監査主体なし、Curare が同値を保持 (`synergos-control/src/auth.rs:12-27`, `synergos-control/src/api/mod.rs:41-76`, `synergos-control/src/api/reconcile_api.rs:67-71`) | 事件中に侵害された運用者や Curare だけを切り離せない | critical | M10 |
| SEC-BR-05 | 脆弱性 | シナリオ (d) custody token / 受領証秘密。受領証秘密だけで全組織の全実体の GET/PUT URL をオフライン・無記録で作れ、custody token は即時・不可逆の reclaim を足す。両方を Curare と共有 (`synergos-custody/src/handlers.rs:184-188`, `synergos-custody/src/handlers.rs:33-34`, `synergos-custody/src/store.rs:210-219`, `synergos-custody/src/contract.rs:8-15`) | 1 つの HMAC 鍵の漏えいで全スタジオのリリース成果物が露出し、2 つ目で破壊される | critical | M09 (+ M16) |
| SEC-BR-08 | 設計ギャップ | シナリオ (g) 履歴ノード。既定 `projects = ["*"]` で全版を平文保管し、任意 peer に応答し、削除権付きの冷蔵層資格を持ち、自己 purge できる (`synergos-net/src/config.rs:453-455`, `synergos-core/src/exchange/mod.rs:959-975`, `synergos-core/src/history/rotation/s3.rs:37-41`) | 唯一の版履歴を流出と破壊の両面で失う | high | M03 + M15 + M27 (暫定 M34) |
| SEC-BR-09 | 設計ギャップ | シナリオ (h) Cloudflare API token。control が組織共通 token で動き scope を検査しない。request-scoped 経路で admin token 保持者が任意の token を通せる (`excubitor.catalog.yaml:35-36`, `scripts/site/lib/control-config.mjs:15,24-26`, `synergos-control/src/cloudflare/client.rs:143,235,245`) | 資産同期の不具合がスタジオ全体の edge 侵害になる | high | M10 |
| SEC-BR-10 | 脆弱性 | シナリオ (i) hook 作者。repo 相対スクリプトを全 daemon 環境・sandbox なし・履歴 index への書込権付きで実行し、post-receive で遠隔から起動される (`synergos-core/src/hooks/runner.rs:314-324`, `synergos-core/src/hooks/runner.rs:443-446`, `synergos-core/src/exchange/mod.rs:855-860`) | 自動変換 hook (PSD 平坦化、PNG→KTX2、FBX 検査) が資産受信ごとの RCE になる | high | M12 (暫定 M34) |
| SEC-BR-12 | 欠落機構 | kill switch と失効伝播がどこにも無い。識別鍵・招待は無期限、connector / admin / custody 秘密は再配備でしか変わらない (`synergos-net/src/identity.rs:125-131`, `synergos-net/src/quic/mod.rs:437-475`, `synergos-core/src/invite_token.rs:17`) | 流出の進行中に取れる手段が全スタジオの停止しかない | critical | M07 |
| SEC-BR-13 | 設計ギャップ | 組織ごとの保管・鍵の分離と保管時暗号化が無い。custody は全テナント共通の平坦ディレクトリで、admission に tenant ID なし (`synergos-custody/src/contract.rs:8-15`, `synergos-core/src/history/rotation/s3.rs:37`) | 競合スタジオを同じ SaaS 層に載せられず、外注先の保持終了を証明できない | high | M27 (+ M09 の `org` 付与) |
| SEC-BR-14 | 欠落機構 | 流出が見えない。custody はアクセスログなしで配信し、Synergos の送信は資産・プロジェクト帰属なしの info 行 (`synergos-core/src/exchange/mod.rs:992-997`) | 漏えいを SNS で知ることになる | high | M25 + M26 |
| SEC-BR-15 | 設計ギャップ | control は 1 プロセス 1 listener に Cloudflare token・admin token・custody token・受領証秘密と全組織の custody 実体を持つ (`synergos-control/src/main.rs:76-79`, `excubitor.catalog.yaml:27-28,37`, `scripts/site/run-control.mjs:21-24`) | 大容量 upload のために外へ向けるべき部品が、ネットワークと全成果物を制御する。1 バグで全面乗っ取り | high | M08 (SPEC-SY-custody-topology) |
| SEC-BR-16 | 欠落機構 | 破壊操作にブレーキが無い。custody reclaim は即 unlink、history purge と tag 削除は即時、`revoke_dark` は 1 呼出でアカウント全体 (`synergos-custody/src/handlers.rs:173-181`, `synergos-core/src/history/store.rs:427-441`, `synergos-control/src/api/reconcile_api.rs:60-61`) | 単一資格での破壊が、スタジオのランサム耐性の最弱点になる | high | M16 |
| SEC-BR-17 | 欠落機構 | quota がどこにも無い。唯一の上限は 1 実体 10 GiB (`excubitor.catalog.yaml:31-32`, `synergos-core/src/exchange/mod.rs:686-701`) | 1 アカウントの侵害がマイルストーン直前の全テナント停止になる | medium | M21 (SPEC-CU-quota-reservation) |

### 2.8 侵害シナリオ別の封じ込め評価

「残存」は再設計後に攻撃者が保持できる範囲。「Synergos の PEP」は本書の側にある強制点。

| # | シナリオ | 現状の封じ込め | Synergos の PEP | 封じ込める機構 | 再設計後の残存 |
|---|---|---|---|---|---|
| a | メンバー機 / daemon (SEC-BR-01) | なし | QUIC accept、stream 振分け、受信パス、置換 | M02, M03, M13, M14 | その機器が membership LCT を持つプロジェクトだけ、失効後 ≤60 秒。破壊的な受信は隔離され適用されない。`.git/` と `.synergos/` は書けない |
| a-2 | 同一 uid の悪性ソフト / DCC プラグイン (SEC-BR-02) | uid 内では無し | IPC (`IHELLO`) | M11, M12 | プラグイン token の read / sync 範囲。破壊的コマンドは確認 nonce か step-up が要る |
| b | ノード鍵 / WARP connector token (SEC-BR-03) | ネットワーク層だけ | enrollment、heartbeat、SESS | M04, M03, M10 | ネットワーク到達のみ。到達は会員を意味しない。会員と名簿は Cernere 署名で、鍵だけでは card も membership LCT も作れない。鍵の交換・ノードの除名は Cernere で card を再発行・失効する (control 経由の所有の変更は無い)。connector は reissue で交換 |
| c | control admin token (SEC-BR-04) | なし | control-admin | M10, M16 | 1 組織の admin 範囲で、全操作を記録。`revoke_dark` は 2 名承認。会員・enrollment・所有・到達性を変えるルートは control-admin に無く、`409 authority_is_cernere` を返す。できるのは deny だけの緊急 freeze / quarantine で、Cernere の失効を解除できない |
| d | custody token / 受領証秘密 (SEC-BR-05) | なし | custody-admin、custody-transfer | M09, M16 | Curare は受領証の**公開鍵**と delegation しか持たず、受領証を偽造できない。保持期間内は reclaim の要求だけができ、削除はできない |
| e | Curare サーバホスト (SEC-BR-06、Curare 正本) | なし、組織横断 | custody-admin (delegation 発行)、control-admin | M09, M10, M27 (+ Curare 側 M19, M22, M26) | 束縛された組織の capability を ≤1 時間減衰できるだけで、custody の償還予算の範囲内。M26 で検知。履歴の書換・ロック済み実体の削除・webhook 秘密の一括読取はできない |
| f | 盗まれた Cernere セッション (SEC-BR-07、Curare 正本) | 一部 | 失効バンドル経由で custody・daemon | M07 (+ Curare 側 M17, M26) | 流出予算まで。超えると step-up、失効まで ≤60 秒 |
| g | 履歴ノード (SEC-BR-08) | なし | stream 振分け、GC、冷蔵層 | M03, M15, M27 | 明示列挙したプロジェクトの暗号文だけ。不変複製は壊せない |
| h | Cloudflare API token (SEC-BR-09) | なし | control-admin の起動時 scope 検査 | M10, M08 | 専用の最小 scope token の範囲。`revoke_dark` は JIT broker token。token は control-admin の配備単位だけに置かれ、control-node・custody・relay には無い (M08) |
| i | hook 作者 / peer 由来 hook (SEC-BR-10) | なし | hook 実行 | M12 | sandbox の出力ディレクトリだけ |
| j | magic link 経由の外部ベンダー (SEC-BR-11、Curare 正本) | 一部 | custody-transfer (≤120 秒 LCT) | M09 (+ Curare 側 M18) | 固定した版だけ、≤72 時間、ダウンロード上限、使用時に再検査 |
| k | 供給網: org harness・action tag・build host (SEC-SC-01/02/06) | なし | Revisor ゲート、サイトでの署名検証 | M29, M30, M31 | レビュー済みの依存集合だけ。サイトは署名済み成果物を検証し、コンパイルしない |
| l | AI エージェント環境 (Codex クラウドの ephemeral ノード `tag:codex`、AGD-AI-12。Curare: `spec/feature/codex-cloud-access.md:10-16`) | tailnet ACL で本社 6443 / 4443 だけに限る (同 `:29`, `:34`)。Synergos の管理 API は tailnet に出さない (同 `:35`)。資格は人間級の Cernere token (同 `:42-43`) | custody-transfer (`/v1/custody/transfer` だけを `tailscale serve` で 4443 に公開)、Curare | M34 (暫定)、M09, M10 (+ SPEC-CR-service-principal、Curare 側 M18) | 暫定期間は、エージェント token で Curare が発行した HMAC URL (現行 ≤900 秒、再生可) を期限内に再生できる。再設計後はサービス主体の範囲と ≤300 秒・`jti` 単位の capability だけ。エージェントの書いた版が人と区別される (SPEC-CR-service-principal 提供後)。暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。 |

### 2.9 根本原因の整理

117 件は 4 つの根本原因に集約できる。

1. **識別 = 認可**: PeerId (`synergos-net/src/quic/mod.rs:459-476`)、同一 uid (`synergos-core/src/ipc_server.rs:235-241`)、静的 bearer (`synergos-control/src/auth.rs:12-28`) のどれも「誰か」を示すだけで、「何をしてよいか」を表す資格が無い。→ LCT v1 と PEP (M01〜M03, M10, M11)。
2. **共有・無期限・kid なしの秘密**: 1 つの秘密が複数用途・複数サービスにまたがる (`synergos-custody/src/handlers.rs:33-34`)。→ 用途別の鍵・kid・失効バンドル (M04, M07, M09)。
3. **破壊が即時・不可逆・無記録**: 受信置換・purge・reclaim・回転が旧バイトを残さない (`synergos-core/src/exchange/mod.rs:795`, `synergos-core/src/history/store.rs:441-471`, `synergos-custody/src/store.rs:210-221`)。→ 旧バイト保持・保持ロック・不変複製・破壊操作ブレーキ (M14〜M16, M28)。
4. **検査が人手・事後**: マージ後に走る Actions、手動 reconcile、ログ無し。→ Revisor ゲート・イベント網羅表・検知ジョブ (M25, M26, M29)。

### 2.10 他テーマとの相互参照

- 転送効率 (検証済み `spec/plan/redesign-analysis/findings/transfer-verified.json` の kept[] 79 件): TRN-DD-01 / TRN-DD-07 / TRN-DD-23 (CDC と重複排除範囲) は D-10 / D-11 の不変条件と SPEC-SY-chunk-access に縛られる。TRN-WP-21 / TRN-DD-22 / TRN-TP-16 (C++ 移行契約が版なしの wire を凍結する、ALPN が無い) は SPEC-SY-hlo2-session-binding と SPEC-SY-decode-bounds と同じ改訂を要する。TRN-TP-15 (再接続の嵐で Hello 待ちが直列) は NR-1 と同一の改修。TRN-WP-15 / TRN-DD-09 (custody の単発 PUT) は転送効率文書の SPEC-SY-custody-transfer-v2 が持ち、本書は upload_id 単位の capability (X3) で受ける。TRN-TP-07 (custody admin と transfer の分離が Curare の同一オリジン前提と衝突) は M08 と同じ方向。d404f20 の `excubitor.catalog.yaml:27` は単一の `127.0.0.1:4250` のままだが、プロキシの層では分離が既に始まっている。2026-10-23 の neco 判断 (Curare: `spec/feature/codex-cloud-access.md:10-13`、Curare ローカル main 744f013) が `tailscale serve` で `/v1/custody/transfer` だけを tailnet の 4443 → `127.0.0.1:4250` に出し、`SYNERGOS_CUSTODY_PUBLIC_URL` をその URL にする。これは D-07 の選択肢 B (listener 分離) の事実上の暫定形で、M34 で扱う。
- AI・ゲーム開発 (検証済み `spec/plan/redesign-analysis/findings/ai-game-verified.json` の kept[] 77 件。repo 別は curare 17・both 47・synergos 13): 本書の機構に直接かかるのは次の 7 件。すべて検証済みの相互参照として扱い、正本の行は `spec/plan/redesign-ai-game-dev.md` (Synergos) と Curare: `spec/plan/redesign-ai-game-dev.md` が持つ。

| ID | 要旨 | 本書の受け口 |
|---|---|---|
| AGD-AI-11 (curare) | 取込時の検査・隔離が無い。upload がすぐ配信される版になる | M24 (展開ゲート)、M12 (解析器の sandbox) |
| AGD-AI-12 (curare) | 機械・エージェント主体が無い。Codex クラウドのエージェントも人間級の長命 Cernere token を持つ (Curare: `spec/feature/codex-cloud-access.md:42-43`) | SPEC-CR-service-principal の追加提案 (§4.34、Q7) の根拠。§2.8 l、M34 の Codex 経路の行 |
| AGD-GW-13 (curare) | 外注の往復が半分しか無く、パイプライン・レンダーファームにサービス主体が無い | SPEC-CR-service-principal、Curare 側 M18 (magic link v2) |
| AGD-AI-16 (both) | 履歴ノードが任意の版を任意の接続 peer に返す (`synergos-core/src/exchange/mod.rs:959-990`、`docs/versioning-design.md:101`) | M03 (history 役割の会員資格ゲート) |
| AGD-OP-09 (both) | SaaS のテナントが第一級でない。全組織が 1 つの保管ノード・鍵空間・DB を共有し、quota も無い | M27 (プロジェクト KEK と保管名)、SEC-BR-13、M21 (quota) |
| AGD-AI-08 (both) | 学習クラスタ向けの取得面が無い。実体ごとの 300 秒 capability URL で全体を逐次取得する | E2 (集合スコープの capability、≤1000 実体を 1 呼出)、M09 (`object_keys[]` / `prefix` caveat で全範囲を 1 capability が許す) |
| AGD-DM-08 (both) | 保持の正本が宣言されていない。Curare は保持方針の正本を名乗り、Synergos は node-local の tag と版下限を持つ (`spec/feature/opaque-custody.md:42` を含む) | M15・M16 と D-25 (custody の `retain_until` は Curare が設定し、Synergos は強制だけを行う) |

## 3. 設計判断と選択肢の比較

評価は HARNESS §3.5 の 4 軸。各 1〜5 点で 5 が最良 (AI 学習量が少ない / 作業コストが小さい / 解決度が高い / 主目的との一致度が高い)。**[変更提案]** は既存の neco 決定や既存 spec を変える判断で、neco の再判断事項 (Q1) とする。Curare 文書は同じ印を [CHALLENGE] と書いている。印は [変更提案] に統一するよう Curare 文書へ申し送る。D 番号は両 repo の文書で共通。Curare だけに関わる D-18 (一括取得) と D-20 (Curare 認証の実装時期) は Curare: spec/plan/redesign-security.md が持つ。D-21〜D-25 は本改訂で足した Synergos 側の判断。

### 3.1 D-01 資格・capability の形式 (SEC-AZ-01, SEC-AZ-13, SEC-BR-05, SEC-CR-05)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. HMAC URL を維持し nonce と kid を足す (`spec/plan/redesign-analysis/HANDOFF.md` §4.2) | 5 | 5 | 1 (検証者が発行秘密を持ち続ける) | 1 |
| B. Cernere を根にした Biscuit (`spec/plan/redesign-analysis/research/breach-containment.md` §8 P0-1) | 2 (Datalog) | 1 (保守された C++ 実装が無く Rust FFI が要る (推定)。C++ 最小依存の方針に反する) | 5 | 3 |
| C. peer 用の小型 Ed25519 MembershipCert + custody 発行の HMAC capability v2 | 4 | 3 | 4 (形式が 2 つ、実体ごとに custody 呼出) | 4 |
| **D. LCT v1: 連鎖 Ed25519 block、JCS payload、閉じた caveat 語彙、積集合意味論** | 3 | 3 | 5 | 5 |
| E. Curare が要求ごとに JWT を発行 | 5 | 4 | 2 (減衰できず、Curare の鍵が全組織分を発行する) | 2 |

**採用: D。** C++ と TS に codec が 1 つずつ、オフラインで減衰でき、共有秘密が無い。C の MembershipCert は LCT の単一 block の membership profile として吸収する。B は保守された C++ 検証器が現れた場合の代替として文書に残す。

### 3.2 D-02 発行鍵の根

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. Cernere が全 token に署名 | 3 | 2 (Cernere の大改修) | 4 (P2P 経路に Cernere の実行時依存) | 3 |
| B. PDP 側の発行者 (membership は control、delegation は custody)、鍵は Vault に pin するだけ | 4 | 4 | 3 (発行鍵の組織単位の証明が無い) | 4 |
| **C. PDP 側の発行者の公開鍵を Cernere の鍵委任記録 (SPEC-CR-capability-signing-keys) で認証 (custody-delegation・受領証・失効 (deny だけ) に限る。membership と roster は Cernere 自身の鍵、オーナー決定 8)** | 3 | 3 | 5 | 5 |

**採用: C (範囲を狭める、オーナー決定 8)。** C は custody-delegation・受領証・失効 (deny だけ) の鍵に限って残す。membership と roster (会員証・node card・ProjectRoster・RosterStatus・OfflineLease) は C ではなく、Cernere 自身の `purpose: membership` / `purpose: roster` の kid で署名する (SPEC-CR-membership-issuer、SPEC-CR-node-roster)。membership について「暫定は B (Vault に pin した control の発行鍵)」は撤回し、synergos-control はどのフェーズでも membership・roster の発行鍵を持たない。オーナー決定 4 (Cernere が署名鍵を公開する) を満たし、PEP の検証はローカルのまま (要求ごとの往復が無い。membership LCT の更新は 50 % 経過時に heartbeat 経由で Cernere へ取り次ぐ。control や Cernere に届かない間の扱いは D-21)。2026-09-08 境界と決定 8 の締め直しに沿う: Curare は版・承認の PDP、Cernere は本人・所属とプロジェクト会員・ノード名簿の PDP、Synergos は強制だけを行う。P2P だけのプロジェクトも含めて会員資格の権威は Cernere (D-19)。

### 3.3 D-03 peer セッション認証 (SEC-CR-01, SEC-AZ-02, SEC-CR-13, SEC-DA-01)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. HLO2: 片方向 TLS 上で exporter に束縛した署名 Hello (`spec/plan/redesign-analysis/HANDOFF.md` §4.1) | 4 | 4 | 3 (識別層が二重、最初の stream 待ちが残る) | 3 |
| **B. 自己署名 ed25519 クライアント証明書を双方向 pin する mTLS、ALPN `synergos/2`、最初の frame SESS に membership LCT を載せ、その `peer_id` caveat を TLS の PeerId と一致させる** | 3 | 3 | 5 | 5 |
| C. control 発行の X.509 SVID (CA を置く) | 2 | 2 | 5 | 3 |

**採用: B を契約 ID SPEC-SY-hlo2-session-binding として。** ID はオーナーの「HLO2」の名を引き継ぐ。HLO1 は型付き close code で拒否する。A の exporter 署名 Hello は、クライアント証明書を運べない経路が現れた場合の代替として残す (現時点で該当は無い。relay は QUIC セッションを端から端まで中継するため)。**互換契約の変更**である。`cpp-migration.md:39-40` は「preserve … QUIC ALPN, peer authentication」と書くが、現行に ALPN は存在しない (`synergos-net/src/quic/mod.rs:543-546`, SEC-CR-13) ので、この行は「維持」ではなく「置換」になる。

### 3.4 D-04 署名対象の正規化 (SEC-CR-12)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| **A. RFC 8785 JCS、整数のみ・浮動小数なし** | 4 (現行の鍵整列 JSON に近い。`cpp-migration.md:85-86` は受領証で浮動小数を既に拒否) | 4 | 5 (仕様 1 つと golden vector) | 5 |
| B. 決定的 CBOR (RFC 8949 §4.2) | 3 | 3 (C++ と TS に CBOR ライブラリを新設) | 5 | 4 (token が小さい) |

**採用: A。** ブラウザ GET で URL 長が問題になる場合 (3 block で約 1.5 KB と推定)、token の wire 形式に限って CBOR を代替とする。

### 3.5 D-05 custody capability の発行方式 (SEC-AZ-13, SEC-DA-13, SEC-BR-06)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. custody が admin API で capability を 1 件ずつ発行 | 4 | 3 | 4 | 2 (N 実体の clone で admin 呼出 N 回。E2 を破る) |
| **B. custody は認証済みの Curare 主体へ ≤1 時間の delegation LCT を発行し、Curare が要求ごとにローカルで減衰** | 3 | 3 | 5 | 5 |

**採用: B。** A が狙った記録と予算は移す: 発行記録は SPEC-CU-capability-ledger、償還記録は SPEC-SY-custody-journal、台帳に無い jti の償還は `capability.orphan` 警報、レート制限は custody 側の delegation 単位・`sub` 単位の償還予算。

### 3.6 D-06 受領証の署名 (SEC-CR-04, SEC-CR-12) [変更提案]

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 共有 HMAC を維持 (現行 SPEC-SY-opaque-custody) | 5 | 5 | 1 (Curare が受領証を偽造できる) | 1 |
| B. HKDF で用途別鍵を導出、ただし共有のまま | 4 | 4 | 2 | 2 |
| **C. custody インスタンスごとの Ed25519、kid、JCS バイト。Curare は公開鍵だけを持ち、バイト・署名・kid をそのまま保存** | 3 | 3 | 5 | 5 |

**採用: C。** 変更する既存記述: `spec/feature/opaque-custody.md:12` 「HMAC署名URLだけを渡す」、`:17` 「HMAC-SHA256を計算する」、`:21` 「signature.rsのcanonical/sign/verifyはCuと共有する署名表現」、`:53` 「Cuと共有する32文字以上の署名鍵」、`spec/plan/cpp-migration.md:45-46` 「Preserve canonical receipt bytes, HMAC」。所有境界 (Curare が版・許可を持ち、Synergos は不透明実体を運ぶ) は変えない。変わるのは署名方式だけ。v1 の HMAC 受領証は保存済みの行に限り検証専用で残す。

### 3.7 D-07 custody と control の配置 (SEC-BR-15, SEC-CR-06) [変更提案]

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 1 プロセス 1 listener (`spec/feature/opaque-custody.md:9`「別listenerを起動しない」) | 5 | 5 | 1 | 2 |
| B. 1 プロセスで listener を分ける (公開 transfer と非公開 admin) | 4 | 4 | 3 | 3 |
| **C. 配備単位と OS ユーザーを分離 (`custody-transfer`、`custody-admin`、`control-admin`、`control-node`、`relay`)、それぞれ別の Vault policy** | 3 | 3 | 5 | 5 |

**採用: C。C++ の最初の段階として B を許す。** endpoint は C++ で書き直すので、分離の追加費用は小さい。B は既にプロキシの層で事実上始まっている。2026-10-23 の neco 判断 (Curare: `spec/feature/codex-cloud-access.md:10-13`, `:34-36`) は `tailscale serve` で `/v1/custody/transfer` だけを tailnet の 4443 に出し、管理 API (`/v1/custody/*` の受け渡し以外、`/v1/orgs` など) は tailnet にも出さない。プロセスは 1 つのままでも、到達できる経路はパスで分かれている。これを B の暫定形として扱い、運用上の補償を M34 に置く。

### 3.8 D-08 失効の伝播 (SEC-AZ-17, SEC-BR-12, SEC-DR-07, SEC-CR-09)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 短い TTL だけ | 5 | 5 | 2 | 2 |
| **B. 署名付きの単調増加バンドルを 30 秒ごとに取得し、300 秒を超えて古ければ fail-closed。control と Curare に SSF/CAEP 受信。TTL 上限 (G2)** | 3 | 3 | 5 | 5 |
| C. 要求ごとのオンライン照会 | 4 | 3 | 5 | 1 (stream・チャンクごとに 1 RTT 増えて E1 を破り、control の瞬断がそのまま全転送の停止になる) |
| D. gossip による配布 | 3 | 3 | 1 (認可の無い mesh が配布を握りつぶせる) | 1 |

**採用: B。** 30 秒取得で G1 (p99 60 秒) に余裕を残す。B も control に届かない状態では新規許可を止める (fail-closed)。C との違いは、止まるまでに 300 秒 (新規セッション) と LCT の `exp` (既存セッション) の猶予があり、その間の各要求に往復が無いことである。オフライン拠点と control 障害の扱い、および可用性の目標は D-21 と G11 で決める。会員とノードの失効の正本は Cernere 署名の RosterStatus で、バンドルはそれをそのまま埋め込み、control 自身の署名部分は deny だけを足す (SPEC-SY-revocation-bundle、オーナー決定 8)。

### 3.9 D-09 保持と不変性 (SEC-IN-01/05/06/09, SEC-DR-09/17) [変更提案]

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. ローカルのごみ箱だけ | 5 | 4 | 2 | 2 |
| **B. 版ごとの Object-Lock 意味論 (`retain_until`、governance / compliance、`legal_hold`)、置換旧バイトの保管、put 専用 ID が書く不変複製、月次訓練** | 3 | 2 | 5 | 5 |
| C. 外部のバックアップ製品だけ | 4 | 4 | 3 (プロトコル上の保持受領が無い) | 3 |

**採用: B。** 2026-08-16 の履歴ノードフラグ方式 (`docs/versioning-design.md:25`「D. 履歴ノードフラグ (採用)」、`:111`「既定は無効」) に対する変更提案になる。再設計では、auto-pull 同期モードのプロジェクトは保持可能ノードからの `RetentionAck` を 1 件以上必要とする (SPEC-SY-retention-ack)。保持は「任意のノードフラグ」から「同期モードの義務」に変わる。2026-08-16 決定のうち「全体転送・CDC なし」の部分には本書は**触れない** (転送効率文書が再検討する。オーナー決定 3)。

### 3.10 D-10 保管時暗号化 (SEC-CR-19, SEC-BR-13)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 暗号化なし、またはボリューム暗号化だけ | 5 | 5 | 1 | 1 |
| B. blob ごとのランダム DEK をプロジェクト KEK で包む | 3 | 2 | 5 | 3 (版をまたぐチャンク重複排除が効かず E5 を破る) |
| **C. プロジェクト単位の封筒: KEK は Excubitor Vault Transit、実体全体の DEK を KEK で包む、チャンク DEK = HKDF(project_chunk_key, chunk_blake3) (プロジェクト内で収束)、AD は `{org, project, blake3, size}`、保管名は HMAC(project_key, blake3)** | 3 | 2 | 5 | 5 |
| D. 組織をまたぐ大域収束暗号 | 2 | 2 | 1 (テナント間の存在オラクル) | 2 |

**採用: C。** 重複排除範囲は既定でプロジェクト。組織範囲は組織ごとの opt-in で、組織内プロジェクト間の存在オラクルを代償とする (neco 判断、Q6)。大域範囲は採らない。

### 3.11 D-11 CDC / チャンク保管のセキュリティ観点 (オーナー決定 3。配置の判断は転送効率文書の D-T03)

| 選択肢 (セキュリティ観点) | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. チャンク索引と保管を Synergos daemon と custody の内部に置く | 3 | 3 | 3 (チャンク索引が custody admin とプロセス・資格を共有する) | 3 |
| **B. 狭い PEP の裏の独立チャンク保管モジュール: `(org, project_ref, chunk_id)` で put / get、LCT 必須、プロジェクト横断の一覧なし、発行者の役割なし** | 3 | 3 | 5 | 4 |

セキュリティ観点では B を推した。転送効率文書は D-T03 で 4 軸の比較を行い、B (`synergos-chunkstore`、独自の CMake target とドメイン `chunk-store`、seed / archive / custody / edge だけがプロセス内で link し、後で同じ API のまま別プロセスにできる) を採用した。よって **SPEC-SY-chunk-access は条件付きではなく必須の契約**になる (追補 X5)。モジュールの API は転送効率文書の SPEC-SY-chunk-store が持ち、本書はその境界で次の不変条件を持つ:

1. 重複排除範囲はプロジェクト (組織は opt-in のみ)。大域にしない。
2. チャンクの読取はすべて、manifest か実体集合を caveat に名指しした LCT で認可する。
3. wire と一覧に出るチャンク ID は `HMAC(project_dedup_key, blake3)`。
4. チャンク記録は包んだ鍵への参照を持ち、チャンク DEK は D-10 に従う。
5. チャンクの GC は保持ロック (M15) を守り、別の ID で動く。
6. 配信ごとに `content.served` イベントを出す。

これらはモジュール境界の契約 SPEC-SY-chunk-access として §4.24 に書く。

### 3.12 D-12 供給網検査の実行場所 (SEC-SC-01/02/03)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. GitHub Actions を堅牢化 (SHA pin、`permissions`) | 4 | 4 | 2 (Revisor のマージ後に走る) | 1 (RULE.md §6.5 に反する) |
| **B. repo 所有の `scripts/gate/*.mjs` を Revisor のマージ前ケースにし、`.github/workflows/*` を削除** | 4 | 3 | 5 | 5 |
| C. 両方 | 3 | 2 | 5 | 2 (正本が二重) |

**採用: B。** RULE.md §6.5 は「GitHub CI / GitHub Actions の workflow は置かない」。Synergos には `ci.yml`・`harness.yml`・`native.yml` がある。SEC-SC-02 (AIFormat の再利用 workflow) は workflow 削除で消える。

### 3.13 D-13 quick-xml 0.30 (RUSTSEC-2026-0194/0195、SEC-SC-05、オーナー決定 5)

経路は `Cargo.lock` で確認した。`quick-xml 0.30.0` を使うのは `zbus_xml 4.0.0` だけ (`Cargo.lock:9374-9379`)。逆依存を辿ると `zbus_xml` ← `zbus-lockstep 0.4.4` ← `atspi-common 0.6.0` ← `accesskit_atspi_common 0.10.1` ← `accesskit_unix 0.13.1` ← `accesskit_winit 0.23.1` ← `egui-winit 0.31.1` ← `eframe 0.31.1` ← `synergos-gui 0.1.0` で、根は `synergos-gui` だけ。`synergos-gui/Cargo.toml` は `eframe = "0.31"` を既定 feature で使い、eframe 0.31.1 の既定 feature に `accesskit` が入っている (eframe 0.31.1 の `Cargo.toml` の `[features] default`)。`egui-winit` が `accesskit_winit` を引くのはこの feature のときだけで、テスト用の `egui_kittest 0.31.1` は `egui` と `kittest` だけに依存する (`Cargo.lock`)。workspace の members は `synergos-gui` と `synergos-tauri/src-tauri` の両方を含む (`Cargo.toml:2-12`)。`cargo tree -i` による確認は、offline 実行で一部 crate の取得ができず未確認 (lockfile と eframe の Cargo.toml の走査で代替した)。

依存は要るか: C++ 移行は `desktop-gui` を移植対象のモジュールに挙げ (`spec/plan/cpp-migration.md:24-27`, 手順 5 `:58`)、egui GUI は C++ の desktop-gui に置き換わる。ただし置き換わるまでは、`cpp-migration.md:48`「Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces.」により egui GUI は受入面で、parity の基準でもある。README (`README.md:18`, `:154`)、`docs/getting-started.md:18`、`docs/setup-macos.md:298`、`docs/platforms.md:121` も `synergos-gui` を GUI アプリとして案内している。暫定のデスクトップ面を `synergos-tauri` とする根拠は弱い: `docs/operations-readiness.md:36` は「GUI (egui) / Tauri」を同じ状態として並べ、`synergos-tauri/README.md:3` は自らを「Synergos の … ベース GUI」と書くだけである。したがって「Tauri が暫定面」は**推定**として扱い、Q3 で確かめる。

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. `audit.toml` で除外 | 5 | 5 | 1 | 1 (決定 5 で却下) |
| B. 赤のまま許容 | 5 | 5 | 1 | 1 (却下) |
| C. eframe を `default-features = false` にし、`accesskit` 以外の既定 feature (`default_fonts`, `glow`, `wayland`, `x11`) だけを明示する | 4 | 4 | 4 (経路は消える。Linux のスクリーンリーダー対応を失う) | 3 |
| D. P0 で `synergos-gui` を workspace から削除する | 4 | 5 | 5 | 2 (C++ desktop-gui がまだ無い時点で、`cpp-migration.md:48` の受入面と parity の基準を消す) |
| **E. P0 で Cargo を `server/` と `desktop/` の workspace に分け、`synergos-gui` を `desktop/` へ移して C を適用する。egui バイナリは C++ desktop-gui が受入ゲートを通るまで `desktop/` で build できる状態に保ち、その時点で撤去する** | 4 | 4 | 5 | 5 |

**採用: E。** サーバ成果物の依存グラフから GUI 系 crate が消え、desktop workspace からも `quick-xml 0.30` の経路が消える (C の効果。feature 解決後の消滅は `cargo tree -e features -i quick-xml` で P0 に確かめる)。C の後も経路が残ると分かった場合は、egui バイナリを配布物から外して parity の基準としてだけ build し、`desktop/` の audit に赤のまま残す (除外はしない)。Q3 で egui GUI の利用者がいないと分かった場合は、C++ desktop-gui を待たずに撤去してよい。`cpp-migration.md:48` の「native GUI」を「C++ desktop-gui (暫定は egui と Tauri)」に改める改訂を §4.37 に [変更提案] として載せる。これは workspace と設定の変更で、決定 6 で禁じた 3 件の実装項目には当たらない。

### 3.14 D-14 hook・解析器・変換器の sandbox (SEC-DA-06, SEC-AZ-10, SEC-BR-10, SEC-DA-20)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 全環境で gVisor | 2 | 2 | 3 (デスクトップに無い) | 3 |
| B. bubblewrap + seccomp だけ | 3 | 3 | 2 (Linux のみ) | 3 |
| **C. 1 つの SPEC-SY-sandbox-profile 契約と OS 別 backend: Linux は gVisor、無ければ bubblewrap + seccomp。macOS は App Sandbox。Windows は AppContainer + Job。backend が無ければ hook / 解析器を拒否** | 3 | 3 | 5 | 5 |

**採用: C。**

### 3.15 D-15 取込検査の場所 (SEC-DA-20)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. Curare だけ | 4 | 4 | 2 (P2P だけのプロジェクトが無防備) | 3 |
| B. Synergos だけ | 3 | 3 | 2 (Curare が未検査バイトを配信し、判定が所有者の外に出る) | 1 |
| **C. 判定は Curare が持つ (SPEC-CU-ingest-verdict)。Synergos は運ばれた判定と局所の展開ゲートを強制 (実行ビットを立てない。`clean` の無いスクリプト可能拡張子は隔離)** | 3 | 3 | 5 | 5 |

**採用: C。** 2026-09-08 の「判定は Curare が持つ」を守る。

### 3.16 D-16 検知パイプライン、D-17 濫用制限、D-19 データプレーンの会員資格の権威

| 判断 | 選択肢 (学習量 / 作業 / 解決 / 一致) | 採用 |
|---|---|---|
| D-16 検知 (SEC-DR-14) | ローカルログだけ (1/5/1/1)、**共有スキーマ + ハッシュ連鎖ジャーナル + Vestigium + Concordia 規則 (4/3/4/5)**、外部 SIEM (2/2/4/2) | 中央案。新規ツールが最少で RULE_SRE に沿う。外部 SIEM は企業顧客向けの代替 |
| D-17 濫用制限 (SEC-DA-22) | ハンドラごとの定数 (5/4/2/2)、**宣言的 AbusePolicy 表 + 登録テスト (3/3/5/5)**、WAF だけ (4/4/2/2) | 表。ゲートで検証できる唯一の案 |
| D-19 会員資格の権威 (SEC-AZ-01, SEC-AZ-03) | ホスト daemon の招待台帳 (4/4/2/2)、control のレジストリが membership LCT を発行する (3/3/5/4、オーナー決定 8 で不採用)、**Cernere が全プロジェクトの会員とノード名簿の正本で、membership LCT・node card・ProjectRoster・RosterStatus・OfflineLease を署名する。control は配布・キャッシュ・検証だけ (3/3/5/5)**、ストリームごとに Curare へ照会 (3/3/4/1、E1 を破り、P2P だけのプロジェクトには使えない) | Cernere (オーナー決定 8) |

D-19 の補足 (オーナー決定 8 で置換)。権威は Curare 紐付けかどうかによらず、すべてのプロジェクトで Cernere である (SPEC-CR-membership-issuer、SPEC-CR-node-roster)。

- **全プロジェクト共通**: Cernere が `synergos_projects` と `synergos_memberships` を持ち、membership LCT の block 0 を ctx `membership` で署名する。`project_policy{invite_max_role, require_approval, offline_lease_allowed}` も Cernere に置く。synergos-control は membership・roster の発行鍵をどのフェーズでも持たず、会員・所有・到達性を決めない。control ができるのは配布・キャッシュ・検証と、deny だけの緊急 freeze / quarantine で、Cernere の失効を解除できない。
- **Curare 紐付けプロジェクト**: Synergos のプロジェクト会員の正本は Cernere (SPEC-CR-membership-issuer)。Curare は会員を輸出・発行・判定しない。Curare 紐付けの scope では、作成者の権限で `project_ref` を Cernere に登録し、Curare の UI での会員変更は操作者本人として Cernere へ取り次ぐだけ (SPEC-CU-membership-export の改訂後の定義)。
- **P2P だけのプロジェクト**: 所有者が Cernere にプロジェクトを登録し、招待 (SPEC-SY-invite-v2) は host から control 経由で Cernere の `member-requests` へ取り次がれ、Cernere がプロジェクトの方針で判定する。
- 2026-09-08 の境界 (`spec/feature/opaque-custody.md:3-5`) は、決定 8・9 で「Cernere は本人・組織・所属に加えて Synergos のプロジェクト会員とノード名簿の正本」と締め直した (§1.1)。Synergos 側に方針の権威を置かないので、旧 [変更提案] D-19 (i) (control に `project_policy` を置く) は撤回し、(ii) (全プロジェクトを Curare 紐付けにする) は論点でなくなった。

### 3.17 D-21 control に届かない間の振る舞い (失効の鮮度とオフライン運用)

D-08 の採用案 (B) も、control に届かない状態では新規の許可を止める。LAN だけの拠点、外部と切れた拠点、control の障害では、何もしなければ 300 秒で新規セッションと償還が止まり、最長 3600 秒 (membership LCT の寿命) で既存セッションも終わる。control-node はデータプレーン全体の可用性の依存先になる。

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 厳格な fail-closed だけ (新規 300 秒・既存は LCT の `exp`) | 5 | 5 | 3 (失効は確実。control の障害と分断で全拠点が止まる) | 2 |
| B. control-node の多重化 (別ホスト・別拠点に 2 インスタンス以上。署名鍵は Excubitor Vault Transit の同じ鍵を各インスタンスが使い、入力 journal は複製する) | 4 | 3 | 3 (control の障害は吸収する。拠点の分断は救えない) | 4 |
| C. プロジェクトごとの署名付きオフライン貸与: `OfflineLease{project_ref, site_id, site_cidrs[], peers[{peer_id, role}], issued_at, exp ≤ 72 h, max_stale ≤ 24 h}` を Cernere が ctx `project-roster` で署名し (プロジェクトの所有者の step-up が発行条件、Cernere の `offline_lease_allowed` が真のときだけ。列挙できるのは現会員を、その役割以下でだけ。SPEC-CR-membership-issuer)、control は配布するだけ。貸与中、拠点内のアドレスから来た列挙 peer に限り、古いバンドルと期限切れ LCT のもとでも CapSet を与える | 3 | 3 | 4 (分断中も拠点内で作業が続く。失効の遅れは貸与の窓に限られ、受け入れが記録される) | 4 |
| D. 失効バンドルの寿命を全体で延ばす (例: 24 時間) | 5 | 5 | 1 (すべての失効が最長 24 時間遅れる) | 1 |

**採用: A を既定とし、B を必須 (G11)、C をプロジェクトごとの opt-in にする。** 貸与は Cernere 署名 (オーナー決定 8)。B の多重化は control の失効 (deny だけ) 鍵と配布の可用性だけを担い、会員・名簿の鍵は持たないので、Cernere 自体に届かない間は membership LCT の更新・enrollment・名簿の変更が止まる (G11 の注記)。 C の窓の上限 (`max_stale` ≤24 時間、`exp` ≤72 時間) と、受け入れの承認者は Q-SY-6 で決める。負けた案の残存リスク: D はすべての失効を遅らせる。A だけでは control の障害がそのまま全拠点の停止になる。C を受け入れたプロジェクトでは、失効済み peer が貸与に列挙されている限り、その拠点内で `max_stale` まで残る。この残存は G1 の例外として明記する。

### 3.18 D-22 C++ への切替: 組織ごとの flag day か、期限付きの二重スタックか

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| **A. 組織ごとの flag day。同じ組織の中で Rust ノードと C++ ノードを混在させない (HLO1 と `synergos/2` は相互接続しない)** | 4 | 3 | 5 | 4 |
| B. 期限付きの二重スタック: C++ ノードが一定期間 HLO1 も受け、Rust ノードと相互接続する | 3 | 2 (2 つの session・framing・認可を同時に保守する) | 2 (窓の間、SEC-CR-01 の再生で会員資格ゲートを素通りできる。M03 が意味を失う) | 3 |
| C. 二重スタックで、HLO1 側は mesh 範囲と読取専用に限る | 3 | 2 | 3 (再生した HLO1 で読取は続く) | 3 |

**採用: A。** 転送効率文書も peer wire の切替 (TXF2) を同じ flag day に載せる。残存リスク: 切替日に更新できないノード (オフラインの外注機など) は更新まで接続できない。戻す場合は「その組織を M34 の設定付きで Rust 実行時に留める」で、プロトコルの後退は行わない。B を採らない理由は、窓の間 critical の SEC-CR-01 をそのまま残すからである。

### 3.19 D-23 DHT: 削除か堅牢化か (SEC-AZ-05、Q4)

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| **A. 削除する。発見は control が配布する Cernere 署名の node card の directory (転送効率文書の SPEC-SY-directory、「DHT1 is not ported」。control は directory 自体に署名せず、各 card と ProjectRoster が個別に検証される) と mDNS の拠点内 beacon が担う** | 5 | 5 | 5 | 4 |
| B. 残して、Announce を membership LCT 付きの署名記録に限り、FindProjectPeers を会員だけに答える | 3 | 2 (本番で休眠している機能を C++ へ移して保守する) | 4 | 2 |

**採用: A (Q4 の既定)。** 残存リスク: control を経ないインターネット規模の発見は無くなる。将来必要になれば、B を新しい契約として作り直す。B を採らない理由は、使われていない攻撃面を C++ へ持ち込むからである。

### 3.20 D-24 署名封筒・LCT codec の置き場所 (`cpp-migration.md:79-80`)

`spec/plan/cpp-migration.md:79-80` は「C++ source is placed in existing module membership rather than introducing a cross-domain directory.」と定める。M01 の署名封筒・LCT・JCS は network-transport・mesh-core・opaque-custody・control-plane のすべてが使う。

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 新ドメイン `trust-primitives` と新ディレクトリ (例 `synergos-trust/native/`) を設ける | 3 | 3 | 5 | 3 (`:79-80` に反し、neco の再判断が要る) |
| **B. network-transport の `synergos-net/native/trust/` に library target として置き、mesh-core・opaque-custody・control-plane の C++ はそこを link する** | 4 | 4 | 4 (custody と control が net の library に依存する向きが 1 本増える。現行の `synergos-custody/Cargo.toml` は synergos-net に依存しない) | 5 |

**採用: B。** 既存の決定の範囲に収まり、変更提案にしない。署名は peer セッション・gossip・招待・node card など network-transport の利用が最も多い。依存の向きの増加は、trust library が net の他の部分 (QUIC、gossip) を引かない独立 target であることで抑える。A は、custody の build が net 全体を引くと分かった場合の代替として残す (Q-SY-4)。

### 3.21 D-25 custody の保持の所有者 (`opaque-custody.md:41-42`、AGD-DM-08)

`spec/feature/opaque-custody.md:41-42` は「Cuが参照のない未確定実体だけをreclaimする。Syは永続tombstoneを先に記録して削除し…Cuの業務保持参照をSy独自ポリシーで無視しない。」と定め、Curare: `spec/plan/versioning-types.md:8` は Curare を保持ポリシーの正本とする。

| 選択肢 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. custody の保管層に Synergos の既定保持ロック (30 日) を置き、reclaim は要求だけにする | 4 | 4 | 5 | 2 (Synergos 独自の保持方針になり、境界を変える) |
| **B. `retain_until`・`mode`・`legal_hold` は Curare が admission と commit で設定し、custody は強制だけを行う。reclaim の tombstone 期間も Curare 所有の SPEC-CU-destructive-op の値 (既定 30 日、下限 7 日) に従う。Synergos 独自の既定 30 日は P2P の履歴ノードだけに使う** | 4 | 4 | 4 (Curare が短すぎる値を送っても、custody は SPEC-CU-destructive-op の下限 7 日より短い値を拒否するだけ) | 5 |

**採用: B。** 保持の所有者は Curare のまま変わらない。変わるのは `:41-42` の削除手順の文言 (即時削除 → `retain_until` 付き tombstone と別 ID の GC) で、これは §3.22 の [変更提案] に入れる。

### 3.22 既存決定への変更提案の一覧 (Q1)

| 変更提案 | 既存記述 | 根拠 |
|---|---|---|
| 受領証を HMAC から Ed25519 へ (D-06) | `spec/feature/opaque-custody.md:12,17,21,53`、`spec/plan/cpp-migration.md:45-46` | SEC-CR-04, SEC-BR-05 |
| custody / control の配備分離 (D-07) | `spec/feature/opaque-custody.md:9`「別listenerを起動しない」 | SEC-BR-15, SEC-CR-06 |
| 保持を同期モードの義務に (D-09、SPEC-SY-retention-ack) | `docs/versioning-design.md:25,111` (2026-08-16 の履歴ノードフラグ、既定無効) | SEC-IN-01, SEC-DR-09 |
| ALPN と peer 認証の置換 (D-03) | `spec/plan/cpp-migration.md:39-40`「preserve … QUIC ALPN, peer authentication」 | SEC-CR-01, SEC-CR-13 |
| 局所 IPC 認可の置換 (SECURITY-M11) | `spec/plan/cpp-migration.md:38`「Local IPC authorization: same-user checks」 | SEC-AZ-09, SEC-BR-02 |
| GET URL の最大寿命 900 秒 → capability ≤300 秒 (ブラウザ URL 形式 ≤120 秒) (M09) | `spec/feature/opaque-custody.md:37`「最大900秒のGET URLを要求する」。実装は `synergos-custody/src/handlers.rs:153` (`1..=900`) と `:98` (PUT URL も 900 秒上限) | SEC-CR-05, G2 |
| 取得名の束縛を LCT の caveat `download_name` で保つ (M09) | `spec/feature/opaque-custody.md:39`「URL署名はHTTPメソッド・実体キー・期限・取得名へ結び付ける」。現行は `name` を署名対象に含める (`synergos-custody/src/handlers.rs:184-194`) | 閉じた caveat 語彙に `download_name` (GET だけ、安全な ASCII basename) を足し、`Content-Disposition` の値をこの caveat だけから作る。束縛は落とさない |
| 中断時の再送: 実体全体の再 PUT → バイト位置からの再開 (転送効率文書の SPEC-SY-custody-transfer-v2。本書は capability を upload_id 単位にする追補 X3 だけを持つ) | `spec/feature/opaque-custody.md:40`「バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送」 | TRN-WP-15, TRN-DD-09。100 GB の upload は 300 秒の capability より長い |
| reclaim の手順: tombstone を書いて即時削除 → `retain_until` 付き tombstone と別 ID の GC (保持の所有者は Curare のまま、D-25) | `spec/feature/opaque-custody.md:41-42` | SEC-IN-10, SEC-BR-16, AGD-DM-08 |
| 受入面の「native GUI」を C++ desktop-gui に改める (暫定は egui と Tauri、D-13) | `spec/plan/cpp-migration.md:48`「Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces.」 | SEC-SC-05, オーナー決定 5 |
| IPC の command / response / event の「維持」を「維持 + 追加」に改める (`IHELLO` を最初の frame に、scope、破壊的コマンドの 2 段階) (M11) | `spec/plan/cpp-migration.md:35-37`「Preserve command, response and event variants」 | SEC-AZ-09, SEC-BR-02 |
| 版管理の「restore/checkout semantics, tags and protected history の維持」を改める: checkout は job (転送効率文書の SPEC-SY-ipc-v2)、tag 削除と purge は SPEC-CU-destructive-op の tombstone、保持は SPEC-SY-retention-lock (M14, M15, M16) | `spec/plan/cpp-migration.md:41-43` | SEC-IN-05, SEC-IN-15, SEC-BR-16 |
| Infisical 記述の訂正 (変更提案ではなく誤記訂正) | `spec/feature/opaque-custody.md:58`「設定はInfisical/Excubitorの注入を使用する」 | Infisical は廃止済み。秘密の取得元は Excubitor Vault だけ |
| 「protobuf schemas」の訂正 (変更提案ではなく誤記訂正。正本は転送効率文書の SPEC-SY-peer-wire) | `spec/plan/cpp-migration.md:39`「preserve protobuf schemas」 | 現行の peer wire は手組みの MessagePack / JSON 形式で protobuf ではない (SEC-DA-19 の 6 形式) |

`spec/plan/cpp-migration.md:79-80` (cross-domain directory を作らない) は D-24 で守るため、変更提案に入れない。

## 4. 再設計の内容

### 4.0 読み方

- 各機構の見出しは SECURITY-Mxx (両 repo 共通の機構 ID)。機構ごとの正本と相手側の役割は §4.0.1 の表に置く (Curare 文書 §4.0 の表と同じ列)。
- **区分** は `spec/plan/redesign-analysis/HANDOFF.md` §6 の区別に従い、Curare 文書と同じ記号を使う。**C** = 契約の変更 (C++ 移行の互換契約 `cpp-migration.md` "Compatibility contracts" と `native-contracts.md` の `[id:]`、または repo をまたぐ SPEC id を変える・足す)。**I** = 実装要件 (契約は変えず、C++ 実装が最初から避ける欠陥・非効率。NR-x)。**O** = 運用・設定 (配置・設定・文書・ゲート)。C の下位区分として W = wire / 保存形式の契約変更、A = 追加の新契約、X = 外部依存 (Cernere) を添える。
- **規範文の置き場所 (1 機能 1 正本)**: 本書が正本の契約は、英語の引用ブロック 1 つが唯一の規範文で、C++ 移行の改訂時にそのまま `native-contracts.md` の `[id:]` 節へ移す。相手 repo や Cernere が正本の契約は、本書では ID・正本の文書と節・1 行の要約・Synergos での使い方だけを書き、定義を写さない (§4.33、§4.34)。Curare 文書は SPEC-SY-* の定義を日本語訳で写しているので、写しをやめて ID と要約にするか、「非規範の写し。差分があれば正本が優先」と明記するよう申し送る。写しの一致は M01 の契約登録ゲート (§4.1) で検査する。
- **担保手段** は HARNESS §2.0 の 3 区分 (記憶 / フック / ローカル検証ゲート) に、実行時の強制 (ランタイム) を加えた 4 種で書く。ローカル検証ゲートは Revisor のマージ前ケースを指す。M29 以降、本 repo に GitHub の CI は無い。
- 実装の着地はすべて C++ (`<module>/native/`、`cpp-migration.md:26-27`)。現行 Rust には入れない (オーナー決定 6)。Rust 側の変更は M30 (workspace 構成) と M34 (設定だけ) に限る。
- 試験の ID は AT-SY-SEC-nn (本書) と AT-CU-SEC-nn (Curare 文書)。効率の計測は AT-SY-EFF-nn。

#### 4.0.1 機構の一覧と役割

| 機構 | 名前 | Synergos の役割 | 区分 | 担保手段 | ドメイン | 段階 |
|---|---|---|---|---|---|---|
| SECURITY-M01 | 署名封筒・LCT codec・golden vector・契約登録ゲート | 正本 (Curare は TS codec で消費) | C (W, A) | ゲート + ランタイム | network-transport | P1 |
| SECURITY-M02 | HLO2 peer セッションと受付パイプライン | 正本 | C (W) + I | ランタイム + ゲート | network-transport | P4 |
| SECURITY-M03 | データプレーンの会員資格ゲート | 正本 | C (W) | ランタイム + ゲート | network-transport, mesh-core | P4 |
| SECURITY-M04 | enrollment・heartbeat・鍵のライフサイクル | 正本 (取り次ぎ・heartbeat・検証の契約。claim code・承認・node card の発行は Cernere の SPEC-CR-node-roster) | C (W) + X | ランタイム + ゲート | control-plane, network-transport | P3 |
| SECURITY-M05 | node card・招待 v2・外向き通信 | 正本 (card と招待の形式・検証。card の署名と招待の判定は Cernere) | C (W) + X | ランタイム + ゲート | mesh-core, network-transport | P4 |
| SECURITY-M06 | relay の認証 | 正本 (C++ だけで実装、オーナー決定 7) | C (W) | ランタイム + ゲート | network-transport | P4 |
| SECURITY-M07 | 失効バンドルと事件 API | 正本 (会員・ノードの失効の正本は Cernere の RosterStatus を埋め込む。control の部分は deny だけ。Curare は消費 + 入力 + PEP) | C (W) + X | ランタイム + ゲート | control-plane | P3 |
| SECURITY-M08 | custody と control の配置分離 | 正本 | C (W) + O | ランタイム + 記憶 + ゲート | opaque-custody, control-plane | P3 |
| SECURITY-M09 | custody 資格 v2 | 正本 (Curare は絞り込みと受領証検証で消費) | C (W) | ランタイム + ゲート | opaque-custody | P1 (codec)、P3 |
| SECURITY-M10 | control の管理者認証・サービス主体・Cloudflare の範囲 | 正本 (Curare は `nodes.read` だけのクライアントとして消費。SPEC-CU-membership-export は Curare が正本の取り次ぎだけの契約で、control は Curare から会員資格を受け取らない。会員・名簿を変えるルートは持たない) | C (W) | ランタイム + ゲート | control-plane, admin-ui | P3 |
| SECURITY-M11 | 局所 IPC の capability | 正本 | C (W) | ランタイム + ゲート | network-transport (synergos-ipc), mesh-core (ipc_server) | P4 |
| SECURITY-M12 | sandbox profile | 正本 (Curare の検査ワーカーが消費) | C (A) | ランタイム + ゲート | mesh-core | P4 |
| SECURITY-M13 | パス方針・pull 専用の受信・容量予約 | 正本 | C (W) + I | ランタイム + ゲート | mesh-core, network-transport | P4 |
| SECURITY-M14 | 置換旧バイト・ブレーカー・保持受領・耐久書込 | 正本 | C (W) + I | ランタイム + ゲート | mesh-core | P4 |
| SECURITY-M15 | 履歴の保持ロック・copy-not-move・不変複製・scrub | 正本 | C (W, A) + I | ランタイム + ゲート | mesh-core, opaque-custody | P5 |
| SECURITY-M16 | 破壊操作ブレーキと custody のライフサイクル | Synergos 側半分 (手順の正本は Curare の SPEC-CU-destructive-op、custody lifecycle の正本は本書) | C (W) | ランタイム + ゲート | opaque-custody, mesh-core, control-plane | P3 |
| SECURITY-M20 | 安全な既定設定と起動時 fail-fast | Synergos 側半分 (manifest 形式の正本は Curare) | O + I | ランタイム + ゲート | control-plane, mesh-core | P0 (site)、P3/P4 |
| SECURITY-M21 | AbusePolicy と quota | Synergos 側半分 (表の形式の正本は Curare) | C + I | ゲート + ランタイム | 全ドメイン (行は各ハンドラのドメイン) | P0 (報告モード)、P3/P4 |
| SECURITY-M22 | 外向き通信の方針 | 消費 (正本は Curare) | C | ランタイム + ゲート | mesh-core | P4 |
| SECURITY-M23 | decoder の境界と fuzz 登録 | 正本 | C (W) + I | ゲート | 各 decoder のドメイン、quality-gates (登録表) | P1 |
| SECURITY-M24 | 取込判定の強制と展開ゲート | 消費 (判定の正本は Curare) | C | ランタイム + ゲート | mesh-core | P5 |
| SECURITY-M25 | セキュリティイベントと custody ジャーナル | custody ジャーナルの正本 (イベント形式の正本は Curare) | C (A) | ゲート + ランタイム | opaque-custody, mesh-core | P3, P4 |
| SECURITY-M26 | 検知ジョブ | Synergos 側半分 | O + C | ランタイム + ゲート | control-plane, opaque-custody | P6 |
| SECURITY-M27 | 封筒暗号・テナント分離・チャンク境界 | 正本 (Curare の MinIO 側が消費) | C (A) | ランタイム + ゲート | opaque-custody, mesh-core, chunk-store | P5 |
| SECURITY-M28 | バックアップ・不変層・復旧訓練・サイト bundle v2 | Synergos 側半分 (bundle 形式の正本は Curare) | O + C | ランタイム + ゲート | control-plane (scripts/site) | P5 |
| SECURITY-M29 | マージ前の供給網ゲート | Synergos 側半分 | O | ゲート + フック | quality-gates | P0 |
| SECURITY-M30 | quick-xml の経路除去と workspace 分割 | 正本 | O | ゲート | desktop-gui | P0 |
| SECURITY-M31 | 隔離ビルドと署名付き成果物 | Synergos 側半分 | O + I | ゲート + ランタイム | control-plane, desktop-gui, admin-ui | P6 |
| SECURITY-M32 | HTTP の基線 | Synergos 側半分 | I | ゲート | control-plane, admin-ui, opaque-custody | P3 |
| SECURITY-M33 | 運用文書と演習 | Synergos 側半分 | O | 記憶 + ゲート | control-plane | P0 |
| SECURITY-M34 | 現行 Rust 実行時への暫定補償 | 正本 | O | 記憶 + ランタイム (設定値) | control-plane, mesh-core, network-transport | P0 |

M17 (Curare のセッション検証と SSF 受信)・M18 (Curare の権限モデルと magic link v2)・M19 (Curare のデータ不変性) は Curare だけの機構で、Curare: spec/plan/redesign-security.md が持つ。

#### 4.0.2 転送効率文書からの追補 (X1〜X5) の受け入れ

転送効率文書は、本書が正本の契約に次の追補を求めた。本書はすべて受け入れ、§4 の契約文を改めた。転送効率文書は改めた後の文面を参照し、「変更提案 (security 正本への追補)」の印を外す。

| # | 契約 | 改訂前 | 改訂後 (本書 §4 の契約文) |
|---|---|---|---|
| X1 | SPEC-SY-relay-auth | "WSS only." | QUIC (UDP 443) か WSS (TCP 443)。join の challenge はどちらの経路でも同じ。framing は SPEC-SY-relay-v2 (§4.6) |
| X2 | SPEC-SY-node-card | `topo` なし | Cernere 署名の payload の中に任意の `topo` (SPEC-SY-node-roles。承認された役割・site・relay・容量) を持つ。実行時に測った到達性 `candidates[]` は card に入れず、ノード鍵で署名する `Presence` (ctx `presence`) に分ける (§4.5、オーナー決定 8) |
| X3 | SPEC-SY-custody-capability-v2 | "PUT is single-use per jti until commit, and resume reuses the jti"、`digest` 必須 | PUT は `object_keys = ["upload:<upload_id>"]` と `max_bytes` を名指しし、`digest` は任意。長い upload の更新は同じ `upload_id` に新しい jti を発行し、「upload_id ごとに commit は 1 回」が単回性の単位になる (§4.9) |
| X4 | SPEC-SY-membership-token | TXFR, BSW1, GSP1, DHT, FileWant, history serve | ゲート対象は TXF2, SYNC, PLAN, RLY, GSP1, JOIN1。TXFR v1・BSW1・DHT・FileWant は `synergos/2` で受けない (C++ 移行で除去) (§4.3) |
| X5 | SPEC-SY-chunk-access | 条件付き | 無条件。転送効率文書の D-T03 が独立モジュールを採用した (§4.24) |

### 4.1 SECURITY-M01 署名封筒と LCT v1 codec、golden vector

- 区分: C (SPEC-SY-signed-envelope は W、SPEC-SY-lct-v1 は A)。対象発見: SEC-CR-10, SEC-CR-12, SEC-DA-19。フェーズ P1。
- 目的: 1 つの鍵が複数構造に署名する曖昧さ (SEC-CR-10) と、3 実装に分かれた正規化 (SEC-CR-12) を 1 つの署名関数と 1 つの token codec に畳む。以後の全機構 (membership、受領証、失効バンドル、ジャーナル) はこの 2 つだけを使う。
- 契約 SPEC-SY-signed-envelope (正本: 本書):

  > Every signature is `Ed25519(ctx ‖ 0x00 ‖ ver:u8 ‖ JCS(payload))`. `ctx` comes from a closed registry: `synergos/{peer-session, membership, gossip, chain, invite, node-card, presence, project-roster, roster-status, heartbeat, enroll, revocation, receipt, lct, journal-checkpoint}`. Each registry row names its allowed signers: `membership`, `node-card`, `project-roster` and `roster-status` are signed only by Cernere kids published under SPEC-CR-capability-signing-keys; `enroll`, `heartbeat` and `presence` only by node identity keys; `revocation` only by the control-node deny-only kid; `receipt` and `lct` only by custody kids. A signature under a ctx by a signer not listed in its row is invalid. Payloads are RFC 8785 JCS with integers only, no floats, duplicate keys rejected, ≤1 MiB, ≤128 nesting levels. Golden vectors live in Synergos `spec/vectors/` and are imported by a Curare Revisor case at a pinned commit and by a Cernere conformance case at a pinned commit. Direct `sign(` calls outside the envelope module fail a lint gate. This removes cross-protocol signature reuse (SEC-CR-10) and the triple canonicalization (SEC-CR-12).

  (D-28 `curare/{…}` stays open as Q-CU-9.)

- 契約 SPEC-SY-lct-v1 (正本: 本書):

  > LUDIARS Capability Token. It is a chain of blocks `{v:1, kid?, caveats, next_pub}`. Block 0 is signed by the issuer key under ctx `lct`, except in the membership profile, whose block 0 is signed by a Cernere membership kid under ctx `membership` (SPEC-CR-membership-issuer). Block n is signed with the secret of block n−1's `next_pub`; the holder keeps only the last secret, and deleting it seals the token. The caveat vocabulary is closed: `iss, org, project_ref, projects[{project_ref, role}], ops[], object_keys[] \| prefix, digest, byte_range, download_name, nbf, exp, sub, session_id, device_pub, peer_id, jti, max_uses, max_bytes`. An `object_keys` entry is a storage key or `upload:<upload_id>`. `download_name` is a safe ASCII basename, valid only for GET, and is the only source of the `Content-Disposition` filename. Verification intersects all blocks (a later block can only narrow), checks `exp − now ≤ MAX_TTL(profile)`, and checks the revocation bundle (`jti`, `cap_issued_before`, frozen scopes) and, for the membership profile, the embedded Cernere roster status. There are three profiles: **membership**, **custody-delegation** and **custody-capability**. Transport is `Authorization: Capability <b64url>`; the URL form is allowed only for browser GET, with TTL ≤120 s.

- 仕組み:
  - データモデル: `ctx` レジストリは `spec/vectors/ctx-registry.json` に固定 (新設)。`MAX_TTL` は profile ごとに membership 3600 秒、custody-delegation 3600 秒、custody-capability 300 秒 (URL 形式 120 秒)。
  - API: C++ に `envelope::sign(ctx, ver, payload_json)` / `envelope::verify(...)`、`lct::mint(issuer_key, caveats)` / `lct::attenuate(token, caveats, holder_secret)` / `lct::verify(token, issuer_keys, revocation_view, now)` を 1 か所だけ置く。TS 側は Curare が同じ API を持つ (Curare 文書)。
  - golden vector: `spec/vectors/` に非 BMP のキー、重複キー、大きい整数、block 連鎖の切詰め、caveat の拡大の試み、期限超過、`download_name` の不正文字を入れる。Curare は固定 commit で取り込み、Revisor ケースで同値性を検査する。
  - ctx 登録表の変更: `spec/vectors/ctx-registry.json` を変えられるのは Synergos の Revisor 変更だけで、同じ変更で golden vector を足す。Curare からの追加要求は Q-SY-5 で受ける。候補として `release-manifest` (Curare のリリース manifest 署名。Curare 文書 §4.7・Q-CU-9 の要求) を登録表の外に注記しておき、Q-SY-5 の回答後に登録する。`synergos/` の接頭辞は封筒の形式を表し、署名者を表さない。Curare が自分の kid で `journal-checkpoint` (security_events の checkpoint) と、受理後の `release-manifest` に署名することを許す。どの ctx を誰の kid で署名してよいかは、登録表の各行に `signers[]` (例 `["custody", "curare"]`) として持つ。SPEC-SY-signed-envelope の規範文どおり、`membership`・`node-card`・`project-roster`・`roster-status` の行は `["cernere"]` (SPEC-CR-capability-signing-keys で公開された Cernere の `membership` / `roster` kid) だけ、`enroll`・`heartbeat`・`presence` はノード識別鍵だけ、`revocation` は control-node の deny 専用 kid だけ、`receipt`・`lct` は custody の kid だけとし、synergos-control の kid を membership・roster 系の行に載せない。`lct::mint` は membership profile を作らない (Synergos のどの部品も membership LCT を発行しない、オーナー決定 8)。golden vector は Cernere も固定 commit の conformance ケースで取り込む。
  - 契約登録ゲート: `spec/contracts/registry.json` (新設) に `{id, owner_repo, owner_doc, owner_section, definition_sha256}` を置き、`scripts/gate/contract-registry.mjs` が (1) 本 repo の SPEC-SY-* の規範文が 1 か所だけにあること、(2) 本書と `native-contracts.md` が参照する ID がすべて登録されていること、(3) `definition_sha256` が規範文 (英語の引用ブロック) と一致することを検査する。Curare は golden vector と同じ固定 commit でこの表を取り込み、Curare 側の文書に SPEC-SY-* の定義の写しがあれば「非規範の写し」の印を要求する Revisor ケースを置く。
- 不変条件: 後続 block は権限を狭めることしかできない (積集合)。未知の caveat 名は拒否する (閉じた語彙)。浮動小数・重複キー・1 MiB 超・128 段超は拒否する。
- 失敗時: 検証失敗は型付きエラー (`lct.expired`、`lct.widened`、`lct.revoked`、`lct.unknown_caveat`、`envelope.bad_ctx`) を返し、文字列で分類しない (NR-7)。PEP は拒否して `peer.denied` などのイベントを出す。
- 変更対象: `synergos-net/native/trust/` (新設の library target。D-24) に `envelope.{hpp,cpp}`、`lct.{hpp,cpp}`、`jcs.{hpp,cpp}` を置き、`synergos-custody/native/` (受領証)・`synergos-core/native/`・`synergos-control/native/` が link する。Rust の `synergos-net/src/quic/hello.rs:42-49`、`synergos-net/src/gossip/message.rs:110-119`、`synergos-net/src/chain/file_chain.rs:76-85`、`synergos-core/src/invite_token.rs:70-71` の署名箇所は C++ 側で envelope に置き換える (Rust は変更しない)。
- 担保手段: ローカル検証ゲート (golden vector の同値性、`sign(` 直呼びの lint、fuzz の短時間実行、契約登録ゲート)、ランタイム (検証関数が唯一の入口)。

### 4.2 SECURITY-M02 HLO2 peer セッションと受付パイプライン

- 区分: C (SPEC-SY-hlo2-session-binding、W) + I (NR-1、直列 accept の解消)。対象発見: SEC-CR-01, SEC-AZ-02, SEC-CR-13, SEC-DA-01。フェーズ P4。
- 目的: 再生可能な HLO1 (`synergos-net/src/quic/hello.rs:42-49` は peer_id・公開鍵・`ts_ms` を連結して署名するだけ) を廃し、peer の識別を TLS セッションそのものに束縛する。受付の直列化 (`synergos-net/src/quic/mod.rs:445-465`) で 1 本の半開き接続が全受付を止める状態も解消する。
- 契約 SPEC-SY-hlo2-session-binding (正本: 本書):

  > QUIC with TLS 1.3 mutual authentication. Self-signed ed25519 certs on both sides; both verifiers derive the PeerId from the SPKI, and the pinned SPKI is authoritative. ALPN is `synergos/2`. `synergos/1` and HLO1 are refused with a typed close code. The first bidirectional stream carries `SESS{v, membership_lct, client_version, features}`. The LCT `peer_id` caveat must equal the TLS PeerId, or the connection closes with `0x5301 auth_mismatch`. On a duplicate PeerId, the old connection closes with `0x5302 superseded` only after the new one authenticates. 0-RTT is never used for state-changing streams. This replaces the cpp-migration line "preserve … QUIC ALPN, peer authentication".

- 仕組み:
  - プロトコル: TLS 1.3 mTLS。クライアント証明書は ed25519 自己署名。server 側と client 側の verifier がともに SPKI から PeerId を導出し、pin した SPKI と比較する。現行 verifier が無視している validity / intermediates / server_name (`synergos-net/src/quic/verifier.rs:70-80`) は pin 方式では判断に使わないことを契約に明記し、有効期間は SESS 内 LCT の `exp` で表す。
  - メッセージ: `SESS{v:1, membership_lct, client_version, features[]}` を最初の双方向 stream で 1 回だけ送る。応答は `SESS_OK{session_id, capset_digest}` か close。
  - close code: `0x5301 auth_mismatch` (LCT の `peer_id` と TLS の PeerId が不一致)、`0x5302 superseded` (同一 PeerId の新接続が認証済み)、`0x5303 forbidden` (stream 単位、M03)。`synergos/1` や HLO1 マジックへの型付き close code の値は未決 (Q-SY-1)。
  - 受付パイプライン (NR-1、契約ではなく実装要件): endpoint ループは `Incoming` を取るだけ。送信元 IP ごとの token bucket、保留ハンドシェイクの semaphore (既定 64)、semaphore が 50 % を超えたら `Incoming::retry()` 相当でアドレス検証、`Incoming` ごとに 1 task と認証期限 5 秒、接続レジストリは上限付き LRU。
  - 設定 (C++ 側で新設するキー): `quic.max_pending_handshakes` (既定 64)、`quic.auth_deadline_ms` (既定 5000)、`quic.retry_threshold_pct` (既定 50)。ALPN は設定にせず定数。現行の未使用キー `enable_0rtt` (`synergos-net/src/config.rs:585-587`) は C++ 側に持ち込まない。0-RTT は内容ハッシュ指定の GET だけに限る (E1)。
- 不変条件: 認証前の接続は stream を開けない。旧接続の切断は新接続の認証完了後だけ (現行 `synergos-net/src/quic/mod.rs:475` の `store_connection` による無条件上書きを廃止)。閉じた接続は `open_stream` の候補にならない。
- 失敗時: 認証期限超過・LCT 不正・ALPN 不一致は close して `peer.handshake_rejected` を出す。保留上限を超えた `Incoming` は retry か拒否にし、既存接続の処理を止めない。
- 変更対象: `synergos-net` の `quic/mod.rs`・`quic/hello.rs`・`quic/verifier.rs` に対応する C++ 移植 (`synergos-net/native/quic/`、新設)。`synergos-core/src/daemon.rs:750-753` の accept 分岐に対応する C++ の daemon ループ。
- 担保手段: ランタイム (mTLS と SESS 検査)、ローカル検証ゲート (AT-SY-SEC-01 の再生拒否、半開き接続が他の受付を 1 秒以上止めない試験 AT-SY-SEC-23)。

### 4.3 SECURITY-M03 データプレーンの会員資格ゲート

- 区分: C (SPEC-SY-membership-token、W)。対象発見: SEC-AZ-01, SEC-AZ-04, SEC-AZ-05, SEC-AZ-06, SEC-BR-01, SEC-BR-08, SEC-DA-03, SEC-DA-07, SEC-DA-08 (+ AGD-AI-16)。フェーズ P4。
- 目的: 「到達 = 会員」を断つ。stream 振分け (`synergos-core/src/daemon.rs:806-821`) で、本体を読む前にプロジェクト単位の認可をかける。
- 契約 SPEC-SY-membership-token (正本: 本書):

  > The membership profile of LCT v1. Block 0 is issued and signed only by Cernere (SPEC-CR-membership-issuer) under ctx `membership`: `{iss:"cernere", kid, sub_peer_id, node_id, principal (Cernere UUID of the owning human or service principal), device_id, org, projects[{project_ref, role: read\|publish\|history}], roster_epoch, exp ≤ now+3600, jti}`. `projects` is exactly the Cernere membership of `principal` intersected with the project scope of the node's Cernere-signed card. A membership LCT whose block 0 is not signed by a Cernere membership kid is rejected; synergos-control holds no membership issuer key and never mints, widens or re-signs a membership LCT. Renewal starts at 50 % of the lifetime: the node's signed heartbeat carries the renewal request, control-node relays it verbatim to Cernere and relays the new token back, and control may cache an issued token only to deliver it to the same node. `project_ref = blake3(org_id ‖ host_pubkey ‖ local_id)` for P2P-only projects; Curare-bound projects use SPEC-CU-project-descriptor; every `project_ref` is registered in Cernere before any member exists. Each connection installs a CapSet after SESS. Every stream (TXF2, SYNC, PLAN, RLY, GSP1) peeks `(magic, project_ref)` and checks `may(peer, project, op)` **before reading the body**; a deny sends `RESET 0x5303 forbidden` and a `peer.denied` event. A session whose SESS carries no membership LCT may open only `JOIN1` (SPEC-SY-invite-v2). TXFR v1, BSW1, DHT and FileWant are not accepted on `synergos/2`; history and archive nodes serve through TXF2 under the `history` or `read` role. Gossip fans out only to authorized peers and binds `origin = signer = sender`. History nodes need an explicit project list in their Cernere-signed card; `"*"` is refused.

- 仕組み:
  - データモデル: 接続ごとの `CapSet{peer_id, session_id, projects: map<project_ref, role>, exp, jti}`。`may(peer, project_ref, op)` は CapSet のローカル照合だけで、往復を足さない (E1)。CapSet を入れる前に、SESS の membership LCT の block 0 が Cernere の membership kid (JWKS で pin) の署名であること、`roster_epoch` と `principal`・`node_id` が手元の Cernere 署名 RosterStatus で失効していないことを検査する。control が作った・広げた・署名し直した LCT は拒否する (オーナー決定 8)。
  - 操作と役割 (stream 種別は転送効率文書の SPEC-SY-peer-wire が列挙する。追補 X4): TXF2 の WANT / WANT_MANY と PLAN_REQ は `read` (保持側が履歴ノードなら `history` でも可)。SYNC の HEAD 送信は `publish` (HEAD は publish 役割の会員の署名だけを受ける)、HAVESET と DIFF は `read`。RLY は relay 経由の session 確立で、送受双方の CapSet に同じ `project_ref` が要る。GSP1 の購読は `read`、HEAD の gossip は `publish`。JOIN1 は会員資格を要求しない唯一の stream で、招待の証明だけを受ける。
  - 除去する stream 種別: TXFR v1 (転送効率文書の golden vector の相互運用試験でだけ使い、本番の `synergos/2` では受けない)、BSW1 (bitswap)、DHT、FileWant。これらの攻撃面 (SEC-AZ-05, SEC-AZ-06, SEC-DA-03, SEC-DA-08) は種別ごと消える。DHT の削除は D-23。
  - gossip: fan-out は当該 `project_ref` の CapSet を持つ peer だけ。`origin = signer = sender` を検査し、無署名の ConflictAlert (`synergos-net/src/gossip/message.rs:86`) は受理しない。
  - 履歴ノード: `history.projects` に `"*"` を許さない (現行既定は `synergos-net/src/config.rs:453-455`)。現行の `serve_from_history` は要求者の会員資格を見ずに任意の版を返す (`synergos-core/src/exchange/mod.rs:959-990`、AGD-AI-16) ので、C++ では TXF2 の WANT と同じゲートを通す。
  - `project_ref` と Curare space の対応は AI・ゲーム開発文書の共有識別子に従う (本書は認可の向きだけを固定する)。どの `project_ref` も、会員が生まれる前に Cernere に登録する (P2P だけのプロジェクトは所有者が、Curare 紐付けの scope は Curare が作成者本人の権限で登録を取り次ぐ)。
- 不変条件: 本体の 1 バイト目を読む前に認可が決まる。CapSet の期限切れ・失効で接続を閉じる。
- 失敗時: `RESET 0x5303 forbidden` と `peer.denied{peer_id, project_ref, op, reason}`。同一 peer の拒否が閾値を超えたら M21 の挙動点数を加算する。
- 変更対象: C++ の stream 振分け (`synergos-core/src/daemon.rs:806-841` 相当、mesh-core)、交換 (`synergos-core/src/exchange/mod.rs:700-770, 902-990` 相当)、gossip (`synergos-net/src/gossip/node.rs:216-250` 相当、network-transport)。bitswap (`synergos-net/src/content/bitswap.rs`) と DHT (`synergos-net/src/dht/`) は C++ へ移さない。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-02: LCT の無い peer は全 stream 種別で本体前に RESET、除去した種別の magic は session ごと拒否)。

### 4.4 SECURITY-M04 ノード enrollment・heartbeat・鍵のライフサイクルと保管

- 区分: C (SPEC-SY-node-enrollment、W) + X (SPEC-CR-node-roster)。対象発見: SEC-CR-08, SEC-CR-09, SEC-CR-11, SEC-AZ-12, SEC-BR-03, SEC-DA-12。フェーズ P3 (enrollment の部分は Cernere の SPEC-CR-node-roster の提供待ち)。
- 目的: データプレーンが信頼できる根を Cernere のノード名簿に置き、control はその取り次ぎと liveness の受け口にする (オーナー決定 8)。現行の heartbeat は貼り付けた bearer の保持を示すだけで、`peer_id` は自己申告 (`synergos-core/src/control_heartbeat.rs:103-111`、`synergos-control/src/api/heartbeat.rs:88-91`)。
- 契約 SPEC-SY-node-enrollment (正本: 本書):

  > Enrollment is approved in Cernere, never in control. A Cernere org admin (or a principal that Cernere policy allows to own nodes) creates a single-use claim code in Cernere after SPEC-CR-step-up-assertion; it lasts 15 min and names the owning principal, the allowed roles and the project scope (SPEC-CR-node-roster). The node calls control-node `POST /v1/enroll {claim_code, node_pubkey, requested{roles, site_id, endpoints[]}, proof = Sig(enroll, claim_code ‖ node_id ‖ pubkey ‖ nonce)}`; control forwards the body verbatim to Cernere and relays the answer `{node_card (Cernere-signed, SPEC-SY-node-card), membership_lct (Cernere-signed)}`. Control does not evaluate, approve or record the claim code beyond forwarding. Node keys and connector tokens never appear in JSON. Heartbeat to control-node is `Sig(heartbeat, {node_id, peer_id, mesh_ip, version, ts, counter, card_hash, renew_membership?})`; it reports liveness and feeds the directory only and never changes the roster. Control rejects a counter regression, skew over 120 s, or a `peer_id`/`card_hash` that does not match the node's current Cernere-signed card. A key change is a dual-signed `KeyRotation{old_pub, new_pub, ts}` submitted through control to Cernere, which re-issues the card; changes of roles, project scope or declared endpoints are likewise Cernere card re-issues (roles and project scope need step-up). The identity key lives in the OS keystore by default; the file fallback is v2 with AAD-bound argon2 parameters, written atomically 0600.

- 仕組み:
  - API: `POST /v1/enroll` (control-node) は本文をそのまま Cernere の `POST /v1/synergos/enroll` (SPEC-CR-node-roster) へ取り次ぎ、Cernere 署名の node card と membership LCT を返すだけ。claim code の発行・承認は Cernere の step-up 必須の操作で、control-admin には claim code を作るルートが無い (SPEC-SY-control-admin-auth の `409 authority_is_cernere`)。heartbeat は署名付きで、`renew_membership` を含むとき control がその heartbeat をそのまま Cernere の `POST /v1/synergos/membership-tokens` へ取り次ぎ、応答で Cernere が発行した更新後の membership LCT を返す (寿命の 50 % で更新、SPEC-SY-membership-token)。鍵交換は新旧両鍵の署名を持つ `KeyRotation{old_pub, new_pub, ts}` を control 経由で Cernere へ出し、Cernere が card を再発行する。
  - 鍵の保管: 既定は OS keystore (Windows DPAPI / macOS Keychain / Linux Secret Service)。ファイル保管は v2 形式で、argon2 のパラメータを AAD に束縛し、`O_EXCL` の一時ファイル → fsync → rename で 0600 書込。`/tmp` や `.` へのフォールバック (`synergos-net/src/identity.rs:198`) は廃止する。
  - ノード名簿の正本は Cernere (SPEC-CR-node-roster)。control のレジストリは Cernere 署名の card の写し (キャッシュ) と liveness だけを持ち、Cernere UUID を持ってメールアドレスを持たない。node ごとに `{node_id, peer_id, card_hash, card (Cernere 署名のまま), last_counter, last_ts}` を持ち (§4.35)、card は Cernere の `GET /v1/synergos/orgs/{org}/roster?since=epoch` で更新する。写しは control の判断で足したり変えたりしない。
  - claim code: Cernere に移る (SPEC-CR-node-roster: 128 bit、1 回だけ表示、SHA-256 で保存、15 分、単回)。control は claim code を保存・評価せず、本文を取り次ぐだけ。
  - heartbeat の処理順: 署名検証 → counter と時刻ずれの検査 → `peer_id`・`card_hash` が現在の Cernere 署名 card と一致するかの検査 → その後にだけ liveness と directory の状態に触れる (SEC-DA-12 の「検査前の全 clone」を廃止)。heartbeat は名簿を変えない。保存は 1 件単位の追記 journal で、全 JSON の書換えをしない。
- 不変条件: node key と connector token は JSON 応答に出ない。enrollment の承認・node card の発行・役割と project scope と宣言済み到達性の変更・peer_id の変更 (二重署名の `KeyRotation`) は、すべて Cernere での card の再発行を経る。control は card を作らず、名簿を変えない。
- 失敗時: counter の巻戻し・120 秒超のずれ・card と一致しない `peer_id` / `card_hash` は 401 と `node.heartbeat_rejected`。同じ node の連続失敗は M21 で遮断し、M26 に警報。Cernere に届かないとき enrollment と membership LCT の更新は 503 で失敗し、control は代わりに発行しない。
- 変更対象: control の C++ 移植 (`synergos-control/src/api/heartbeat.rs`、`synergos-control/src/store/json_store.rs:211-212` 相当)、daemon の heartbeat 送信 (`synergos-core/src/control_heartbeat.rs` 相当)、識別鍵 (`synergos-net/src/identity.rs` 相当)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-06: card と一致しない鍵・`card_hash` と counter の巻戻しを拒否、AT-SY-SEC-13: Cernere の claim code の再使用と 15 分超を拒否、AT-SY-SEC-40: control が作った LCT・card・status を拒否)。

### 4.5 SECURITY-M05 node card による bootstrap、招待 v2、daemon の外向き通信

- 区分: C (SPEC-SY-node-card、SPEC-SY-invite-v2、ともに W)。対象発見: SEC-CR-02, SEC-CR-03, SEC-AZ-03, SEC-AZ-07, SEC-DA-10, SEC-DA-11。フェーズ P4。
- 目的: 平文 HTTP の TOFU (`synergos-core/src/peer_bootstrap.rs:196-200`) と無期限・多回の bearer 招待 (`synergos-core/src/invite_token.rs:44-47`) をやめる。招待は「会員資格の要求」にして、判定と資格の発行は Cernere が行う (control は取り次ぐだけ、オーナー決定 8)。node card は Cernere のノード名簿の 1 行として Cernere だけが署名する。
- 契約 SPEC-SY-node-card (正本: 本書):

  > A node card is the Cernere roster entry of one node (SPEC-CR-node-roster), signed only by Cernere under ctx `node-card`: `{v:2, org, node_id, peer_id, pubkey, principal (Cernere UUID of the owning human or service principal), device_id?, site_id?, projects_scope[] (explicit project_refs; required for history, archive and seed roles; "*" refused), endpoints[] (declared stable reachability, such as the tailnet address), protocol:"synergos/2", topo? (SPEC-SY-node-roles), roster_epoch, iat, exp ≤ 24 h}`. It replaces `/peer-info`. Reachability measured at run time is not in the card: the node publishes `Presence{node_id, peer_id, card_hash, candidates[], iat, exp ≤ 10 min}` signed by the node key named in the card under ctx `presence`, and a presence whose key or `card_hash` does not match a valid card is dropped. Control and peers distribute cards and presences but never sign or alter them. Every bootstrap names the expected `peer_id` (`bootstrap=[{url, peer_id}]`, `--peer`) and verifies the Cernere signature, so there is no TOFU. http is allowed only to loopback or the mesh/tailnet range. Every daemon-initiated fetch goes through the SPEC-CU-egress-policy classifier.

- 契約 SPEC-SY-invite-v2 (正本: 本書):

  > `{project_ref, host_peer_id, host_pubkey, invitee?, role, nonce16, iat, exp (24 h default, 7 d max), uses (default 1)}`, signed under ctx `invite`. It is consumed through a `JOIN1` stream over an authenticated session against the host's ledger; the joining node must already hold a Cernere-signed node card. The host then forwards a membership *request* `{invite, invite_proof, joiner_node_id, joiner_principal}` through control-node, which relays it verbatim to Cernere (SPEC-CR-membership-issuer `member-requests`). Cernere decides with the project's Cernere policy (`invite_max_role`, `require_approval`, owner approval with step-up when required) for every project, Curare-bound or P2P-only; control and the host never decide. On approval Cernere issues the membership LCT and a new ProjectRoster epoch. Includes `invite ls` and `invite revoke`.

- 仕組み:
  - 設定: 現行の `bootstrap_urls: Vec<String>` (`synergos-net/src/config.rs:61`) を C++ 側では `bootstrap = [{url, peer_id}]` に置き換える。`peer_id` の無い項目は起動時に拒否する。
  - node card は Cernere 署名で `exp` (≤24 時間) を持ち、ノードが登録されている間は Cernere が期限前に署名し直す。外部 IP-echo への問合せ (`synergos-core/src/peer_info_server.rs:137-145`) は行わない。宣言済みの安定した到達先 (tailnet アドレスなど) は card の `endpoints[]` で、変更は Cernere での card の再発行になる。実行時に測った到達性は、ノード鍵で署名する `Presence` (ctx `presence`、`exp` ≤10 分) で出し、control も peer もそれを配るだけで署名・変更しない。
  - 招待: ホストは `invite_ledger{nonce16, project_ref, role, invitee?, uses_left, exp, revoked}` を持つ。`JOIN1` stream で消費し、ホストは control 経由で Cernere の `member-requests` (SPEC-CR-membership-issuer) へ会員追加を要求する。判定は Curare 紐付けかどうかによらず Cernere のプロジェクト方針 (`invite_max_role`、`require_approval`、必要なら所有者の step-up 承認) で行い、control とホストは判定しない。承認されると Cernere が membership LCT と新しい ProjectRoster の epoch を出す。CLI に `invite ls` と `invite revoke` を足す。参加側が `project_id` を指定してローカルプロジェクトを別名化する経路 (`synergos-core/src/ipc_server.rs:1071-1092`) は廃止する。
  - 外向き通信: 招待・bootstrap・`peer add-url` の URL 取得は SPEC-CU-egress-policy の分類器 (接続先アドレスで判定) を通す。分類器の試験 vector は Curare から取り込む。
- 不変条件: 期待 `peer_id` が一致しない node card と、Cernere の署名が無い・検証できない node card は捨てる。招待の消費は 1 回 (既定)。会員資格と node card の発行元 (issuer) は Cernere だけで、control・ホスト・Curare は発行しない。
- 失敗時: 不一致は `bootstrap.peer_mismatch`、期限切れ・使用済み・失効の招待と Cernere の拒否は `invite.rejected{reason}`。Cernere に届かないときの招待の消費は 503 で失敗し、ホストの台帳の `uses_left` を減らさない。分類器で拒否した宛先は `egress.denied`。
- 変更対象: C++ の `peer_bootstrap`・`peer_info_server`・`invite_token`・IPC の招待コマンド相当。control の会員要求の取り次ぎ API (Cernere の `member-requests` へそのまま渡す)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-13: 偽の node card・Cernere 署名の無い card・peer_id 不一致の bootstrap 拒否、招待の再使用・期限切れ・失効・7 日超の拒否。AT-SY-SEC-28: 分類器 vector の取込試験。AT-SY-SEC-41: directory の改竄の破棄)。

### 4.6 SECURITY-M06 relay の認証 (DERP 型の暗号文中継)

- **着地 (オーナー決定 7、2026-10-23 夕)。** 外向きのセキュアな中継 (DERP 型 relay: 両端が 443 へ outbound で接続し、relay は end-to-end 暗号化された QUIC を中継するだけ) は C++ 版でだけ作る。SPEC-SY-relay-auth と SPEC-SY-relay-v2 は C++ 移行の契約で、現行 Rust には何も足さない (`synergos-relay` の Rust 実装は改修も配備もしない)。C++ relay が受入ゲートを通るまで、拠点と外部環境 (Codex クラウドなど) は Tailscale でつなぎ、`synergos-node` は tailnet IP にだけ bind する (Synergos `docs/site-operations.md`)。
- 区分: C (SPEC-SY-relay-auth、W)。対象発見: SEC-AZ-08, SEC-CR-14, SEC-DA-09。フェーズ P4。
- 目的: room_id だけが秘密の平文 WS (`synergos-relay/src/lib.rs:13-15`、`synergos-relay/src/server.rs:187-200`) をやめ、relay を「認証済みメンバー間の暗号文中継」に限る。中継の framing (QUIC 優先、WSS は代替) は転送効率文書の SPEC-SY-relay-v2 が持ち、本書は参加の認証だけを持つ (追補 X1)。
- 契約 SPEC-SY-relay-auth (正本: 本書):

  > QUIC on UDP 443 or WSS on TCP 443; the join challenge is the same on both transports, and framing is SPEC-SY-relay-v2. Plain `ws://` and unauthenticated joins are refused. Join is a challenge answered with `{pubkey, membership_lct, Sig(nonce ‖ room)}`. `room = HMAC(project_relay_key, sorted peer_ids)`, and `from_peer_id` is taken from the join. The relay forwards QUIC ciphertext only (DERP model), so the end-to-end mTLS of SPEC-SY-hlo2-session-binding is preserved. It meters bytes per client, enforces an idle timeout and checks the revocation bundle.

  本契約は C++ でだけ実装する (オーナー決定 7)。`native-contracts.md` の `[id:]` 節として移し、Rust 実行時には入れない。暫定の到達経路は Tailscale (tailnet IP への bind) で、relay の代替ではなく、C++ relay が無い間の唯一の経路である。

- 仕組み: relay は QUIC (UDP 443) と WSS (TCP 443) を受け、どちらでも join 時に同じ challenge nonce を返し、Cernere 署名の membership LCT と join の署名を検証する。relay の役割を持つノードは、Cernere 署名の card の `topo.roles` に `relay` を持つものだけ。`project_relay_key` は control が署名付き heartbeat の応答で、Cernere 署名の membership LCT がその `project_ref` を含む daemon にだけ渡し (この鍵は会員資格を与えず、room 名の導出にだけ使う)、daemon はメモリにだけ持つ。漏れても relay が会員の組合せを推測できるだけで、中継内容は読めない。`from_peer_id` は join で確定した値だけを使い、frame の自己申告は無視する。中継するのは QUIC の暗号文だけで、relay は平文を見ない。メッセージ上限・キュー上限・idle timeout・接続前上限は M21 の行で持つ (現行の既定 64 MiB と未使用の `idle_timeout_secs` は C++ 側で置き換える)。失効バンドルの対象になった peer の room は即時に退去させる。
- 不変条件: relay が平文の asset バイトを見る経路は無い。room 名から会員を推測できない (HMAC)。
- 失敗時: 認証失敗は close と `relay.join_denied`。上限超過は close と `relay.limit`。失効バンドルが 300 秒より古い relay は新規 join を拒否し、既存の room は維持する (D-21)。
- 変更対象: `synergos-relay` の C++ 移植 (`synergos-relay/native/`、新設、network-transport)、`synergos-net/src/relay/mod.rs` 相当のクライアント。C++ relay (`synergos-relay/native/`) が受入ゲートを通るまで relay を配備しない。到達は Tailscale (M34、オーナー決定 7)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-09: membership LCT の無い join・署名の不一致・`from_peer_id` の詐称の拒否、relay が平文を得られないことを QUIC と WSS の両経路で確認)。

### 4.7 SECURITY-M07 失効バンドルと事件 API (kill switch)

- 区分: C (SPEC-SY-revocation-bundle、W)。対象発見: SEC-AZ-17, SEC-BR-12, SEC-DR-07, SEC-CR-09 (+ SEC-BR-07 は Curare 正本)。フェーズ P3 (control 側)。Curare の受信側は P2 (Curare 文書)。
- 目的: 発行済みの資格・セッション・room・capability を、全 PEP で 60 秒以内 (p99) に止める手段を作る。現行は共有秘密を交換して全体を止めるしかない (`synergos-custody/src/signature.rs:52-53`、`docs/mesh-operations.md:296-303`)。
- 契約 SPEC-SY-revocation-bundle (正本: 本書):

  > `GET /v1/orgs/{org}/revocation?since=seq` returns a bundle of two independently signed parts. (1) `roster_status`: the latest Cernere-signed `RosterStatus` (SPEC-CR-node-roster, ctx `roster-status`), embedded verbatim; it is the only source of `nodes_revoked[]`, `principals_revoked[]`, `memberships_revoked[{principal, project_ref}]` and membership `sessions_revoked[]`. (2) `local`: `Sig(revocation){seq (monotonic), issued_at, exp = issued_at+300, jtis_revoked[], delegations_revoked[], cap_issued_before{org\|principal: ts}, sessions_revoked[], principals_frozen[], projects_frozen[{project_ref, mode: read_only\|full}], orgs_frozen[], quarantined_nodes[], invites_revoked[]}` signed by the control-node deny-only kid. Every entry of `local` can only deny; nothing in this bundle grants, restores or widens access, and a PEP denies when either part denies. PEPs poll every 30 s and also on a Cernere `roster-changed` SSF event. If either part is past its `exp`, a PEP denies new sessions and redemptions; streams on an already-authenticated session continue only until that session's membership LCT `exp`. Listed peers and sessions close immediately. A project may hold a Cernere-signed `OfflineLease{project_ref, site_id, site_cidrs[], peers[{peer_id, role}], issued_at, exp ≤ 72 h, max_stale ≤ 24 h}` (ctx `project-roster`, issued by Cernere on the project owner's step-up; every listed peer must be a Cernere member with at least that role); while it is valid, PEPs on that site keep granting CapSets to the listed peers from `site_cidrs` for up to `max_stale` past the bundle `exp`. Inputs to `local` are (1) SPEC-CR-ssf-caep-transmitter events, applied at once as session closes and confirmed by the next `RosterStatus`; (2) Curare's SSF receiver through `POST /v1/orgs/{org}/revocation/inputs` on `control-admin`, body `{source:"curare", idempotency_key, jtis[], cap_issued_before{<principal>: ts}}`, authenticated by the SPKI-pinned mTLS client cert of SPEC-SY-custody-service-auth plus a SPEC-CR-service-principal JWT once available, answering `202 {input_id, effective_from_seq}` and returning the first result for a repeated `idempotency_key`; and (3) the operator incident API `POST /v1/incident/{freeze\|quarantine\|revoke}`, which requires SPEC-CR-step-up-assertion, plus a second approver for org-wide scope. Incident freezes and quarantines are emergency, deny-only local entries; removing a principal or node from membership or the roster is done in Cernere, and control never lifts a Cernere revocation. The pinned verification keys are `SYNERGOS_REVOCATION_PUBKEYS` (control deny-only kids) and the Cernere `roster` kids of SPEC-CR-capability-signing-keys.

- 仕組み:
  - 発行: 会員・ノードの失効の正本は Cernere の `RosterStatus` (60 秒ごとと変更のたびに再発行、`exp` 300 秒) で、control-node はそれを改変せずに `roster_status` として埋め込む。control-admin は deny だけの `local` 部分の入力 (Cernere SSF/CAEP イベントを受けた即時のセッション切断、Curare の SSF 受信器からの jti / `cap_issued_before`、運用者の事件 API) を集め、control-node が deny 専用 kid で署名して配る。`seq` は単調増加で、差分取得 (`since=seq`) と全量取得の両方を持つ。`local` には許可・復帰・拡大の項目が無い。
  - Curare からの入力 API (この境界の正本は本書。Curare: spec/plan/redesign-security.md §4.4 の「入力」はこの名前を参照する形に改めるよう申し送る): `POST /v1/orgs/{org}/revocation/inputs`。受け口は control-admin だけで、control-node には置かない。認証は SPEC-SY-custody-service-auth と同じく Curare の SPKI を pin した mTLS で、SPEC-CR-service-principal の提供後はその JWT (scope `org:<id>:revocation.input`) を足す。本体は `{source:"curare", idempotency_key (UUID), jtis[] (≤10 000), cap_issued_before{<principal UUID>: ts}}`。同じ `idempotency_key` の再送は最初の結果 `202 {input_id, effective_from_seq}` を返し、内容が違えば 409。`effective_from_seq` は入力が反映される最初のバンドルの `seq` で、Curare はこの値を台帳 (`capability_issuance.revoked_at`) に残す。入力は追記専用の `revocation_inputs` journal に書く (§4.35)。M21 の行 (`principal_key: delegation`、毎分 60 件) を持つ。凍結 (`principals_frozen` など) はこの API では受けず、事件 API だけが行う。
  - PEP: daemon (QUIC accept・stream 振分け)、relay、custody-transfer (償還)、custody-admin (delegation)、Curare (API・取得・`/dl`) が 30 秒ごとに取得する。バンドルが `exp` を過ぎたら新規セッション・stream・償還を拒否し、既存の列挙対象は即時に閉じる。
  - freeze の意味: custody はその組織の capability を拒否、daemon はその組織のセッションを落とす、relay は room を退去させる、Curare は 423 を返す。
  - 事件 API: `POST /v1/incident/freeze`、`/quarantine`、`/revoke`。SPEC-CR-step-up-assertion が必要で、組織全体の範囲は 2 人目の承認が要る。SPEC-CR-step-up-assertion が提供されるまでは 2 名承認だけで運用する。これは緊急の deny だけの手段で、会員や名簿からの除名は Cernere で行う。control は Cernere の失効を解除できない。
  - 設定: 検証鍵は `SYNERGOS_REVOCATION_PUBKEYS` (control の deny 専用 kid → 公開鍵、Excubitor Vault から注入) と、SPEC-CR-capability-signing-keys で公開された Cernere の `roster` kid (JWKS から取得して pin)。取得間隔 30 秒と失効猶予 300 秒は定数とし、設定で緩めない。
- 不変条件: バンドルの `seq` は後退しない (後退は拒否)。古すぎるバンドルでは新規の許可を出さない (fail-closed)。
- 失敗時 (D-21): control に届かない PEP の振る舞いは時間で決まる。(1) 最後のバンドルの `exp` まで (発行から 300 秒) は通常どおり。(2) その後は新規セッション・新規 relay join・custody の償還・Curare の capability 発行を拒否し (`revocation.stale`、custody と Curare は 503 `revocation_stale`)、認証済みセッションの stream は各セッションの membership LCT の `exp` まで続ける。heartbeat が通らないので LCT は更新されず、最長 3600 秒で既存セッションも閉じる。Cernere に届かず `RosterStatus` が `exp` を過ぎた場合も同じ扱いになる。(3) Cernere 署名の `OfflineLease` があるプロジェクトでは、拠点内アドレスの列挙 peer に限り `max_stale` まで新規セッションも許す (`revocation.lease_used` を記録)。M26 に警報する。control-node の 1 インスタンスの停止は G11 の多重化で吸収し、PEP は 2 つ目のインスタンスから取得する。Curare からの入力 API に届かない間、Curare は入力を手元の送信待ち表に積んで再送する (同じ `idempotency_key`)。
- 変更対象: control の C++ 移植 (`synergos-control/native/`、新設、control-plane。新 API・入力 API・署名・`revocation_inputs` journal)、daemon・relay・custody の C++ 側に共通の `revocation_view` (取得・検証・照合・貸与。`synergos-net/native/trust/` の library)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-10: SSF イベントと事件 API から custody 403 と QUIC close まで ≤60 秒、AT-SY-SEC-32: control 不達 10 分 / 2 時間の振る舞い、AT-SY-SEC-39: Curare からの入力 API の認証と `idempotency_key`)。
- 既知の穴: P3 の会員資格の部分と P4 は Cernere (SPEC-CR-membership-issuer、SPEC-CR-node-roster、SPEC-CR-ssf-caep-transmitter の追加イベント) を待つ。Synergos 側に代わりの暫定は置かない (control に会員・名簿の鍵を持たせない)。それまでは拠点が現行 Rust 実行時を M34 の設定で Tailscale 越しに動かし、利用者除名の G1 (≤60 秒) は満たさない。既知の穴として記録する。

### 4.8 SECURITY-M08 custody と control の配置分離、プロセス内 TLS [変更提案]

- 区分: C (SPEC-SY-custody-topology、W) + O。対象発見: SEC-BR-15, SEC-CR-06。フェーズ P3。
- 目的: 1 プロセス 1 listener (`synergos-control/src/main.rs:76-79`) に Cloudflare token・admin token・custody token・受領証秘密・全組織の custody 実体が集まる状態をやめる。
- 契約 SPEC-SY-custody-topology (正本: 本書):

  > There are separate deployables and OS users:<br>• `custody-transfer`: public TLS; LCT only.<br>• `custody-admin`: private; mTLS from Curare.<br>• `control-admin`: loopback or mTLS; OIDC operators.<br>• `control-node`: mesh TLS; enrollment, heartbeat and bundle; holds no custody keys.<br>• `relay`.<br>Each has its own Vault policy and data under OS service directories outside any checkout. Split listeners in one process are allowed only as the first C++ step. This replaces `opaque-custody.md` "別listenerを起動しない".

- 仕組み:
  - 配備単位ごとに OS ユーザー・Vault policy・データディレクトリ (OS のサービスディレクトリ、checkout の外) を分ける。秘密は 1 つの所有サービスだけが持つ。
  - TLS はすべてプロセス内で終端し、証明書は Vault PKI から得る。`http://` は loopback 以外で拒否する。
  - Excubitor の catalog は配備単位ごとのサービス定義になる。現行の単一定義 (`excubitor.catalog.yaml:4-37`、`SYNERGOS_CONTROL_BIND: 127.0.0.1:4250`) は C++ 移行時に置き換える。
  - C++ の最初の段階は 1 プロセスで listener を分ける形 (public transfer と private admin) を許す。現行の Rust 実行時では、Curare: `spec/feature/codex-cloud-access.md:10-13` の `tailscale serve` がパスで transfer だけを tailnet に出しており、これが同じ分離のプロキシ層での暫定形になる (M34)。
  - 配備単位の組合せの禁止 (転送効率文書の SPEC-SY-node-roles の起動検査と同じ規則): custody-admin と control-admin は、インターネットに向く役割 (custody-transfer、relay、edge、seed) と同じホストで動かさない。
- 不変条件: custody-transfer は LCT 以外の資格を受けない (旧 `SYNERGOS_CUSTODY_TOKEN` の bearer も、猶予期間後の HMAC URL も受けない)。control-node は custody の鍵を持たない (Vault policy に custody の鍵の path が無い)。Cloudflare token は control-admin の配備単位だけが持つ。どの配備単位もリポジトリ checkout の下にデータを置かない。
- 失敗時: TLS 設定や Vault policy が欠けた配備単位は起動しない (M20 の起動検査)。禁止された組合せの設定は起動時に拒否する。
- 変更対象: `synergos-control/src/main.rs` と `synergos-custody/src/lib.rs` の C++ 移植 (`synergos-control/native/`、`synergos-custody/native/`)、`excubitor.catalog.yaml` と `excubitor.secrets.json` (control-plane)、`scripts/site/*` (C++ 版の配備手順、control-plane)。
- 担保手段: ランタイム (起動時の検査)、記憶 (`spec/operations.md` に配備図)、ローカル検証ゲート (AT-SY-SEC-14: custody-transfer が LCT 以外の資格を拒否、`excubitor.secrets.json` 上で control-node の consumers に custody の鍵が無い)。

### 4.9 SECURITY-M09 custody 資格 v2 (サービス認証、delegation、capability、受領証)

- 区分: C (SPEC-SY-custody-service-auth、SPEC-SY-custody-capability-v2、SPEC-SY-custody-receipt-v2、いずれも W。receipt-v2 と capability-v2 は [変更提案])。対象発見: SEC-CR-04, SEC-CR-05, SEC-CR-12, SEC-AZ-13, SEC-BR-05, SEC-BR-06, SEC-DA-13, SEC-DR-08。フェーズ P1 (codec)、P3 (C++)。Curare の利用側は P2/P3 (Curare 文書)。
- 目的: 1 つの `receipt_secret` が bearer 比較 (`synergos-custody/src/handlers.rs:33-34`)・capability URL 署名 (`:185-188`)・受領証署名 (`:125-128`) を兼ね、検証者の Curare も同値を持つ状態をやめる。受領証の鍵を持つ者が capability を作れず、capability を持つ者が受領証を偽造できない形にする。
- 契約 SPEC-SY-custody-service-auth (正本: 本書):

  > Curare authenticates to `custody-admin` with an SPKI-pinned mTLS client cert. Once available, it adds a SPEC-CR-service-principal JWT carrying `orgs[]`. Custody returns a custody-delegation LCT `{iss:"custody:<instance>", sub:"curare:<deployment>", orgs[], ops:[admit, head, verify, get, put, reclaim_request, commit], exp ≤ 3600, jti, max_bytes_per_hour}`. It replaces `SYNERGOS_CUSTODY_TOKEN` and Curare's `SYNERGOS_STORAGE_TOKEN`.

- 契約 SPEC-SY-custody-capability-v2 (正本: 本書):

  > The custody-capability profile of LCT v1, attenuated by Curare from the delegation: `{org, project_ref, object_keys[]\|prefix, digest?, method: GET\|PUT, byte_range?, download_name?, max_bytes?, exp ≤ 300 (URL ≤120), jti, sub, session_id, device_pub?}`. For PUT the capability names `object_keys = ["upload:<upload_id>"]` and `max_bytes`; `digest` is optional, and when it is absent the receipt digest is authoritative and Curare compares it at commit. A long upload renews by minting a new jti for the same `upload_id`; custody allows a single commit per `upload_id`, and a commit never replaces published bytes. For GET, `download_name` is the only source of the `Content-Disposition` filename. Custody enforces TTL, revocation and per-delegation and per-`sub` redemption budgets. Each redemption is written to SPEC-SY-custody-journal with the jti. This replaces the HMAC-signed URL ("Clientには…HMAC署名URLだけを渡す") and the 900 s GET URL maximum of SPEC-SY-custody-acquisition.

- 契約 SPEC-SY-custody-receipt-v2 (正本: 本書):

  > `JCS{v:2, kid, alg:"Ed25519", iss: custody_instance_id, iat, exp, nonce, operationId, opaqueObjectId, org, project_ref, digests[{alg, value}], byteLength, locations[], durability: durable_single\|durable_replicated}` plus a detached signature under ctx `receipt`. Public keys are served at `GET /v1/custody/keys` and pinned in Curare as `SYNERGOS_RECEIPT_PUBKEYS={kid: pub}`. Curare stores the bytes, signature and kid verbatim and accepts `iat ≤ now + 120 s`. v1 HMAC is verify-only for stored rows during the grace window, then removed.

- 仕組み:
  - 鍵の用途分離: custody-admin は (1) Curare 認証用の mTLS 信頼 (Curare の SPKI を pin)、(2) delegation 発行鍵、(3) 受領証署名鍵を、それぞれ別の kid で持つ。custody-transfer は delegation 発行鍵の**公開鍵**と失効バンドルだけを持つ。
  - 流れ: Curare は custody-admin から組織ごとの delegation LCT (≤1 時間) を得て、要求ごとにローカルで capability に減衰する (E2: 実体ごとの custody-admin 呼出 0 回)。クライアントは `Authorization: Capability <b64url>` で custody-transfer に提示する。URL 形式はブラウザの GET だけで ≤120 秒。
  - 償還予算: delegation 単位と `sub` 単位の bytes / objects / 同時 PUT の上限 (M21 の行)。
  - PUT の単回性 (追補 X3): 単回性の単位は jti ではなく `upload_id` で、custody は `upload_id` ごとに commit を 1 回だけ受ける。100 GB の upload は 300 秒の capability より長く続く (転送効率文書の TG-06) ので、Curare は同じ `upload_id` に新しい jti の capability を 300 秒に 1 回以下で発行し (E2)、custody はどの jti の PATCH でも同じ `.part` に追記させる。`max_bytes` は upload 全体の上限で、jti をまたいで数える。`digest` は任意で、無い場合は custody が commit 時に計算した digest を受領証に書き、Curare が changeset の commit で照合する (保管名は内容から決まるので、upload の開始時には分からない)。再開はバイト位置からで、手順は転送効率文書の SPEC-SY-custody-transfer-v2 (`draft-ietf-httpbis-resumable-upload`) に従う。これは `spec/feature/opaque-custody.md:40`「バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送」に対する転送効率文書の [変更提案] で、本書は capability の側だけを持つ。
  - GET の寿命と取得名: GET の capability は ≤300 秒 (ブラウザ URL 形式は ≤120 秒) で、`spec/feature/opaque-custody.md:37` の「最大900秒のGET URL」と現行実装 (`synergos-custody/src/handlers.rs:153` が `1..=900` を受け、`:98` が PUT URL を 900 秒で切る) を置き換える [変更提案]。`:39` の「URL署名は…取得名へ結び付ける」は、caveat `download_name` で保つ。現行は `name` を HMAC の対象に含めている (`synergos-custody/src/handlers.rs:184-194`)。caveat は LCT の署名に含まれるので束縛は落ちない。custody は `Content-Disposition` を `download_name` からだけ作り、caveat が無ければ `attachment` だけを返す。
  - 受領証: v2 は JCS バイト + 分離署名 (ctx `receipt`)。`GET /v1/custody/keys` で公開鍵を配り、Curare は `SYNERGOS_RECEIPT_PUBKEYS` に pin する。Curare はバイト・署名・kid をそのまま保存する (現行は署名を保存しない、SEC-CR-12)。
  - 旧秘密の撤去: kid の二重受理の猶予 (≤30 日) の後に、次の 3 組の共有秘密を廃止する。名前の対応は両 repo の `excubitor.catalog.yaml` のコメントに従う (Synergos `excubitor.catalog.yaml:37`、Curare `excubitor.catalog.yaml:36-37`)。

    | Synergos の名前 | Curare の名前 | 置き換え先 |
    |---|---|---|
    | `SYNERGOS_CUSTODY_TOKEN` | `SYNERGOS_STORAGE_TOKEN` | Curare の SPKI を pin した mTLS + delegation LCT (SPEC-SY-custody-service-auth) |
    | `SYNERGOS_CUSTODY_RECEIPT_SECRET` | `SYNERGOS_RECEIPT_SECRET` | custody の受領証署名鍵 (Ed25519、Vault Transit) と Curare の `SYNERGOS_RECEIPT_PUBKEYS` |
    | `SYNERGOS_CONTROL_ADMIN_TOKEN` | `SYNERGOS_CONTROL_TOKEN` | SPEC-SY-control-admin-auth (M10) |
- 不変条件: 受領証の検証者は署名鍵を持たない。capability の TTL は検証時に上限で切る (`exp − now ≤ 300`、URL 形式は 120)。admission は `{org_id, project_ref}` を持ち、他組織の delegation では償還できない。`upload_id` ごとの commit は 1 回で、公開済みの実体を置き換えない。
- 失敗時: 期限超過・範囲外・予算超過は 403 と `custody.redeem_denied{jti, reason}`。同じ `upload_id` の 2 回目の commit は 409。台帳に無い jti の償還は `capability.orphan` 警報 (Curare 側の SPEC-CU-capability-ledger と jti で照合)。失効バンドルが古い間の償還は 503 `revocation_stale` (D-21)。
- 変更対象: `synergos-custody/src/handlers.rs`・`signature.rs`・`contract.rs`・`config.rs` の C++ 移植 (`synergos-custody/native/`、opaque-custody)。`synergos-custody/native/signature.cpp` は v1 検証専用になる (§4.39)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-08、AT-SY-SEC-31 の寿命上限、AT-SY-EFF-01 の E2 計測)。

### 4.10 SECURITY-M10 control の管理者認証、サービス主体、Cloudflare の範囲

- 区分: C (SPEC-SY-control-admin-auth、W)。対象発見: SEC-AZ-11, SEC-BR-04, SEC-BR-06, SEC-CR-15, SEC-AZ-18, SEC-BR-09, SEC-SC-09, SEC-AZ-25。フェーズ P3 (C++)。Curare のクライアント側は P2。
- 目的: 全組織共通の静的 bearer (`synergos-control/src/auth.rs:12-28`) と、ブラウザの sessionStorage に置く admin token (`synergos-admin-ui/src/session.rs:3-6`)、組織共通の Cloudflare token をやめる。
- 契約 SPEC-SY-control-admin-auth (正本: 本書):

  > Operators sign in with Cernere OIDC code+PKCE and get an HttpOnly, SameSite=Strict cookie (30 min idle, 8 h absolute). Roles come from Cernere org roles: `synergos.node_admin`, `synergos.mesh_operator`, `synergos.account_admin`. Control-admin has no route that creates, approves, changes or removes project membership, node enrollment, node ownership, node keys or declared node reachability; such requests answer `409 authority_is_cernere` with a link to the Cernere roster or membership page. `revoke_dark` and connector issuance need step-up (`auth_time` < 5 min); `revoke_dark` also needs an approved dry-run diff and a second person. Curare uses a service principal scoped to `org:<id>:nodes.read` only. Cloudflare uses a dedicated least-scope token whose scope is checked at boot and fails closed; there is no request-scoped token path, and `revoke_dark` uses a JIT token from the Excubitor Cloudflare broker. The break-glass token is single-use with forced rotation and cannot reach any membership or roster function. Every route declares `{principal_kind, scope, step_up}`.

- 仕組み: control-admin の全ルートが `{principal_kind, scope, step_up}` を宣言し、宣言の無いルートは起動時に失敗する。会員・enrollment・ノードの所有・ノード鍵・宣言済み到達性を作る・承認する・変える・消すルートは置かず、そうした要求には `409 authority_is_cernere` と Cernere の名簿・会員ページへのリンクを返す (key reissue とノード表示名の変更も Cernere での card の再発行になるので、旧 `nodes.display_name.write` は無くなる)。break-glass token も会員・名簿の機能には届かない。運用者 allowlist (`SYNERGOS_OPERATOR_IDS` など) は Cernere の役割 claim に置き換える。Curare の IPC proxy の資格は `read` だけ (M11)。request-scoped の Cloudflare token 経路 (`synergos-control/src/api/mesh_setup.rs:132-137`) は削除する。node_key / connector_token を JSON やシェル命令文字列で返す応答 (`synergos-control/src/api/mesh_setup.rs:168-171`) は廃止し、enrollment (M04) に置き換える。
- 不変条件: 静的な共有 admin bearer は存在しない (break-glass を除き、それも単回)。Cloudflare token は 1 サービス専用で、起動時に範囲を検査する。control-admin は会員・名簿を変えられない (権威は Cernere、オーナー決定 8)。
- 失敗時: 範囲検査に失敗した Cloudflare token では起動しない。step-up の無い危険操作は 403。会員・名簿を変える要求は `409 authority_is_cernere`。
- 変更対象: `synergos-control/src/auth.rs`・`api/mod.rs`・`api/mesh_setup.rs`・`api/reconcile_api.rs`・`cloudflare/client.rs` の C++ 移植 (`synergos-control/native/`、control-plane)、`synergos-admin-ui` のセッション処理 (cookie 化、admin-ui)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-07、ルート宣言の網羅試験 AT-SY-SEC-17、AT-SY-SEC-40 の `409 authority_is_cernere`)。
- 依存: SPEC-CR-step-up-assertion、SPEC-CR-service-principal (提案中。AGD-AI-12 と AGD-GW-13 のとおり、Codex クラウドのエージェントやレンダーファームは今は人間級の Cernere token を持つしかない)。提供前は mTLS と 2 名承認で運用する。

### 4.11 SECURITY-M11 局所 IPC の capability

- 区分: C (SPEC-SY-ipc-capability、W)。対象発見: SEC-AZ-09, SEC-BR-02, SEC-AZ-18, SEC-DA-21, SEC-CR-17。フェーズ P4。
- 目的: 「同一 uid = 全権」(`synergos-core/src/ipc_server.rs:235-241`) と、資格取得失敗時の fail-open、Windows の DACL 不在をやめ、クライアント種別ごとの範囲付き資格にする。
- 契約 SPEC-SY-ipc-capability (正本: 本書):

  > The first frame is `IHELLO{v, client_kind: cli\|gui\|plugin\|tauri\|curare, token}`. The token file is `<runtime_dir>/synergos/ipc.token` (0600 in a 0700 dir, or an owner-SID DACL on Windows); there is no `/tmp` fallback, and a credential-lookup error means deny. Scopes are `read`, `project:<ref>:sync`, `project:<ref>:publish`, `admin`, `destructive`. Destructive commands (HistoryPurge, TagRm, Shutdown, new-root ProjectOpen) need a 60 s confirmation nonce or a step-up token and emit `ipc.destructive`. Curare's token is limited to `read`. Logs record the command discriminant only. This replaces the cpp-migration line "Local IPC authorization: same-user checks".

- 仕組み: 最初の frame が `IHELLO` で、token の範囲に無いコマンドは型付きエラーで拒否する。破壊的コマンドは 2 段階 (要求 → 60 秒有効の確認 nonce → 実行) か step-up token。ログはコマンドの判別子だけを記録し、payload は記録しない (`synergos-core/src/ipc_server.rs:504` の Debug 出力を廃止、NR-6)。接続数・idle・読取の上限と、1 MiB を超える応答のページングは M21 と転送効率文書の IPC 項目が持つ。
- 互換契約との関係: `cpp-migration.md:35-37` は「Preserve command, response and event variants」と定める。既存の variant はすべて残し、`IHELLO` の frame と破壊的コマンドの確認 (`ConfirmRequired{nonce, expires_at}` 応答と `Confirm{nonce}` 要求) を足す。これは「維持」から「維持 + 追加」への改訂で、§4.37 に [変更提案] として載せる。frame の上限 1 MiB (`SPEC-SY-native-framing`) は変えない。
- 不変条件: 資格の取得に失敗したら拒否する (fail-closed)。Curare の IPC proxy は `read` だけ。
- 失敗時: 範囲外は `ipc.denied{client_kind, command}`、破壊的コマンドは確認無しで実行しない。60 秒を過ぎた確認 nonce と、別の接続から出された確認 nonce は拒否する。
- 変更対象: `synergos-ipc` の C++ 移植 (`synergos-ipc/native/`、既存の `frame.cpp` に続く層、network-transport)、`synergos-core/src/ipc_server.rs` 相当の C++ (mesh-core)、CLI・Tauri (desktop-gui)・Ars プラグイン (ars-plugin) のクライアント。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-05、AT-SY-SEC-34)。

### 4.12 SECURITY-M12 hook・解析器・変換器の sandbox profile

- 区分: C (SPEC-SY-sandbox-profile、A)。対象発見: SEC-DA-06, SEC-AZ-10, SEC-BR-10, SEC-CR-07, SEC-SC-12, SEC-DA-20。フェーズ P4 (daemon の hook)。Curare の検査・変換ワーカーは P5。
- 目的: shell 文字列実行 (`synergos-core/src/hooks/runner.rs:430-439`)、全環境継承 (`:315-325`)、peer が上書きできる repo 相対スクリプトの実行をやめる。
- 契約 SPEC-SY-sandbox-profile (正本: 本書):

  > argv only, with whole-element placeholders; `sh -c` and `cmd /C` are refused for post-receive. `env_clear()` and then an allowlist. No network. Input is read-only, output is write-only to a single directory, and the state dir is unreachable. CPU, memory and wall-clock caps apply. Executables must be outside the synced tree or `command_hash`-pinned (BLAKE3), and `hooks.toml` changes need per-project approval recorded with the hash. Backends per OS follow D-14; a host with no backend refuses to run. Each run emits `hook.executed`.

- 仕組み: `HookDef` の `command: String` (`synergos-net/src/config.rs:113-123`) を C++ 側では `argv: [string]` + `command_hash` に置き換え、`{file}` などの置換は argv の 1 要素全体だけに許す。OS 別 backend は D-14 (Linux: gVisor、無ければ bubblewrap + seccomp。macOS: App Sandbox。Windows: AppContainer + Job)。`hooks.toml` の承認は `hook_approvals{project_ref, hooks_toml_blake3, approved_by, approved_at}` に記録し、ハッシュが変われば再承認まで実行しない。
- 不変条件: backend の無いホストでは hook を実行しない。hook は daemon の秘密を環境から読めない。
- 失敗時: 拒否は `hook.refused{reason}`。資源上限超過は kill して `hook.killed`。
- 変更対象: `synergos-core/src/hooks/` の C++ 移植、`docs/hooks.md` の更新 (POSIX 前提の記述と `%SYNERGOS_FILE%` の例を差し替え)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-12 の適合試験: ネットワーク接続・出力外書込・環境変数の秘密読取がすべて失敗すること)。

### 4.13 SECURITY-M13 パス方針、pull 専用の受信、受信前の容量予約

- 区分: C (SPEC-SY-path-policy、SPEC-SY-pull-only-receive、ともに W) + I (handle 相対の操作、NR-3)。対象発見: SEC-DA-04, SEC-DA-05, SEC-IN-02, SEC-IN-17, SEC-BR-01。フェーズ P4。
- 目的: 大小文字区別の `.synergos` 保護 (`synergos-core/src/manifest.rs:305-311`)、`.git/` 無保護、未要求の TXFR 受理 (`synergos-core/src/exchange/mod.rs:700-770`)、文字列による check-then-use をやめる。
- 契約 SPEC-SY-path-policy (正本: 本書):

  > One normalizer: NFC; always case-fold; strip trailing dot and space; reject 8.3 short names, ADS and device names. Deny at any depth: `.synergos .git .hg .svn .p4config .vscode .idea .cargo .github .vs .gitattributes .gitmodules *.lnk`. Exec bits are never set on received files. File operations are handle-relative (openat2 `RESOLVE_BENEATH`, `O_NOFOLLOW_ANY`, NtCreateFile relative to the root handle). This extends the line "Reject path escapes, symlinks/junctions".

- 契約 SPEC-SY-pull-only-receive (正本: 本書):

  > Received content is accepted only as the answer to an outstanding local want (a TXF2 `WANT`/`WANT_MANY`, or the fetch ranges of a range plan) whose `(project_ref, blake3, size)` equals a publisher-signed Head or manifest entry; unsolicited pushes do not exist, and any body that answers no outstanding want is reset before a byte is persisted. Paths come from the signed manifest, never from a frame header or the holder. Space is reserved against the project quota (SPEC-CU-quota-reservation, or the local project quota for P2P-only projects) and a free-space watermark before the body is read.

- 仕組み: 正規化関数は C++ の `relative_path.cpp` (`synergos-core/native/`、既存) を拡張して 1 つに保ち、publish・受信・restore・IPC・Curare CLI の clone で共有する (試験 vector を Curare に渡す)。受信は「自分が出した want」と「発行者署名の Head / manifest 項目」の両方に一致した内容だけを受け、パスは署名済み manifest から取る。転送効率文書の TXF2 は受信側主導の pull なので、この契約は構造として満たされる。現行の TXFR (`synergos-core/src/exchange/mod.rs:700-770`) のような未要求の送信は、C++ では種別ごと存在しない。本体を読む前にプロジェクト quota と空き容量の閾値で予約する。予約の設定キーは C++ 側で `receive.free_space_watermark_bytes` と `receive.project_quota_bytes` を新設する (Curare 紐付けプロジェクトは SPEC-CU-quota-reservation が上限を与える)。
- 不変条件: 受信ファイルに実行ビットを立てない。deny リストのパスはどの深さでも書かない。予約できない受信は本体を読まない。
- 失敗時: `receive.rejected{reason: unsolicited\|path_denied\|quota\|watermark}`。
- 変更対象: `synergos-core/native/relative_path.cpp` (mesh-core)、受信・restore の C++ 移植 (`synergos-core/src/exchange/mod.rs`、`manifest.rs`、`restore.rs` 相当、mesh-core)、TXF2 の受信側 (`synergos-net/native/`、network-transport)。
- 担保手段: ランタイム、ローカル検証ゲート (正規化関数の property test と fuzz、Windows / macOS / Linux で `.GIT/hooks/x` と `.SYNERGOS/` が拒否される AT-SY-SEC-03、未要求の送信が本体前に拒否される AT-SY-SEC-15)。

### 4.14 SECURITY-M14 置換旧バイトの保持、大量変更ブレーカー、保持受領、単一の耐久書込 [変更提案]

- 区分: C (SPEC-SY-displaced-preimage は W、SPEC-SY-retention-ack は W [変更提案]) + I (NR-4、fsync なし・二重書込の解消)。対象発見: SEC-IN-01, SEC-IN-07, SEC-IN-08, SEC-IN-15, SEC-BR-01。フェーズ P4。
- 目的: ランサム被害者の大量送信 1 回で全メンバーの作業コピーが旧バイト無しに置換される経路 (`synergos-core/src/exchange/mod.rs:764-769, 795`) を断つ。
- 契約 SPEC-SY-displaced-preimage (正本: 本書):

  > Every received replace first renames the displaced bytes to `<state_dir>/displaced/<blake3>` on the same volume, with a journal entry and TTL ≥14 d; `project undo-receive --since` restores them. A mass-change breaker counts files and bytes per window and watches content-class shifts such as entropy jumps; when it trips, the project is quarantined (new versions staged, not applied) and release needs step-up. All writes use the single durable-write primitive: tmp `O_EXCL` 0600, fsync, rename/MoveFileEx `WRITE_THROUGH`, fsync of the directory.

- 契約 SPEC-SY-retention-ack (正本: 本書):

  > A project in auto-pull sync mode must have at least one valid `RetentionAck{project_ref, version, node_id, retain_until, sig}` from a retention-capable node within its policy. Otherwise publish to that project is staged and not propagated. This turns the 2026-08-16 opt-in history flag into an obligation of the sync mode.

- 仕組み:
  - 置換は「旧ファイルを同一ボリュームの `displaced/` へ rename → journal 追記 → 新ファイルを耐久書込で公開」の順。全体コピーをしない (E6)。`project undo-receive --since <ts>` で戻す。
  - ブレーカー: 窓ごとのファイル数・バイト数・内容種別の変化 (エントロピーの急変など) を数え、閾値で当該プロジェクトを隔離する (新版は staging に置き、作業ツリーへ適用しない)。解除は step-up。暫定の閾値は「60 秒の窓で、置換が 500 ファイル以上、または置換バイトが 1 GiB 以上、または置換ファイルの 30 % 以上で Shannon エントロピーが 7.5 bit/byte を超えて上がる」のいずれか。設定キーは `sync.breaker.window_s` (60)、`sync.breaker.max_files` (500)、`sync.breaker.max_bytes` (1 GiB)、`sync.breaker.entropy_jump_ratio` (0.3)。P4 のランサム演習の実測で Q-SY-2 として見直す。
  - 保持受領: `SyncMode::Full` (`synergos-core/src/project.rs:27-34`、既定値 `:76`) のプロジェクトは、保持可能ノードの `RetentionAck` を方針の範囲内で 1 件以上持たない限り、publish を staging に留めて伝播しない。
  - manifest の更新は意図 journal で行い、全体の書換えをしない (SEC-IN-08)。manifest の保存形式そのものは転送効率文書と AI・ゲーム開発文書が持つ。
  - checkout は再開可能で検証付きの job にする (SEC-IN-15)。対象 manifest は内容がそろい検証が済んでから反映する。
- 不変条件: 受信による置換は必ず旧バイトを 14 日以上残す。全書込は単一の耐久書込関数を通る (`synergos-custody/src/durable_file.rs` の既存手順を一般化)。
- 失敗時: `displaced/` の容量不足は置換を止め、`integrity.displaced_full` を出す。ブレーカー作動は `sync.quarantined{project_ref, files, bytes}`。
- 変更対象: 受信・checkout・restore・manifest の C++ 移植 (`synergos-core/src/exchange/mod.rs`、`checkout.rs`、`project.rs`、`manifest.rs` 相当)、`synergos-net/src/transfer/mod.rs:232-233` 相当の書込経路。
- 担保手段: ランタイム、ローカル検証ゲート (各段の間で kill して再生するクラッシュ注入試験 AT-SY-SEC-16、大量置換の AT-SY-SEC-04、旧バイト保持の AT-SY-SEC-30)。

### 4.15 SECURITY-M15 履歴の保持ロック、GC の正本、copy-not-move、不変複製、scrub

- 区分: C (SPEC-SY-retention-lock は W、SPEC-SY-immutable-replica は A) + I (OS ファイルロック・fsync 無しの GC の解消)。対象発見: SEC-IN-03, SEC-IN-04, SEC-IN-05, SEC-IN-06, SEC-IN-11, SEC-IN-12, SEC-IN-16, SEC-BR-08。フェーズ P5。
- 目的: `index.json` を唯一の生存根拠にする GC (`synergos-core/src/history/store.rs:497-509`)、wire の版番号で操作される保持 (`synergos-core/src/history/gc.rs:37-50`)、move による冷蔵層回転 (`synergos-core/src/history/rotation/mod.rs:451-469`) をやめる。
- 契約 SPEC-SY-retention-lock (正本: 本書):

  > Per-version `{retain_until, mode: governance\|compliance, legal_hold}`. For custody objects the values are set by Curare at admission and commit (SPEC-SY-custody-lifecycle) and custody only enforces them, refusing values below the floor of SPEC-CU-destructive-op (7 d). For history nodes of P2P-only projects the values come from project policy with a 30 d default and a 7 d floor. The source of truth is the sidecar or embedded store, not `index.json`. GC is two-phase mark-and-sweep through trash under a separate identity. Retention is never steered by wire-supplied version numbers. Offload is a **copy** to the immutable tier, never a move.

- 契約 SPEC-SY-immutable-replica (正本: 本書):

  > An append-only replica in a bucket with Versioning and Object Lock (compliance) in a separate account. It is written by a put-only replication identity. The daemon checks `GetObjectLockConfiguration` at start. The rule is that an object reaches the replica before the referencing version row commits.

- 仕組み: 版ごとの sidecar に `{retain_until, mode, legal_hold, sha256, blake3, last_scrubbed_at, scrub_status}` を持ち、GC はこれを正本に 2 段 (mark → ごみ箱 → sweep) で別 ID が行う。custody の値は Curare が設定し (D-25)、custody は値を作らない。GC は OS ファイルロックを取る (現行はプロセス内 mutex だけ、`synergos-core/src/history/store.rs:118-123`。lookup 経路 `:267-268` は無ロック)。冷蔵層への退避はコピーで、復元しても遠隔コピーを消さない。scrub は保管バイトのハッシュを定期に再計算し、不一致で `integrity.violation` を出す。archive 失敗と空き容量の閾値は warn ではなくエラーにする (SEC-IN-16)。custody の保管 root は絶対パス・`.git` の祖先の外・専用ボリュームを起動時に検査する (SEC-IN-11)。
- 不変条件: 保持期限内の版は API でも GC でも消えない。複製に届く前に版の参照を確定しない。
- 失敗時: 不変複製の設定 (Object Lock) が無いと起動しない。scrub の不一致は当該版を配信停止にして警報。保持期限内の版への削除要求は 409 `retention_locked`。
- 変更対象: `synergos-core/src/history/` (store・index・gc・tags・rotation) の C++ 移植 (`synergos-core/native/`、mesh-core)、custody の保管層 (`synergos-custody/native/`、opaque-custody)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-11、AT-SY-SEC-30)。
- 前提: 別アカウントの保管先 (Q5)。未確保の間は P5 を開始できない。

### 4.16 SECURITY-M16 破壊操作ブレーキ (Synergos 側) と custody のライフサイクル

- 区分: C (SPEC-SY-custody-lifecycle、W)。手順の正本は Curare の SPEC-CU-destructive-op。対象発見: SEC-BR-16, SEC-DR-17, SEC-IN-05, SEC-IN-10 (+ SEC-AZ-21 は Curare 正本)。フェーズ P3 (C++)。
- 目的: 即時 unlink の reclaim (`synergos-custody/src/store.rs:210-221`)、全版消去の purge (`synergos-core/src/history/store.rs:427-441`)、1 呼出でアカウント全体の `revoke_dark` (`synergos-control/src/api/reconcile_api.rs:60-61`) を、dry-run・承認・猶予付きの操作に変える。
- 契約 SPEC-SY-custody-lifecycle (正本: 本書):

  > The lifecycle is `admitted → stored → committed → reclaim_requested → purged`. Curare sets `committed` with `POST /v1/custody/objects/{id}/commit {retain_until, mode, legal_hold}`; Curare owns these values and custody only enforces them. Committed objects are never reclaimable through the API while `retain_until` is in the future or `legal_hold` is set. Reclaim requires `{operation_id, reason, actor, step_up_ref, retain_until}` per SPEC-CU-destructive-op and only sets a tombstone with that `retain_until` (custody refuses values below the protocol floor of 7 d). Physical purge is done by a separate GC identity that re-checks references and locks. Reclaiming an id that was never admitted returns 404 with no tombstone. Admissions carry `{org_id, project_ref}`, and the layout is `org/<org>/<shard>/<key>`.

- 仕組み: custody・history purge・tag 削除・control `revoke_dark` の各破壊操作を SPEC-CU-destructive-op の手順 (dry-run → step-up 承認、閾値超で 2 人目 → `retain_until` 付き tombstone → 別 ID の GC → 期限まで復元可) に登録する。登録表は Synergos 側で `spec/data/destructive-ops.json` (新設) に置き、ゲートが網羅を検査する。
- 境界との関係: `spec/feature/opaque-custody.md:41-42` の「Cuが参照のない未確定実体だけをreclaimする」「Cuの業務保持参照をSy独自ポリシーで無視しない」はそのまま守る (D-25)。変えるのは「Syは永続tombstoneを先に記録して削除し」の削除の時点で、即時 unlink から `retain_until` 後の別 ID の GC に移る ([変更提案]、§3.22)。
- 不変条件: committed の実体は保持期限内に API から消せない。物理削除は別 ID だけ。保持の値を custody が自分で決めることはない。
- 失敗時: 承認や step-up の欠けた要求は 403。保持期限内の committed の reclaim は 409 `retention_locked`。未受付 ID の reclaim は 404 で墓標を作らない (現行 `synergos-custody/src/store.rs:79-82` の挙動を変える)。
- 変更対象: custody の C++ 移植 (`synergos-custody/native/`、opaque-custody)、`synergos-core/src/history/store.rs`・`tags.rs` 相当 (mesh-core)、control の `revoke_dark` (control-plane)、`spec/data/destructive-ops.json` (新設、quality-gates)。
- 担保手段: ランタイム、ローカル検証ゲート (破壊操作の登録網羅試験 AT-SY-SEC-17、AT-SY-SEC-11、custody のライフサイクル試験 AT-SY-SEC-18)。

### 4.17 SECURITY-M20 安全な既定設定と起動時 fail-fast (Synergos 側)

- 区分: O + I (NR-5, NR-6)。manifest の形式の正本は Curare (SPEC-CU-secrets-manifest)。対象発見: SEC-SC-10, SEC-SC-12, SEC-SC-18, SEC-CR-06, SEC-CR-07, SEC-CR-17, SEC-CR-20, SEC-IN-11, SEC-DR-08。フェーズ: サイトスクリプトは P0、C++ 実行時は P3/P4。
- 目的: 秘密が複数サービスで値ごと共有され (SEC-DR-08)、片側の未設定が起動後まで見えず (SEC-SC-10)、子プロセスとログへ漏れ (SEC-CR-07, SEC-SC-12, SEC-CR-17)、永続データが checkout と同居する (SEC-SC-18, SEC-IN-11) 状態をやめる。
- 仕組み:
  - `excubitor.secrets.json` (SPEC-CU-secrets-manifest の形式) を本 repo の直下に置き、Synergos の各秘密に所有サービスを 1 つだけ割り当てる。行の例: 発行者鍵の Transit 鍵名 (`synergos-custody-delegation`、`synergos-custody-receipt`、`synergos-control-revocation`。membership と roster の発行鍵は Cernere が持ち、Synergos の manifest には行を作らない。オーナー決定 8)、Cloudflare の最小範囲 token (所有 control-admin)、Curare の SPKI pin (所有 custody-admin)。値は書かず、鍵名・手順・形式・所有者・`consumers[]`・`kid_required` だけを書く。秘密の取得元は Excubitor Vault だけ。
  - 起動検査: loopback 以外の URL は https / wss 必須。Vault の 403 / 404 は致命。代替値への fallback なし。custody 保管 root の検査 (M15)。
  - `Secret<T>` 型と秘匿する formatter を C++ に置き、コマンド payload を Debug 出力しない (NR-6)。子プロセスはすべて `spawn_child()` を通し、環境を空にしてから許可リストだけを足す (NR-5。cloudflared は `synergos-net/src/tunnel/mod.rs:336-347` 相当)。
  - サイト手順は秘密を手順ごとに渡す (`setup` は build に秘密を渡さない、SEC-CR-20)。コード・ビルド出力・永続データを別の場所に置く。既存の `SYNERGOS_SITE_DATA_DIR` (`scripts/site/lib/paths.mjs:9-15`、絶対パスのみ) を本番では必須にする。
- 不変条件: 秘密は 1 サービス 1 用途。`excubitor.secrets.json` の同じ鍵が 2 つのサービスの `consumers[]` に並ばない (G3)。`kid_required` の鍵は kid を持つ。ログに秘密が出ない (種を仕込んだ秘匿試験で 0 件)。
- 失敗時: 欠けた秘密や不正な形式は、鍵名を示して非 0 で終了する。Vault の 403 / 404 では代替値で起動しない。loopback 以外の `http://` 設定は起動を拒否する。秘匿 formatter が失敗した行は捨てて `log.redaction_failed` を数える (秘密を出すより行を失う方を選ぶ)。
- 変更対象: `scripts/site/lib/control-config.mjs`・`scripts/site/run-control.mjs`・`scripts/site/setup.mjs`・`scripts/site/lib/paths.mjs` (P0、control-plane)、`excubitor.secrets.json` (新設、control-plane)、C++ の共通 `config`・`log`・`spawn_child` (`synergos-net/native/`、`synergos-core/native/`)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-26 の起動マトリクスとログ秘匿、AT-SY-SEC-27 の secrets manifest 検査)。

### 4.18 SECURITY-M21 AbusePolicy と quota (Synergos 側)

- 区分: C (SPEC-CU-abuse-policy の Synergos 行) + I (NR-2)。表の形式の正本は Curare。対象発見: SEC-DA-01, SEC-DA-02, SEC-DA-03, SEC-DA-07, SEC-DA-08, SEC-DA-09, SEC-DA-12, SEC-DA-13, SEC-DA-21, SEC-DA-22, SEC-BR-17, SEC-DR-12。フェーズ: P0 (登録ゲートを報告モードで)、C++ 実行時 (P3/P4)。
- 目的: 制限が frame / request 単位の定数だけで、主体単位の quota・レート・挙動点数が無い (SEC-DA-22)。1 peer で daemon を OOM にでき (SEC-DA-02)、control・custody・relay・IPC も同じ構造で飽和する (SEC-DA-12, SEC-DA-13, SEC-DA-09, SEC-DA-21)。
- 仕組み: Synergos の全ハンドラ (HTTP ルート、QUIC stream 種別 TXF2 / SYNC / PLAN / RLY / GSP1 / JOIN1、relay の join と frame、gossip topic、IPC コマンド、失効の入力 API) に `abuse-policy.yaml` の行を置く。行の形は SPEC-CU-abuse-policy に従い、`principal_key` は `peer_id`・`delegation`・`ip`・`token_hash` を使う。C++ 実装は種別ごとの stream 上限、最初のバイトと frame の期限、peer ごとのメモリ予算 (既定 64 MiB、`limits.peer_memory_bytes`)、挙動点数、TransferLedger の退避 (SEC-DA-07)、custody の同時 PUT 上限と償還予算を持つ。heartbeat は鍵検査を先に行う (M04)。C++ の各 binary は起動時に登録済みハンドラの一覧 (`handlers.json`) を出力でき、登録ゲート `scripts/gate/abuse-policy.mjs` がこれと表を突き合わせる。
- 不変条件: 行の無いハンドラは登録ゲートで赤になる。洪水は 429 / RESET で返し、OOM にしない (G8)。予算の超過で daemon 全体を止めない。
- 失敗時: HTTP は 429 と `Retry-After`。QUIC stream は RESET (application error `0x5305 limited`。値は Q-SY-1 で他の code と合わせて確定)。メモリ予算を超えた peer は最も大きい受信バッファの stream から RESET し、プロセスは止めない。挙動点数が閾値を超えた peer は 10 分間 accept で拒否して `peer.throttled` を出す。
- 変更対象: `abuse-policy.yaml` (新設、quality-gates)、`scripts/gate/abuse-policy.mjs` (新設、quality-gates)、C++ の limiter (`synergos-net/native/`、`synergos-core/native/`、`synergos-control/native/`、`synergos-custody/native/`、`synergos-relay/native/`、`synergos-ipc/native/`)。
- 担保手段: ローカル検証ゲート (AT-SY-SEC-17 の登録網羅、AT-SY-SEC-23 の洪水)、ランタイム。

### 4.19 SECURITY-M22 外向き通信の方針 (Synergos 側)

- 区分: C (SPEC-CU-egress-policy の利用側)。正本は Curare。対象発見: SEC-DA-10, SEC-DA-11 (+ SEC-DA-17, SEC-AZ-24 は Curare 正本)。フェーズ P4。
- 目的: 招待・`peer add-url`・`bootstrap_urls` から任意ホストへ GET し、攻撃者指定の UDP へ QUIC 発信を繰り返す (SEC-DA-11)。`/peer-info` が要求ごとに外部 IP-echo を引く (SEC-DA-10)。チャットで流れる招待リンクが作業機を内部網の探査機に変える。
- 仕組み: 招待・bootstrap・`peer add-url` の URL 取得と QUIC 発信先は、接続した後のアドレスで分類器に通す (IANA 特殊用途の範囲、IPv4 射影・NAT64・6to4 を含む)。分類器の試験 vector は Curare から固定 commit で `spec/vectors/egress/` に取り込む。分類済みアドレスへの redirect は追わない。`/peer-info` の外部 IP-echo への問合せ (`synergos-core/src/peer_info_server.rs:137-145`) は削除する (M05)。
- 不変条件: daemon が loopback・RFC1918・メタデータ endpoint へ利用者指定の URL で到達しない (mesh 範囲は明示設定だけ)。
- 失敗時: 分類で拒否した宛先には接続せず、`egress.denied{host_hash, class}` を出し、CLI には分類名を返す。DNS の解決失敗も拒否として扱い、再試行は指数 backoff。
- 変更対象: C++ の `peer_bootstrap`・招待取込・QUIC 発信の前段 (`synergos-core/native/`、`synergos-net/native/`)、`spec/vectors/egress/` (取込)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-28)。

### 4.20 SECURITY-M23 信頼できない decoder の境界と fuzz 登録

- 区分: C (SPEC-SY-decode-bounds、W) + I (NR-3)。対象発見: SEC-DA-19, SEC-DA-02。フェーズ P1。
- 目的: 手組みの 6 形式が版バイトなし・decode してから上限を見る・fuzz 無し (SEC-DA-19) で、1 peer の不正 frame が panic・二乗 decode・巨大確保で全体を止める (SEC-DA-02)。全作業機が他者のバイトを解析するので、ここが崩れると転送の再設計も安全に展開できない。
- 契約 SPEC-SY-decode-bounds (正本: 本書):

  > Every wire and file decoder checks the length against a per-(protocol, version, kind) maximum before allocating, bounds collections during decode, and is listed in `fuzz/registry.json` with a target and corpus. Curare's LCT and receipt verifiers apply the same bounds. The frame layout (ranges, resume) is owned by the transfer doc's framing contract. This contract only fixes the bounds invariants.

- 仕組み: C++ の全 decoder (SESS、TXF2、SYNC、PLAN、RLY、GSP1、JOIN1、relay frame、IPC、LCT、署名封筒、受領証、失効バンドル、node card、enroll / heartbeat の JSON) と保存形式 (manifest、state、history index / sidecar、custody ジャーナル) を `fuzz/registry.json` に `{decoder_id, protocol, version, kind, max_len, max_items, target, corpus_dir, domain}` で登録する。既存の `SPEC-SY-native-framing` (`spec/feature/native-contracts.md:6-13`) は「確保前に長さを拒否」を既に満たしており、その方式を全 decoder に広げる。frame の配置 (範囲・再開・版バイト) は転送効率文書の SPEC-SY-peer-wire が持つ。fuzz はマージごとに各 target を 30 秒 (合計 10 分以内に分割)、夜間に各 1 時間を Excubitor か Concordia のタイマーで回す。
- 不変条件: 長さ検査より前に確保しない。集合の要素数は decode 中に上限で切る。decoder の例外・panic は外へ出さない。
- 失敗時: 上限超過は型付きエラー `decode.too_large{decoder_id, declared, max}` で stream か接続を切断し、確保は行わない。要素数の超過も同じ。不正な形は `decode.malformed{decoder_id}` にする。夜間 fuzz の crash は Actio のタスクに起こし、該当 decoder の corpus に再現入力を足すまでリリースゲートを赤にする。
- 変更対象: 各 decoder の `native/` (network-transport、mesh-core、opaque-custody、control-plane)、`fuzz/registry.json` と `fuzz/<decoder_id>/` (新設、quality-gates)、`scripts/gate/fuzz-registry.mjs` (新設、quality-gates)。
- 担保手段: ローカル検証ゲート (AT-SY-SEC-22 の登録網羅と時間を区切った fuzz)。

### 4.21 SECURITY-M24 取込判定の強制と展開ゲート (Synergos 側)

- 区分: C (SPEC-CU-ingest-verdict の利用側)。判定の正本は Curare。対象発見: SEC-DA-20 (+ AGD-AI-11)。フェーズ P5。
- 目的: 実行可能なアセット形式 (Maya の scriptNode、Blender の Python、`.unitypackage` など) を検査・隔離せず、変換 hook が信頼できないバイトを sandbox なしで解析する (SEC-DA-20)。判定は Curare が持ち (D-15)、Synergos は作業ツリーへの展開時に強制する。
- 仕組み: manifest の項目または版が運ぶ判定 `{scanner_version, ruleset_sha256, verdict, rules_hit[]}` を daemon が読み、作業ツリーへの展開時に強制する。実行ビットは立てない (M13)。`clean` の判定を持たないスクリプト可能な拡張子 (`.ma`、`.mb`、`.blend`、`.unitypackage` など、規則は Curare の `ingest-rules.yaml` を固定 commit で取り込む) は隔離ディレクトリ (`<state_dir>/quarantine/<blake3>`) へ置き、作業ツリーへ出さない。P2P だけのプロジェクトでは、プロジェクト方針で Curare の検査器を呼ぶか、隔離を既定にする。解析器は M12 の sandbox の中だけで動く。
- 不変条件: `clean` の判定の無いスクリプト可能形式は作業ツリーに出ない。Curare 紐付けプロジェクトでは Synergos が判定を作らない。
- 失敗時: 判定の欠落や未知の `ruleset_sha256` は隔離して `ingest.quarantined{project_ref, blake3, reason}` を出す。Curare の検査器に届かない P2P プロジェクトでは隔離を既定にする (fail-closed)。隔離の解除は `publish` 役割の利用者の IPC `QuarantineRelease{blake3}` で、破壊的コマンドと同じ 2 段階 (M11) を要する。
- 変更対象: daemon の受信と checkout の展開経路 (`synergos-core/native/`、mesh-core)、IPC コマンドの追加 (`synergos-ipc/native/`)、`ingest-rules.yaml` の取込。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-35 の合成コーパスでの展開試験)。

### 4.22 SECURITY-M25 セキュリティイベント、custody ジャーナル (Synergos 側)

- 区分: C (SPEC-SY-custody-journal、A)。イベントの形式は Curare の SPEC-CU-security-event。対象発見: SEC-DR-02, SEC-DR-03, SEC-DR-04, SEC-DR-13, SEC-DR-14, SEC-AZ-22, SEC-BR-14, SEC-DA-18。フェーズ P3 (custody ジャーナル)、P4 (daemon)。
- 目的: custody・control・daemon が主体単位で何も記録せず (SEC-DR-04, SEC-AZ-22, SEC-DR-13)、未公開資産の漏えい範囲を帰属できない (SEC-DR-02, SEC-DR-03, SEC-BR-14)。
- 契約 SPEC-SY-custody-journal (正本: 本書):

  > A hash-chained JSONL access journal of SPEC-CU-security-event records (admit, serve, redeem, deny, reclaim, purge, scrub) with signed checkpoints. Ingestion is through `GET /v1/custody/journal?since=seq`.

- 仕組み: custody は全操作を `ludiars.security-event/v1` 形式でハッシュ連鎖の JSONL に書き、署名付き checkpoint (ctx `journal-checkpoint`) を 5 分ごとに打つ。daemon は `<state_dir>/security/*.jsonl` に peer の受理・拒否、受信・送信 (project_ref と実体の帰属付き)、破壊的 IPC、hook 実行を info 以上で書く (現行は debug で文脈なし、`synergos-core/src/daemon.rs:811-814`)。daemon の手元保持は 30 日で、Vestigium へ送ったものから回収する。送信元は `src_ip_prefix` (IPv4 /24、IPv6 /48) で持ち、メールアドレスは持たない。相関は server 生成の W3C trace id、`cap_id = jti`、`operation_id`。クライアントが選んだ request id は `client_request_id` としてだけ保持する。全イベントは Vestigium へ送る (保持は security 400 日、運用ログ 30 日)。機微なルート・コマンド・stream 種別とイベント種別の対応表 `spec/data/event-coverage.json` をゲートが検査する。
- 不変条件: 連鎖の欠けや改変は検証で必ず検出できる。ログに秘密を含めない。記録できない破壊的操作は実行しない。
- 失敗時: Vestigium に届かない間は手元の JSONL に積む (`security.spool_max_bytes`、既定 1 GiB)。上限に達したら運用ログを先に捨て、security イベントは捨てずに新規の破壊的操作を 503 にする。連鎖の検証失敗は `integrity.journal_broken{stream, seq}` を critical で出す。
- 変更対象: custody (`synergos-custody/native/`、opaque-custody)、daemon (`synergos-core/native/`、mesh-core)、control (`synergos-control/native/`、control-plane)、`spec/data/event-coverage.json` と `scripts/gate/event-coverage.mjs` (新設、quality-gates)。
- 担保手段: ローカル検証ゲート (AT-SY-SEC-24 のイベント網羅、AT-SY-SEC-25 の連鎖検証)、ランタイム。

### 4.23 SECURITY-M26 検知ジョブ (Synergos 側)

- 区分: O + C (イベントの利用)。対象発見: SEC-DR-05, SEC-DR-06, SEC-DR-12, SEC-DR-15, SEC-BR-14, SEC-IN-12 (+ SEC-BR-07 は Curare 正本)。フェーズ P6。
- 目的: 量的な流出と侵害の信号が見えない (SEC-DR-05, SEC-DR-06, SEC-BR-14)。認証失敗を捨て (SEC-DR-12)、reconcile と scrub に計画が無い (SEC-DR-15, SEC-IN-12)。
- 仕組み: custody の償還と daemon の送信を `download_ledger(principal, window, objects, bytes, distinct_projects, distinct_ips)` に集計し、7 日の基準線の 5 倍超で警報 → step-up → 絞込み → M07 の freeze と段階的に上げる。canary はプロジェクトのひな形が custody の admission と capability 値に仕込み (Curare 紐付けプロジェクトでは Curare の canary space、P2P だけのプロジェクトでは control-admin が作る)、接触で 60 秒以内に freeze する。control の認証失敗の連続と「連続失敗の後の成功」を規則にする。Excubitor のスケジュールで、control reconcile (報告のみ) を 15 分ごと、custody の全量 scrub を 30 日ごとに回す。警報は Concordia の通知と Actio の課題に自動で起こす。
- 不変条件: 規則を止めても記録 (M25) は止まらない。canary の一覧は control-admin の Vault KV にだけあり、custody と daemon は「どれが canary か」を知らない (接触はジャーナルの照合で判定する)。
- 失敗時: 検知ジョブの最終実行が 5 分より古くなったら `detection.stale` を Concordia に通知する。自動の freeze は canary 接触だけにし、基準線超過は step-up の要求と絞込みまでを自動にする (誤検知で制作を止めないため)。
- 変更対象: 検知ジョブ (`scripts/detect/*.mjs` 新設、Excubitor のスケジュール、control-plane)、control-admin の canary 登録 API (`synergos-control/native/`)、`download_ledger` の保存先 (Vestigium)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-19)。

### 4.24 SECURITY-M27 封筒暗号、テナント分離、チャンク保管の境界

- 区分: C (SPEC-SY-envelope-encryption、A。SPEC-SY-chunk-access、A)。対象発見: SEC-CR-19, SEC-BR-13, SEC-BR-08 (+ SEC-BR-06 は Curare 正本、AGD-OP-09)。フェーズ P5。
- 目的: custody・履歴ノード・冷蔵層 (生の内容ハッシュ名) に保管時暗号化が無く (SEC-CR-19)、組織ごとの保管・鍵の分離が無い (SEC-BR-13、AGD-OP-09)。資格の窃取がそのまま資産の窃取になり、離脱時に消去を証明できない。転送効率文書が独立させたチャンク保管モジュールにも、同じ分離を境界で課す。
- 契約 SPEC-SY-envelope-encryption (正本: 本書):

  > AEAD (XChaCha20-Poly1305 or AES-256-GCM) in STREAM segments of 64 KiB by default. The segment size is a power of two from 16 KiB to 1 MiB, fixed per object, recorded in the object header and aligned to the 16 KiB groups of SPEC-SY-content-identity, so a range read decrypts at most the requested bytes plus two segments. AD is `{org, project_ref, blake3, size, segment_size}`. There is a per-project KEK in Excubitor Vault Transit. The whole-object DEK is wrapped by the KEK and stored in the admission or manifest entry. The chunk DEK is `HKDF(project_chunk_key, chunk_blake3)`. Storage names are `HMAC(project_key, blake3)`. Unwrap requires an authorized `operation_id` or `jti` and is logged. Off-boarding is a freeze followed by KEK destroy after the retention lock.

- 契約 SPEC-SY-chunk-access (正本: 本書。追補 X5 により無条件):

  > The security boundary of the chunk-store module (SPEC-SY-chunk-store, owned by the transfer doc). Every `put`, `has`, `get_range`, `chunk_map` and `export` call carries a ctx `{org, project_ref}` and an LCT (membership or custody-capability) whose caveats name the manifest or object set; the module verifies LCTs and never issues them. The dedup domain is the project, the org only by opt-in, never global. Chunk ids on the wire and in listings are `HMAC(project_dedup_key, chunk_blake3)`; there is no cross-project listing, and `has` answers only inside the caller's dedup domain. Chunk records reference wrapped keys, and chunk DEKs follow SPEC-SY-envelope-encryption. `gc` runs under a separate identity and honours SPEC-SY-retention-lock. Every served range emits `content.served`.

- 仕組み: custody と MinIO を先に、履歴ノードを後に暗号化する。custody の保管配置は `org/<org>/<shard>/<key>` (SPEC-SY-custody-lifecycle) で、保管名は `HMAC(project_key, blake3)`。冷蔵層 (S3/GDrive) は現行の生の内容ハッシュ名 (`synergos-core/src/history/rotation/s3.rs:66-70`) をやめる。segment は既定 64 KiB で、転送効率文書の Bao の 16 KiB group と TXF2 の frame (64 KiB / 256 KiB / 1 MiB) の境界に揃うので、範囲読取の復号量は E7 に収まる。チャンク保管モジュール `synergos-chunkstore` (`synergos-chunkstore/native/`、ドメイン `chunk-store`) は seed / archive / custody / edge だけが link し、member は link しない。モジュールの API・chunker・pack 形式は転送効率文書の SPEC-SY-chunk-store が持つ。`project_key` と `project_dedup_key` と `project_chunk_key` は、プロジェクトごとの root secret (Vault KV、所有は custody-admin、読めるのは custody と chunk-store の Vault policy だけ) から HKDF で用途ごとに導出する。離脱は freeze → 保持ロック明け → KEK 破棄 (crypto-shred)。
- 不変条件: 重複排除の範囲はプロジェクト (組織は opt-in、Q6)。KEK の unwrap は認可済みの `operation_id` か `jti` を伴い、記録される。平文のチャンクや実体を保管層に書かない。
- 失敗時: KEK (Vault Transit) に届かなければ admit を 503 `kek_unavailable` にし、平文で保存する代替をしない。unwrap の拒否や失敗は配信を 503 にして `crypto.unwrap_denied` を出す。KEK を破棄したプロジェクトへの要求は 410 `crypto_shredded`。チャンク保管モジュールが使えない場合は転送効率文書の規定どおり実体全体の TXF2 に戻り、平文チャンクは作らない。
- 変更対象: `synergos-custody/native/` (opaque-custody)、`synergos-core/native/` の履歴と冷蔵層 (mesh-core)、`synergos-chunkstore/native/` (新設、chunk-store)、Vault Transit と KV の鍵定義 (`excubitor.secrets.json`、control-plane)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-20、AT-SY-SEC-38、AT-SY-EFF-03 の E5、AT-SY-EFF-06 の E7)。

### 4.25 SECURITY-M28 バックアップ・不変層・復旧訓練・サイト bundle v2 (Synergos 側)

- 区分: O + C (SPEC-SY-immutable-replica、SPEC-CU-site-bundle-v2 の Synergos 出力)。対象発見: SEC-DR-09, SEC-IN-13, SEC-IN-06, SEC-SC-11, SEC-CR-21。フェーズ P5。
- 目的: バックアップ・不変コピー・復旧訓練が無く (SEC-DR-09, SEC-IN-13)、冷蔵層の回転が move で (SEC-IN-06)、拠点移転の bundle が全資産史の平文 1 ファイルになる (SEC-SC-11, SEC-CR-21)。ランサム時に身代金を拒否できる根拠を作る。
- 仕組み: custody と履歴の不変複製 (M15)。`scripts/site/export-data.mjs`・`scripts/site/import-data.mjs`・`scripts/site/lib/dir-bundle.mjs` の bundle を SPEC-CU-site-bundle-v2 (宛先サイトの鍵へ暗号化、Ed25519 署名ヘッダ、秘密を含めない、0600) に置き換える。現行の bundle はメール入りの登録簿と全 custody 実体を平文で持つ (`scripts/site/export-data.mjs:9-10`)。月次の復旧訓練ジョブ (`scripts/site/restore-drill.mjs` 新設) が、不変複製から隔離環境へ復元して全実体のハッシュを照合し、成果物 `{drill_id, started_at, rto_s, rpo_s, objects_checked, mismatches, signer_kid, sig}` を Excubitor のジョブ成果物として署名付きで残す。
- 不変条件: bundle に秘密を入れない。復号できるのは宛先サイトの鍵だけ。不変複製に書けるのは put 専用の複製 ID だけ。
- 失敗時: 無署名・format 1・未登録鍵の bundle の import は、移行期間の後は拒否する。最後の訓練が 35 日より古いか、不一致が 1 件以上ならリリースゲートを赤にする。不変複製への書込に失敗した版は commit しない (M15 の規則)。
- 変更対象: `scripts/site/export-data.mjs`・`scripts/site/import-data.mjs`・`scripts/site/lib/dir-bundle.mjs`・`scripts/site/restore-drill.mjs` (control-plane)、custody と履歴の複製 (`synergos-custody/native/`、`synergos-core/native/`)。
- 担保手段: ランタイム、ローカル検証ゲート (AT-SY-SEC-36 の訓練成果物の鮮度と bundle の拒否)。

### 4.26 SECURITY-M29 マージ前の供給網ゲート (Synergos 側)

- 区分: O。対象発見: SEC-SC-01, SEC-SC-02, SEC-SC-03, SEC-SC-05, SEC-SC-13, SEC-SC-16, SEC-SC-17。フェーズ P0。
- 目的: GitHub Actions は Revisor のマージ後にしか走らず、RULE.md §6.5 は workflow を禁じる (SEC-SC-01)。AIFormat の再利用 workflow を可変参照で呼び (SEC-SC-02)、可変タグと prefix 復元の cache を使い (SEC-SC-03)、依存の admission 方針も secret scan も無い (SEC-SC-16, SEC-SC-17)。ほぼ AI が書くリポで、幻覚や typo-squat の依存名が無検査で入る。
- 仕組み: `scripts/gate/supply-chain.mjs` を新設し、Revisor の必須マージ前ケースに登録する。検査項目は (1) `.github/workflows/*` が存在しない、(2) 確定した `deny.toml` に対する `cargo deny check sources bans licenses`、(3) 差分の secret scan、(4) 新規 package ごとの `deps-approvals.json` 行と公開から 7 日未満の版の拒否、(5) `rust-toolchain.toml` の固定。日次の advisory 実行は Excubitor か Concordia のタイマーで回し、Actio のタスクに起こす。サーバ成果物から到達する High / Critical の advisory が未抑止ならリリースを止める。現行 workflow が担っていた Rust のテスト・clippy・native CMake は、`cpp-migration.md:93-98` が Revisor ケースとして登録すると定めた構成 (Rust の compile と実行の分割、CMake、native CTest) に一本化する。
- workflow の削除の条件: `cpp-migration.md:97-98`「Registration alone does not prove they passed in Revisor.」に従い、`ci.yml`・`harness.yml`・`native.yml` の削除は、置き換えの Revisor ケース (Rust の compile と実行、clippy、CMake、native CTest、supply-chain) が main で 1 回以上通った記録を確かめた後の別の変更で行う。
- 不変条件: workflow ファイルは 0 本。依存の追加は `deps-approvals.json` の行と同じ変更でしか入らない。
- 失敗時: ゲートが赤ならマージしない。日次 advisory の失敗はマージを止めず Actio のタスクにする。リリースの判定だけが、サーバ成果物から到達する High / Critical で止まる。
- 変更対象: `scripts/gate/supply-chain.mjs`・`deny.toml`・`deps-approvals.json`・`rust-toolchain.toml` (新設、quality-gates)、`.github/workflows/*` (削除)。
- 担保手段: ローカル検証ゲート、フック (ハーネスの事前検査)。

### 4.27 SECURITY-M30 quick-xml の経路除去 (workspace 分割と eframe の feature)

- 区分: O (workspace 構成)。オーナー決定 6 で禁じた 3 件には当たらない。対象発見: SEC-SC-05。フェーズ P0。
- 目的: RUSTSEC-2026-0194 / 0195 (`quick-xml 0.30.0`) の経路 (`synergos-gui` → eframe の `accesskit` feature → atspi → `zbus_xml`) を、除外ではなく依存ごと消す (オーナー決定 5、D-13)。
- 仕組み: `Cargo.toml:2-12` の workspace を `server/` (synergos-net、-ipc、-core、-relay、-control、-custody、ars-plugin-synergos) と `desktop/` (synergos-gui、synergos-tauri/src-tauri。synergos-ipc は path 依存で共有) に分け、lockfile と audit を別にする。`desktop/` の `synergos-gui` は `eframe = { version = "0.31", default-features = false, features = ["default_fonts", "glow", "wayland", "x11"] }` にする。P0 で `cargo tree -e features -i quick-xml` を両 workspace で実行して消滅を確かめる。audit はサーバ workspace で必須、desktop workspace は報告。除外指定 (`audit.toml`) は使わない。egui バイナリは C++ desktop-gui が受入ゲートを通るまで `desktop/` で build できる状態に保ち (`cpp-migration.md:48` の受入面と parity の基準)、通った時点で撤去する。Q3 で利用者がいないと分かれば、それより前に撤去してよい。
- 文書の更新 (P0): `README.md:18`, `:28`, `:49`, `:154`、`docs/getting-started.md:18`, `:56`, `:122`、`docs/setup-macos.md:51`, `:67`, `:298-304`、`docs/platforms.md:121` の `synergos-gui` の案内を、新しい build 手順 (`cargo build --manifest-path desktop/Cargo.toml -p synergos-gui`) と「C++ desktop-gui で置き換える予定」の注記に改める。`docs/setup-macos.md:65-67` の Gatekeeper 回避の手順は M31 で消す。
- 不変条件: サーバ成果物の依存グラフに GUI 系の crate が入らない。`audit.toml` の除外を置かない。
- 失敗時: feature を外しても経路が残る場合 (未確認。`cargo tree` を P0 で実行するまで分からない) は、egui バイナリを配布物から外して parity の基準としてだけ build し、`desktop/` の audit に赤のまま残す。workspace の分割で Rust の Revisor ケースが壊れたら、分割を 1 commit で戻す。
- 変更対象: `Cargo.toml`、`server/Cargo.toml` と `desktop/Cargo.toml` (新設)、`synergos-gui/Cargo.toml` (desktop-gui)、Rust の Revisor ケースの定義 (`cpp-migration.md:93-98`)、上記の文書。
- 担保手段: ローカル検証ゲート (AT-SY-SEC-33)。

### 4.28 SECURITY-M31 隔離ビルドと署名付き成果物 (Synergos 側)

- 区分: O + I。対象発見: SEC-SC-06, SEC-SC-07, SEC-SC-08, SEC-SC-14, SEC-CR-18。フェーズ P6。
- 目的: 本番サイトで 866 crate の build.rs と proc-macro を Vault の資格に届く環境でコンパイルし (SEC-SC-06)、成果物に署名・checksum・SBOM が無く (SEC-SC-07)、cloudflared を無検証で入れ (SEC-SC-08)、admin-ui と Tauri の配布に完全性が無い (SEC-SC-14, SEC-CR-18)。
- 仕組み: ビルドは offline・frozen で、ビルド環境に Vault の資格を置かない。成果物ごとに `build-info.json`、CycloneDX SBOM、minisign か Sigstore の署名、`SHA256SUMS` を作る。サイトはダウンロードして検証するだけで、コンパイルしない (現行 `scripts/site/setup.mjs:7-11` はサイトで全依存をビルドする)。version は health と heartbeat で報告する (現行は `0.1.0` を自己申告、`synergos-core/src/control_heartbeat.rs:107`)。admin-ui の bundle は署名付き manifest で配る。Tauri の updater 鍵は Vault に置き、Tauri に CSP を設定する。cloudflared は `{version, sha256}` で固定する。
- 不変条件: サイトでコンパイルしない。署名の検証に通らない成果物は配置しない。
- 失敗時: 署名・checksum の不一致は配置を中止して旧版を動かし続け、`release.verify_failed` を出す。
- 変更対象: `scripts/site/setup.mjs` (control-plane)、ビルドジョブ (Excubitor か Concordia のタイマー)、`synergos-tauri/src-tauri/tauri.conf.json` (desktop-gui)、admin-ui の配布 manifest (admin-ui)、`docs/setup-macos.md` と `docs/platforms.md` の導入手順。
- 担保手段: ローカル検証ゲート、ランタイム (サイトでの署名検証、AT-SY-SEC-37)。

### 4.29 SECURITY-M32 HTTP の基線 (Synergos 側)

- 区分: I。対象発見: SEC-DR-10, SEC-DR-11, SEC-AZ-25, SEC-CR-18, SEC-DA-18。フェーズ P3 (C++ の HTTP)。
- 目的: health が version を返さず (SEC-DR-10、RULE_SRE §2)、admin UI に CSP・HSTS・Host / Origin 検査が無い (SEC-DR-11, SEC-AZ-25)。
- 仕組み: `/v1/health` は I/O なしで `{ok, service, version}` を返す (現行は version 欠落、`synergos-control/src/api/mod.rs:100-105`)。readiness は別 endpoint で、ネットワークを限定する。CSP は `default-src 'self'; frame-ancestors 'none'; base-uri 'none'` (Dioxus 用に `wasm-unsafe-eval`)。HSTS、Host の許可リスト (違反は 421)、GET 以外での Origin 検査。custody の GET は `nosniff` と `CSP: sandbox` を付ける。
- 不変条件: health は I/O をしない。全 HTML 応答が CSP を持つ。
- 失敗時: Host の違反は 421、Origin の違反は 403。readiness は依存が欠ければ 503 を返し、health は 200 のまま。
- 変更対象: control-admin・control-node・custody の HTTP 層 (`synergos-control/native/`、`synergos-custody/native/`)、`synergos-admin-ui` の配信設定 (admin-ui)。
- 担保手段: ローカル検証ゲート (AT-SY-SEC-29 のヘッダ回帰試験)。

### 4.30 SECURITY-M33 運用文書と演習 (Synergos 側)

- 区分: O。対象発見: SEC-DR-16, SEC-DR-08。フェーズ P0。
- 目的: RULE_SRE §1 の `spec/operations.md` と、資格漏えい・侵害ノード・悪性送信・資産漏えい・ランサムの runbook が無い (SEC-DR-16)。秘密ごとのローテーション手順も無い (SEC-DR-08)。
- 仕組み: `spec/operations.md` を新設し、重大度表、証拠保全、M07 への封じ込め手順、秘密ごとのローテーション、資格漏えい・侵害ノード・悪性送信・資産漏えい・ランサムの runbook、control-node の多重化と復旧手順 (G11)、オフライン貸与の受け入れ手順 (D-21) を書く。組織共通の `incident-playbooks.md` を参照する。四半期ごとに staging で AI が実行する机上演習を行う。HLO1 から `synergos/2` への切替日 (flag day) は組織ごとに本文書で告知する。
- 不変条件: `excubitor.secrets.json` の全鍵に playbook の見出しがある。
- 失敗時: 見出しの欠けた鍵があればゲートを赤にする。前回の机上演習から 100 日を超えたら Actio にタスクを起こす。
- 変更対象: `spec/operations.md` (新設。control-plane の `specRefs` に足す)、`docs/site-operations.md`、`docs/mesh-operations.md`、`scripts/gate/playbook-coverage.mjs` (新設、quality-gates)。
- 担保手段: 記憶、ローカル検証ゲート。

### 4.31 SECURITY-M34 現行 Rust 実行時への暫定補償 (設定だけ)

- 区分: O。コード変更なし (オーナー決定 6)。対象発見: SEC-AZ-07, SEC-BR-08, SEC-BR-10, SEC-IN-01, SEC-CR-06, SEC-AZ-01, SEC-CR-05 (Codex 経路) (+ AGD-AI-12)。フェーズ P0。
- 目的: C++ 移行が終わるまでの被害半径を、既存の設定項目だけで縮める。下表の設定項目はすべて d404f20 の現行コードに存在することを確認した。存在しない項目は書かない。

| 補償 | 既存の設定項目 (根拠) | 推奨値 | 効く発見 |
|---|---|---|---|
| control (custody を同居) を loopback に限る | `SYNERGOS_CONTROL_BIND` → `bind_addr` (`scripts/site/lib/control-config.mjs:18`、既定 `127.0.0.1:4250`。`synergos-control/src/config.rs:14-16` も既定は loopback) | loopback のまま。外へ出す経路は TLS 終端の前段だけにし、パスで transfer と管理を分ける (次の行)。文書の `0.0.0.0` / `http://` の指示 (`docs/mesh-operations.md:126-130`、`docs/mesh-two-node-checklist.md:48-54`) を改める | SEC-CR-06, SEC-AZ-07 |
| Codex クラウドからの custody 受け渡し (2026-10-23 neco 判断。Curare: `spec/feature/codex-cloud-access.md:10-16`, `:29`, `:34-36`) | Synergos 側の設定項目: `SYNERGOS_CUSTODY_PUBLIC_URL` (tailnet の 4443 の URL)。`tailscale serve` と Tailscale ACL は Synergos の設定ではなく拠点の設定。GET URL の期限を縮める Synergos の設定項目は**無い**: custody は要求ごとの `expires_in` を `1..=900` で受け (`synergos-custody/src/handlers.rs:153`)、PUT URL は 900 秒で切る (`:98`)。期限を決めるのは Curare の要求値 | (1) `tailscale serve` はパス接頭辞 `/v1/custody/transfer/` だけを出す (custody の公開経路は `synergos-custody/src/lib.rs:42-43` の `/v1/custody/transfer/:key` だけで、管理経路 `:30-35` は bearer の層の内側)。ルート `/` を serve しない。(2) Tailscale ACL は `tag:codex` → 本社 4443 / 6443 だけ。(3) 鍵は ephemeral・tag 付き・再利用不可。(4) Curare の取得要求の期限は 300 秒のまま延ばさない (Curare 文書へ申し送り)。(5) Excubitor の定期ジョブで `tailscale serve status` と ACL を取得し、上記からの差分を Concordia に警報する | SEC-CR-05, SEC-BR-15, AGD-AI-12 |
| relay を配備しない | `synergos-relay/src/config.rs:34` の既定 `0.0.0.0:8080` | Rust の relay は起動しない (オーナー決定 7: relay は C++ だけ)。到達は Tailscale で行う | SEC-AZ-08, SEC-CR-14, SEC-DA-09 |
| `/peer-info` を公開しない | `peer_info_listen_addr: Option<SocketAddr>` (`synergos-net/src/config.rs:25-30`、既定 `None` で起動しない) | `None` のまま。公開ノードでも mesh 内アドレスだけ。`docs/mesh-two-node-checklist.md:48-54` の `peer_info_listen_addr = "0.0.0.0:7780"` と `peer_info_advertised_url = "http://<A_MESH_IP>:7780"` の指示を、mesh 内アドレスへの bind と「mesh 外へ出さない」注記に改める。公開する場合も tailnet IP だけに bind する | SEC-AZ-07, SEC-DA-10, SEC-CR-02 |
| QUIC を tailnet IP に限る | `quic.listen_addr` (`synergos-net/src/config.rs:588-593`、既定は `[::]:0`) | `quic.listen_addr` に tailnet IP を明示する (Synergos `docs/site-operations.md`)。`[::]` や `0.0.0.0` にしない | SEC-AZ-01, SEC-DA-01 |
| 履歴ノードのプロジェクトを明示 | `history.projects` (`synergos-net/src/config.rs:269`、既定 `["*"]`、`:453-455`) | 対象プロジェクトを列挙し `"*"` を使わない | SEC-BR-08 |
| post-receive hook を止める | `hooks.allow_project_hooks` (既定 false) と `hooks.hooks` (`synergos-net/src/config.rs:99-108`) | `allow_project_hooks = false` を維持し、`hooks.hooks` に `post-receive` を置かない | SEC-BR-10, SEC-DA-06, SEC-AZ-10 |
| 外部メンバーや高価値資産のプロジェクトで自動同期を止める | プロジェクト設定 `sync_mode` (`synergos-core/src/project.rs:27-34`、既定 `Full` は `:76`。CLI `synergos-core/src/cli.rs:98`) | `manual` | SEC-IN-01 |
| 招待に期限を付ける | `project invite --expires <秒>` (`synergos-core/src/cli.rs:105-110`、未指定は無期限 `synergos-core/src/invite_token.rs:44-46`) | 86400 以下を運用規則にする | SEC-CR-03, SEC-AZ-03 |
| custody の保管先を checkout の外へ | `SYNERGOS_SITE_DATA_DIR` (`scripts/site/lib/paths.mjs:9-15`、絶対パスのみ) と `SYNERGOS_CUSTODY_ROOT` (`scripts/site/run-control.mjs:16`) | 専用ボリュームの絶対パスを指定 | SEC-IN-11, SEC-SC-18 |

- 暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。
- 会員と名簿 (オーナー決定 8): Cernere の SPEC-CR-membership-issuer / SPEC-CR-node-roster が提供されるまで、Synergos 側に会員・名簿の暫定 (control が持つ発行鍵など) は置かない。この間の会員の境界は、上の Tailscale ACL と tailnet IP への bind だけで、データプレーンの会員検査は C++ 移行と Cernere の提供まで無い。
- 設定項目が無く暫定補償できないもの (P4 以降へ送る): HLO1 の再生 (SEC-CR-01)、データプレーンの会員検査 (SEC-AZ-01 の本体)、IPC の uid 全権 (SEC-AZ-09)、受信時の旧バイト保持 (SEC-IN-01 の本体)、Codex 経路での HMAC URL の期限内再生 (SEC-CR-05。期限内の URL は tailnet 上で再生できる)。これらは C++ 移行まで設定では塞げない。上表はネットワーク到達とノード構成で被害半径を狭めるだけである。
- 不変条件: 管理経路 (`/v1/custody/*` の受け渡し以外、`/v1/orgs` など) は loopback 以外に出ない。Rust の relay は動かさず、relay は C++ relay が受入ゲートを通ってから配備する (オーナー決定 7)。`synergos-node` の QUIC は tailnet IP 以外に bind しない。
- 失敗時: 推奨値から外れた設定 (`0.0.0.0` / `[::]` の bind、`"*"` の履歴ノード、`/` の serve) を Excubitor の定期ジョブが見つけたら Concordia に警報し、運用者が戻すまで当該拠点を「暫定補償なし」として `spec/operations.md` の台帳に記録する。自動では書き換えない (共有基盤の設定は運用者の判断)。
- 変更対象: `docs/mesh-operations.md:126-130`、`docs/mesh-two-node-checklist.md:48-54` (`0.0.0.0` の `/peer-info` と `http://` の bootstrap の指示を mesh 内アドレスと loopback に改める)、`docs/site-operations.md`、Excubitor のサイト上書き設定、Excubitor の設定監視ジョブ。
- 担保手段: 記憶 (`docs/site-operations.md` と `spec/operations.md` の推奨設定)、ランタイム (設定値と監視ジョブ)。

### 4.32 ネイティブ実装要件 (実装の非効率。契約は変えない)

| ID | 要件 | 対応する現行の欠陥 |
|---|---|---|
| NR-1 | accept を直列にしない。`Incoming` ごとに 1 task とハンドシェイク期限 | `synergos-net/src/quic/mod.rs:445-465` (SEC-DA-01、TRN-TP-15) |
| NR-2 | peer 単位・種別単位の stream 予算。quinn 相当の窓は既定のままにせず設定する。転送の調整値は転送効率文書が持ち、本書は予算の下限だけを定める | `synergos-core/src/daemon.rs:806`、`synergos-net/src/config.rs:694-695` (SEC-DA-02) |
| NR-3 | 境界付きの decode (SPEC-SY-decode-bounds)。上限検査前に decode しない | `synergos-net/src/content/bitswap.rs:237` (SEC-DA-19) |
| NR-4 | fsync 付きの単一耐久書込。大域ロック下で検証のたびに全体を再ハッシュしない (転送効率文書と相互参照) | `synergos-net/src/transfer/mod.rs:232-233`、`synergos-custody/src/store.rs:153` (SEC-IN-07, SEC-DA-13) |
| NR-5 | 子プロセスはすべて `spawn_child()` を通す。環境を継承しない | `synergos-core/src/hooks/runner.rs:315-325`、`synergos-net/src/tunnel/mod.rs:336-347` (SEC-CR-07) |
| NR-6 | `Secret<T>` と秘匿する formatter。コマンド payload を Debug ログに出さない | `synergos-core/src/ipc_server.rs:504` (SEC-CR-17) |
| NR-7 | エラーは型付き。文字列でエラーを分類しない | `synergos-net/src/quic/mod.rs:266-271` (SEC-CR-13) |
| NR-8 | 検証キャッシュは (jti, block hash) を鍵にする | E3 |

### 4.33 参照する Curare 側の契約 (正本は Curare: spec/plan/redesign-security.md)

本書はこれらを利用するだけで、規範文は Curare 文書にある。本書は定義を写さず、要約と Synergos での使い方だけを書く (1 機能 1 正本)。要約は非規範で、差分があれば正本が優先する。

| 契約 ID | 正本の節 (Curare 文書) | 要約 (非規範) | Synergos での使い方 |
|---|---|---|---|
| SPEC-CU-security-event | §4.12 (M25) | `ludiars.security-event/v1` の行形式、`{seq, prev_hash, row_hash}` の連鎖、ctx `journal-checkpoint` の署名 checkpoint、保持 400 日 / 30 日、イベント網羅表 | custody ジャーナル (SPEC-SY-custody-journal) と daemon の `<state_dir>/security/*.jsonl` がこの形で書く。網羅表は `spec/data/event-coverage.json` |
| SPEC-CU-abuse-policy | §4.9 (M21) | `abuse-policy.yaml` の行 `{handler_id, surface, principal_key, rate, burst, concurrency, max_body_bytes, max_persisted_bytes, timeout_ms, on_violation}` と、行の無いハンドラを赤にする repo ごとの登録試験 | Synergos の全ハンドラ (HTTP、QUIC stream 種別、gossip topic、IPC、relay) が行を持つ (M21) |
| SPEC-CU-egress-policy | §4.10 (M22) | 接続後のアドレスを IANA 特殊用途表で分類する 1 つの分類器、分類済みアドレスへの redirect 拒否、運用者だけの内部許可リスト、共有試験 vector | 招待・bootstrap・`peer add-url` の取得先判定 (M05, M22)。vector を固定 commit で取り込む |
| SPEC-CU-destructive-op | §4.6 (M16) | request → dry-run → step-up 承認 (閾値超で 2 人目) → `retain_until` 付き tombstone (既定 30 日、下限 7 日) → 別 ID の GC → 期限まで復元可、登録網羅のゲート | custody reclaim、history purge / GC / tag 削除、control `revoke_dark`、IPC の破壊的コマンドが従う (M16)。custody は下限 7 日を契約として強制する |
| SPEC-CU-quota-reservation | §4.9 (M21) | admission が Curare 所有の quota に対して `{org, project, bytes}` を予約し、custody と daemon が同じ上限を強制する | M13 の受信前予約、custody の admission |
| SPEC-CU-ingest-verdict | §4.11 (M24) | 版が取得可能になる前の `scanning → clean \| quarantined → review` と、版・manifest 項目が運ぶ判定 | M24 の展開ゲートが読む |
| SPEC-CU-secrets-manifest | §4.8 (M20) | committed の `excubitor.secrets.json` (鍵名・手順・形式・所有者・consumers・kid 要否)、起動時の形式検査、Vault の 403 / 404 は致命、Excubitor Vault だけ | 本 repo も同じ形式の `excubitor.secrets.json` を持つ (M20) |
| SPEC-CU-site-bundle-v2 | §4.15 (M28) | 宛先サイトの鍵への暗号化 (age / HPKE)、Ed25519 署名ヘッダ、秘密を含めない、0600 | `scripts/site/export-data.mjs` の出力をこの形にする (M28) |
| SPEC-CU-membership-export | §4.5 (M10) | 権威の経路としては廃止。Curare は Synergos の会員・ノード名簿を輸出・発行・判定せず、権威としてキャッシュしない。Curare 紐付けの scope では、作成時に作成者本人の権限で `project_ref` を Cernere に登録し、UI での会員変更を操作者本人として Cernere へ取り次ぎ、Cernere 署名の ProjectRoster を読んで表示と Curare の権限の上限に使う | Synergos は使わない。control は Curare から会員資格について何も受け取らない。Synergos のプロジェクト会員の正本は Cernere (SPEC-CR-membership-issuer)。Curare は会員を輸出・発行・判定しない。Curare 紐付けの scope では、作成者の権限で `project_ref` を Cernere に登録し、Curare の UI での会員変更は操作者本人として Cernere へ取り次ぐだけ (SPEC-CU-membership-export の改訂後の定義)。(D-19) |
| SPEC-CU-capability-ledger | §4.2 (M18) | `capability_issuance(jti, …)` の発行台帳、≤1000 実体の集合スコープ LCT (D-18)、主体ごとの予算と 5 倍超の step-up | custody ジャーナルの jti と照合して `capability.orphan` を出す (M09) |
| SPEC-CU-ssf-receiver | §4.1 (M17) | Cernere の SET を検証し、actor キャッシュの無効化・admission の取消・magic link の失効を行い、利用者の未失効 jti と `cap_issued_before` を control へ送る | 送り先は本書が正本の入力 API `POST /v1/orgs/{org}/revocation/inputs` (M07)。Curare 文書 §4.4 はこの名前を参照する形に改める |

### 4.34 Cernere への依存 (SPEC-CR-*、オーナー決定 4。Synergos / Curare には実装しない)

既存の 5 本の規範文は Cernere への依頼 (Actio に登録したタスク) が持ち、本書は要約だけを書く (非規範)。オーナー決定 8 で足した SPEC-CR-membership-issuer と SPEC-CR-node-roster は、依頼の規範文の正本を本書 (§4.34.1、§4.34.2) に置き、Actio で Cernere へ送った。SPEC-CR-membership-issuer と SPEC-CR-node-roster は Actio `actio:fd0e4de8`、鍵の公開・SSF/CAEP・step-up は `actio:2a7af0fa` が運ぶ。

| 契約 ID | 状態 | 要約 (非規範) | 本書での用途 | 提供前の暫定 |
|---|---|---|---|---|
| SPEC-CR-capability-signing-keys | 依頼済み (Actio に登録) | kid とローテーション付きの Cernere JWKS と、Cernere が署名する鍵委任記録 `{issuer, pubkey, kid, orgs[], purpose, nbf, exp}`。鍵委任記録の purpose から `membership` を外し、custody-delegation・受領証・失効 (deny だけ) を残す。Cernere 自身の `membership` と `roster` の kid を JWKS に載せる (user JWT の鍵とは分ける) | custody の発行鍵と control の deny 専用失効鍵の認証 (D-02)。PEP が発行者鍵を読むときに確かめる。membership LCT・node card・ProjectRoster・RosterStatus・OfflineLease の検証鍵 | 発行者の公開鍵を Excubitor Vault に pin (membership と roster は暫定なし) |
| SPEC-CR-ssf-caep-transmitter | 依頼済み | `session-revoked`・`credential-change`・`assurance-level-change`・拡張 `org-membership-removed` の SET の push。`project-membership-changed`・`node-revoked`・`roster-changed` を追加する | 失効バンドルの入力 (M07)、PEP の即時取得 (`roster-changed`) | Curare が 60 秒ごとに所属を照会。利用者除名の G1 を満たさない既知の穴 |
| SPEC-CR-step-up-assertion | 依頼済み | 1 つの操作に束縛した新しい assertion `{sub, amr, auth_time, operation_id}`。用途に claim code・役割と scope の再発行・publish / history の付与・OfflineLease を足す | 事件 API、`revoke_dark`、破壊操作。Cernere 側の claim code 発行・card の役割と scope の変更・publish / history の付与・オフライン貸与の発行 | step-up の代わりに 2 名承認 |
| SPEC-CR-membership-issuer | 新規依頼 (Actio `actio:fd0e4de8`) | 下の引用 (§4.34.1) | membership LCT・ProjectRoster・OfflineLease の唯一の発行者。招待の判定 (`member-requests`) | **なし**。P3 の会員資格の部分と P4 は提供を待つ |
| SPEC-CR-node-roster | 新規依頼 (Actio `actio:fd0e4de8`) | 下の引用 (§4.34.2) | claim code・enrollment の承認・node card・RosterStatus の唯一の発行者 | **なし**。P3 の enrollment の部分と P4 は提供を待つ |

2 本の新しい契約に共通の規則: Cernere の Ed25519 kid は `purpose: membership` と `purpose: roster` の専用で、ローテーション付きで JWKS に公開し、user JWT の鍵と分ける。Cernere は Synergos の golden vector の conformance ケースを固定 commit で走らせる。変更のたびに SSF イベント `project-membership-changed`・`node-revoked`・`roster-changed` を出す。署名する物にメールアドレスを入れない。

**2 本の契約の提供前の暫定: Synergos には無い。** P3 (会員資格と enrollment の部分) と P4 は Cernere を待つ。それまで拠点は現行 Rust 実行時を M34 の設定で Tailscale 越しに動かす。Synergos のどの部品も membership・roster の鍵を持たない。

#### 4.34.1 SPEC-CR-membership-issuer (依頼の規範文。正本: 本書)

> **SPEC-CR-membership-issuer** (request to Cernere; 正本: Synergos `spec/plan/redesign-security.md` §4.34) — Cernere is the source of truth for Synergos project membership and the only issuer of membership tokens. **Projects:** `synergos_projects{org, project_ref, kind: curare_bound\|p2p_only, owner_principal, policy{invite_max_role, require_approval, offline_lease_allowed}, created_at}`; a project is registered by its owner (for a Curare-bound scope, Curare forwards the creating human's request). **Members:** `synergos_memberships{org, project_ref, principal, role: read\|publish\|history, granted_by, granted_at, expires_at?}`; grants of `publish` or `history` and policy changes require SPEC-CR-step-up-assertion; removals take effect at once. **Member requests:** `POST /v1/synergos/projects/{ref}/member-requests {invite, invite_proof, joiner_node_id, joiner_principal}` decides an invite (SPEC-SY-invite-v2) under the project policy, with owner approval when `require_approval`. **Tokens:** `POST /v1/synergos/membership-tokens` takes the node's signed heartbeat (ctx `heartbeat`, relayed verbatim by synergos-control), verifies it against the node's current card, and returns an LCT v1 membership-profile token whose block 0, under ctx `membership`, is `{iss:"cernere", kid, sub_peer_id, node_id, principal, device_id, org, projects[{project_ref, role}], roster_epoch, exp ≤ now+3600, jti}` with `projects` = memberships of `principal` ∩ the card's `projects_scope`; p99 latency < 1 s. **Roster view:** `GET /v1/synergos/projects/{ref}/roster` returns `ProjectRoster{org, project_ref, epoch, members[{principal, role}], nodes[{node_id, peer_id, card_hash}], issued_at, exp ≤ 24 h}` under ctx `project-roster`, readable by members and by synergos-control's service principal. **Offline lease:** on the owner's step-up and only if `offline_lease_allowed`, Cernere issues `OfflineLease{project_ref, site_id, site_cidrs[], peers[{peer_id, role}], issued_at, exp ≤ 72 h, max_stale ≤ 24 h}` under ctx `project-roster`, listing only current members at no more than their role. Synergos components verify these objects and never produce them.

#### 4.34.2 SPEC-CR-node-roster (依頼の規範文。正本: 本書)

> **SPEC-CR-node-roster** (request to Cernere; 正本: Synergos `spec/plan/redesign-security.md` §4.34) — Cernere is the source of truth for which node belongs to which principal and where it is reachable, and the only signer of node cards. **Claim codes:** an org admin, or a principal Cernere policy allows to own nodes, creates after SPEC-CR-step-up-assertion a single-use claim code (128-bit, shown once, stored as SHA-256, 15 min) naming `{owner_principal, roles_allowed, projects_scope, site_id?}`. **Enrollment:** `POST /v1/synergos/enroll` takes the node's enrollment body (SPEC-SY-node-enrollment) relayed verbatim by synergos-control, checks the claim code and the proof under ctx `enroll`, consumes the code, and returns a SPEC-SY-node-card signed under ctx `node-card` (`exp ≤ 24 h`, re-signed before expiry while the node stays enrolled) plus the first membership token (SPEC-CR-membership-issuer). **Changes:** a dual-signed `KeyRotation{old_pub, new_pub, ts}`, a change of declared `endpoints[]`, and (with step-up) a change of roles or project scope each produce a re-issued card and a new roster epoch. **Removal:** removing a node or its owner's org membership revokes the node at once. **Status:** `GET /v1/synergos/orgs/{org}/roster-status` returns `RosterStatus{org, epoch (monotonic), issued_at, exp = issued_at+300, nodes_revoked[], principals_revoked[], memberships_revoked[{principal, project_ref}], sessions_revoked[]}` under ctx `roster-status`, re-issued at least every 60 s and on every change; `GET /v1/synergos/orgs/{org}/roster?since=epoch` returns the changed cards for synergos-control's service principal. A revocation is visible in `RosterStatus` within 10 s and pushed as `node-revoked` / `roster-changed` SSF events, so that, together with the 30 s PEP poll, G1 (p99 ≤60 s) holds.
| SPEC-CR-service-principal | 追加提案 (依頼に未記載、Q7) | `orgs[]` と scope を持つ ≤1 時間のサービス token | Curare → control / custody のサービス資格、失効の入力 API。AGD-AI-12 (Codex クラウドのエージェントが人間級の長命 token を持つ) と AGD-GW-13 (パイプライン・レンダーファームに主体が無い) が根拠 | mTLS のクライアント証明書だけ (SPEC-SY-custody-service-auth) |
| SPEC-CR-guest-identity | 追加提案 (任意) | magic link の受取人の guest subject | magic link 受取人 (Curare 側) | Curare 発行のメール OTP |

### 4.35 新設データ (RULE_DATA_SCHEMA)

RULE_DATA_SCHEMA.md §1 と §4 に従い、種類は master (権威ソースが決まった参照中心のデータ) か user (利用者・ノード・運用者の行動や入力から生じるデータ) のどちらかにする。性質 (秘密鍵、署名文書など) はデータ名の列に書く。保存先が複数にまたがるものは、権威とキャッシュを分けて書く。個人データは Cernere の UUID だけを持ち、メールアドレスを持たない。本表は P0 で `spec/data-schema.md` (新設) へ移す。

| データ名 | 種類 | 権威ソース | 保存先 | 保護要否 | 保護方法 |
|---|---|---|---|---|---|
| 発行者の秘密鍵 (custody delegation 発行・受領証署名・失効バンドルの deny 専用部分の署名。kid 付きの Ed25519。membership と roster の発行鍵は Synergos に無い) | master | 各所有サービス (custody-admin、control-node の deny 専用 kid) | Excubitor Vault Transit (非 export の鍵) | 要 | Vault policy を 1 サービス 1 用途に限る。kid の二重受理でローテーション。平文をディスク・ログに出さない。membership・roster の鍵を Synergos のどの Vault policy にも置かない (オーナー決定 8) |
| Cernere の `membership` / `roster` kid の公開鍵 | master | Cernere (SPEC-CR-capability-signing-keys の JWKS) | PEP と control-node の設定・メモリ (キャッシュ) | 保護不要 (公開値)。完全性は要 | JWKS をローテーション付きで取得し pin。user JWT の鍵と混ぜない |
| 発行者の公開鍵の pin (`SYNERGOS_REVOCATION_PUBKEYS`、`SYNERGOS_RECEIPT_PUBKEYS`、custody delegation の検証鍵) | master | 各所有サービス (Cernere の鍵委任記録で認証) | `GET /v1/custody/keys` と PEP の設定 (Excubitor Vault から注入。キャッシュ) | 保護不要 (公開値)。完全性は要 | SPEC-CR-capability-signing-keys の鍵委任記録で確かめる。提供前は Vault の pin だけを正とする |
| プロジェクト KEK | master | Excubitor Vault Transit (作成・破棄は control-admin の運用操作) | Vault Transit (非 export) | 要 | wrap / unwrap は custody と履歴ノードの Vault policy だけ。unwrap は `operation_id` か `jti` を伴い記録。破棄 = crypto-shred |
| KEK で包んだ DEK (実体ごと) | user (upload に伴って生じる) | custody (admission) / daemon (manifest 項目) | admission 記録・manifest 項目 | 要 | 包んだ形だけを保存し、平文の DEK を書かない |
| プロジェクトの root secret と導出鍵 (`project_key`、`project_dedup_key`、`project_chunk_key`、`project_relay_key`) | master | custody-admin (root secret の作成と回転) | root secret は Vault KV v2。導出鍵はメモリだけ | 要 | KV の読取は custody・chunk-store の policy だけ。`project_relay_key` だけは署名付き heartbeat の応答で、Cernere 署名の membership LCT がその `project_ref` を含む daemon に渡し (会員資格は与えない)、daemon はメモリだけに持つ (漏れても relay が会員の組合せを推測できるだけ) |
| enrollment の claim code | user (org admin の操作) | Cernere (SPEC-CR-node-roster) | Cernere。Synergos は保存しない (control は本文を取り次ぐだけ) | 要 | Cernere が step-up 後に発行。平文は 1 回だけ表示、`SHA-256` で保存、15 分、単回 |
| ノードレジストリ (Cernere 署名の card の写しと liveness: `{node_id, peer_id, card_hash, card, last_counter, last_ts}`) | user (ノードの登録と heartbeat) | 名簿は Cernere (SPEC-CR-node-roster)、liveness だけ control-node | control の保存層 (追記 journal、全 JSON 書換えをしない)。card は Cernere の `roster?since=epoch` のキャッシュ | 要 (完全性) | counter の巻戻しを拒否。card は Cernere の署名のまま保存し、control は足さない・変えない。鍵交換は Cernere での card の再発行 |
| 会員資格レジストリ (Cernere の `synergos_memberships` と ProjectRoster) | master | Cernere (SPEC-CR-membership-issuer)。Curare 紐付けかどうかによらない | Cernere。control は Cernere 署名の ProjectRoster と membership LCT をキャッシュするだけ。CapSet は LCT の写し | 要 | Cernere UUID だけを持つ。Synergos 側に権威の写しを作らない |
| `project_policy` (`invite_max_role`、`require_approval`、`offline_lease_allowed`) | master | Cernere (`synergos_projects.policy`、変更は step-up) | Cernere | 要 (完全性) | Synergos は持たない。招待の判定は Cernere の `member-requests` |
| 失効バンドル (`roster_status` + `local`) | user (SSF 入力・運用者の操作に起因) | `roster_status` は Cernere の RosterStatus (会員・ノードの失効の正本)。`local` (deny だけ) は control-admin (署名は control-node の deny 専用 kid) | control の追記 journal。PEP はメモリに最新 1 件 (キャッシュ、寿命 300 秒) | 要 (完全性) | `roster_status` は ctx `roster-status` の Cernere 署名をそのまま埋め込む。`local` は ctx `revocation` の署名、`seq` 後退の拒否、`exp` = 発行 + 300 秒、許可・復帰の項目を持たない |
| 失効の入力 journal (`revocation_inputs{input_id, source, idempotency_key, jtis[], cap_issued_before, received_at, effective_from_seq}`) | user | control-admin | control の追記 journal | 要 (完全性) | 追記だけ。`idempotency_key` で重複を排除。保持 400 日 |
| オフライン貸与 (`OfflineLease`) | master (プロジェクト所有者の step-up) | Cernere (SPEC-CR-membership-issuer、`offline_lease_allowed` のときだけ) | Cernere が発行。control は配布のキャッシュ。PEP はメモリと `<state_dir>` に最新 1 件 | 要 (完全性) | ctx `project-roster` の Cernere 署名。列挙は現会員をその役割以下でだけ |
| Presence (`{node_id, peer_id, card_hash, candidates[], iat, exp ≤ 10 min}`) | user (ノードの実行時の観測) | 各ノード (card に書かれたノード鍵で署名) | control-node の directory のキャッシュと peer のメモリ | 要 (完全性) | ctx `presence` の署名。鍵か `card_hash` が有効な card と合わなければ捨てる。control も peer も署名・変更しない |
| 招待台帳 (`{nonce16, project_ref, role, invitee?, uses_left, exp, revoked}`) | user | 発行ホストの daemon | `<state_dir>` (OS のサービス / ユーザー領域) | 要 | 0600 / 所有者 SID の DACL、耐久書込 |
| upload の commit 済み集合 (`{upload_id, committed_at, exp}`) | user | custody-transfer | custody の保管層 (期限で掃除) | 要 (完全性) | `upload_id` ごとの commit 1 回を強制。期限超過分は削除 |
| 置換旧バイト保管 (`<state_dir>/displaced/<blake3>` と journal) | user | daemon | 作業ツリーと同一ボリュームの `<state_dir>` | 要 | 14 日以上の TTL、`undo-receive` でだけ戻す。暗号化は M27 の範囲に含める |
| 保持メタデータ (版ごとの sidecar `{retain_until, mode, legal_hold, sha256, blake3, last_scrubbed_at, scrub_status}`) | master | custody の値は Curare (D-25)、履歴ノードはプロジェクト方針 | 保管層の sidecar (GC の正本) | 要 (完全性) | 別 ID の GC だけが削除。`index.json` を正本にしない |
| RetentionAck 記録 | user (保持ノードの応答) | 保持可能ノード | 受信ノードの `<state_dir>` と control の journal | 要 (完全性) | ctx の署名付き。期限切れは無効 |
| custody ジャーナル | user | custody | custody の保管層 (権威)、Vestigium (写し) | 要 (完全性) | `{seq, prev_hash, row_hash}`、ctx `journal-checkpoint`、保持 400 日 |
| daemon のセキュリティイベント (`<state_dir>/security/*.jsonl`。peer_id、`src_ip_prefix`、project_ref を含む行動記録) | user | daemon | `<state_dir>` (権威、30 日)、Vestigium (400 日) | 要 | IP は /24 (IPv4)・/48 (IPv6) の prefix だけ。メールアドレスを持たない。秘密を書かない。0600 |
| `download_ledger` (主体ごとの償還・送信の集計) | user | 検知ジョブ (M26) | Vestigium | 要 | 追記だけ。保持 400 日 |
| canary 登録 | master | Curare 紐付けは Curare の canary space、P2P だけのプロジェクトは control-admin | Vault KV (control-admin の policy だけ) | 要 (秘匿) | custody と daemon には渡さない。照合は検知ジョブだけ |
| 復旧訓練の成果物 (`{drill_id, started_at, rto_s, rpo_s, objects_checked, mismatches, signer_kid, sig}`) | user (運用ジョブ) | 復旧訓練ジョブ (M28) | Excubitor のジョブ成果物 | 要 (完全性) | 署名付き。秘密を含めない |
| hook 承認 (`{project_ref, hooks_toml_blake3, approved_by, approved_at}`) | user | daemon (承認者は step-up 済み) | `<state_dir>` | 要 | ハッシュ不一致で実行しない |
| IPC token (`<runtime_dir>/synergos/ipc.token`) | user (daemon の起動ごと) | daemon | OS のランタイムディレクトリ | 要 | 0600 / 0700、Windows は所有者 SID の DACL、`/tmp` に置かない |
| 接続ごとの CapSet (`{peer_id, session_id, projects, exp, jti}`) | user (一時) | daemon / relay / custody (membership LCT から作る写し) | メモリだけ | 要 (完全性) | 接続の終了か `exp` で捨てる。永続化しない |
| `spec/vectors/ctx-registry.json` (ctx と `signers[]`) | master | Synergos (Revisor 変更だけで更新) | repo (commit) | 保護不要 (公開設定) | 変更は golden vector と同じ変更で行う |
| `spec/contracts/registry.json` (契約 ID と規範文のハッシュ) | master | Synergos | repo (commit) | 保護不要 (公開設定) | 契約登録ゲートが検査 |
| `abuse-policy.yaml`、`spec/data/destructive-ops.json`、`spec/data/event-coverage.json`、`fuzz/registry.json`、`deps-approvals.json` | master | Synergos (Revisor 変更だけで更新) | repo (commit) | 保護不要 (公開設定) | 各ゲートが網羅を検査 |
| `excubitor.secrets.json` (鍵名・手順・所有者・consumers。値は持たない) | master | Synergos | repo (commit) | 保護不要 (公開設定。値を含まない) | 値を書けば secret scan (M29) で赤 |

### 4.36 ドメイン帰属 (`spec/domains/*.domain.json`)

帰属は各ドメイン JSON の `membership` (パスのパターン) に従う。`synergos-ipc` は network-transport、`scripts/site/*.mjs` と `excubitor.catalog.yaml` は control-plane に属する。

| ドメイン | 帰属する機構 | 備考 |
|---|---|---|
| network-transport (`synergos-net`、`synergos-relay`、`synergos-ipc`) | M01 (`synergos-net/native/trust/`、D-24)、M02、M03 (gossip と stream の wire 側)、M05 (bootstrap の取得)、M06、M11 (`synergos-ipc` の `IHELLO` と frame)、M13 (TXF2 の受信側)、M23 | `spec/vectors/` も M01 と同じ変更で membership に足す |
| mesh-core (`synergos-core`) | M03 (stream 振分け)、M05 (招待台帳)、M11 (`ipc_server` の scope 判定と破壊的コマンド)、M12、M13 (パス方針と予約)、M14、M15 (履歴)、M22、M24、M25 (daemon イベント)、M27 (履歴と冷蔵層の暗号化) | |
| opaque-custody (`synergos-custody`) | M08 (custody 側)、M09、M15 (custody の保管層)、M16 (lifecycle)、M25 (ジャーナル)、M27 | 説明文の「受領証」は v2 (Ed25519) を指すよう更新 |
| control-plane (`synergos-control`、`scripts/site/*.mjs`、`excubitor.catalog.yaml`) | M04、M07、M08 (control 側と配備定義)、M10、M16 (`revoke_dark`)、M20 (サイトスクリプト)、M26、M28 (bundle と訓練)、M32、M33、M34 | `excubitor.secrets.json` と `spec/operations.md` (specRefs) を membership に足す。現行の pattern は `excubitor\.(catalog\.yaml\|bootstrap\.json)` だけ |
| desktop-gui (`synergos-gui`、`synergos-tauri`) | M30、M31 (Tauri の CSP・updater) | `desktop/Cargo.toml` を membership に足す |
| admin-ui (`synergos-admin-ui`) | M10 (cookie 化)、M31 (署名 manifest)、M32 | |
| ars-plugin (`ars-plugin-synergos`) | M11 (IPC クライアント側の `client_kind: plugin`) | |
| **chunk-store (新設、転送効率文書の提案)** | M27 (SPEC-SY-chunk-access の PEP) | `synergos-chunkstore/` を membership にする。API の正本は転送効率文書。D-T03 の比較で独立モジュールが採られ、`cpp-migration.md:79-80` の「cross-domain directory」ではなく 1 ドメイン 1 モジュールとして足す |
| **quality-gates (新設提案)** | M21 (`abuse-policy.yaml`)、M23 (`fuzz/`)、M25 (`spec/data/event-coverage.json`)、M16 (`spec/data/destructive-ops.json`)、M29 (`scripts/gate/*`、`deny.toml`、`deps-approvals.json`、`rust-toolchain.toml`)、M33 (`scripts/gate/playbook-coverage.mjs`) | 既存のどのドメインも repo 全体の検査を受け持たない (REVIEW_DESIGN §0 の「既存ドメインの守備範囲に収まるものを不必要に切っていないか」を確かめた結果)。採否は Q-SY-4 で D-24 と一緒に判断する。`.github/workflows/*` は削除するのでドメインを割り当てない |

署名封筒・LCT codec・golden vector (M01) は、D-24 の採用案 B により新ドメインを作らず network-transport に置く。セキュリティイベントの新ドメイン `security-events` は Curare 側の提案で、Synergos はイベントの出力側としてだけ関わる。

### 4.37 `cpp-migration.md` "Compatibility contracts" 改訂案

オーナー決定 6 により、HLO2・custody 強化・認証強化は次の改訂で C++ 移行の互換契約として扱う。改訂は本書とは別の Revisor 変更として、Q1 の回答後に出す。転送効率文書の改訂 (同じ節の `:39-47` を触る) と 1 回の変更にまとめる。左列は現行の文言 (`spec/plan/cpp-migration.md:33-48` と `:79-80`)。区分は「置換」「拡張」「追加」「誤記訂正」のどれかと、[変更提案] の要否。

| 現行の行 | 改訂案 | 区分 |
|---|---|---|
| `:35-37` "IPC: … MessagePack payload, maximum 1 MiB, incremental allocation. Preserve command, response and event variants and compact serde struct field ordering." | 現行文を残し、追加: "Existing variants are preserved; SPEC-SY-ipc-capability adds the `IHELLO` first frame, scopes, and the two-step confirmation (`ConfirmRequired` / `Confirm`) for destructive commands. Result paging and jobs follow the transfer doc's SPEC-SY-ipc-v2." | C (拡張) [変更提案、Q1] |
| `:38` "Local IPC authorization: same-user checks, no network-exposed pipe/socket." | "Local IPC authorization: SPEC-SY-ipc-capability (IHELLO token with scopes, fail-closed credential lookup, confirmation or step-up for destructive commands), no network-exposed pipe/socket." | C (置換) [変更提案、Q1] |
| `:39-40` "Peer protocol: preserve protobuf schemas, QUIC ALPN, peer authentication, CRC/hash validation, transfer cancellation, and catalog exchange." | "Peer protocol: replace HLO1 with SPEC-SY-hlo2-session-binding (mTLS, ALPN `synergos/2`, SESS) and enforce SPEC-SY-membership-token per stream; all signatures follow SPEC-SY-signed-envelope; decoders follow SPEC-SY-decode-bounds. The wire is MessagePack per the transfer doc's SPEC-SY-peer-wire (the current wire has no protobuf schemas); preserve transfer cancellation and catalog exchange semantics." | C (置換) [変更提案、Q1]。「protobuf」は誤記訂正 (現行の wire は手組みの MessagePack / JSON、SEC-DA-19)。CRC と版なし TXFR の扱いは転送効率文書の SPEC-SY-content-identity と合わせる (TRN-WP-21, TRN-DD-22) |
| `:41-43` "Versioning: preserve `.synergos/manifest.json`, local high-water marks, restore/checkout semantics, tags and protected history. Reject path escapes, symlinks/junctions and version overflow before modifying data." | "Versioning: preserve `.synergos/manifest.json` readers (format 1 and the transfer doc's format 2) and local high-water marks. Checkout and restore become resumable verified jobs (SPEC-SY-ipc-v2); tag deletion and history purge follow SPEC-CU-destructive-op tombstones and SPEC-SY-retention-lock; history GC never trusts `index.json` alone. Apply SPEC-SY-path-policy (one normalizer, case-fold, deny list at any depth, handle-relative operations) and SPEC-SY-pull-only-receive; every received replace follows SPEC-SY-displaced-preimage." | C (置換。tag と保護履歴の意味が変わる) [変更提案、Q1] |
| `:44-47` "Custody: SPEC-SY-opaque-custody, SPEC-SY-custody-durability, SPEC-SY-custody-acquisition remain normative. Preserve canonical receipt bytes, HMAC, capability method/object/expiry binding, exclusive publication, file/directory durability and tombstones. Cu remains the authorization owner." | "Custody: SPEC-SY-custody-durability remains normative. SPEC-SY-opaque-custody and SPEC-SY-custody-acquisition are amended by SPEC-SY-custody-receipt-v2 (Ed25519, kid), SPEC-SY-custody-capability-v2 (LCT; method, object, expiry and download-name binding as caveats), SPEC-SY-custody-service-auth, SPEC-SY-custody-lifecycle, SPEC-SY-custody-topology and the transfer doc's SPEC-SY-custody-transfer-v2. HMAC receipts are verify-only for stored rows during the grace window. Preserve exclusive publication, file/directory durability and tombstones. Cu remains the authorization and retention owner." | C (置換) [変更提案、Q1] |
| `:48` "Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces." | "Existing CLI, admin UI and Ars adapter remain acceptance surfaces. The native GUI surface is the C++ desktop-gui; until it passes its acceptance gate, the egui `synergos-gui` (built from the `desktop/` workspace without the accesskit feature) and `synergos-tauri` are the interim surfaces." | C (置換) [変更提案、Q1、Q3] |
| (新規行) | "Control: SPEC-SY-node-enrollment, SPEC-SY-control-admin-auth and SPEC-SY-revocation-bundle replace the static admin bearer and node key. Project membership and the node roster are owned by Cernere (SPEC-CR-membership-issuer, SPEC-CR-node-roster); control only relays, caches and verifies Cernere-signed membership LCTs, node cards, ProjectRoster, RosterStatus and OfflineLease, holds no membership or roster issuer key, and has no route that changes membership, enrollment, ownership or reachability." | C (追加、X) |
| (新規行) | "Relay: SPEC-SY-relay-auth and the transfer doc's SPEC-SY-relay-v2 are implemented only in C++ (`synergos-relay/native/`); the Rust relay is neither changed nor deployed. Until the C++ relay passes its acceptance gate, sites and external environments connect over Tailscale and `synergos-node` binds only to the tailnet IP." | C (追加、オーナー決定 7) |
| (新規行) | "Retention: SPEC-SY-retention-lock, SPEC-SY-retention-ack and SPEC-SY-immutable-replica; history GC never trusts `index.json` alone." | C (追加) [変更提案、Q1] |
| (新規行) | "Execution: hooks and parsers run only under SPEC-SY-sandbox-profile; child processes never inherit the daemon environment." | C (追加) |
| (新規行) | "Storage protection: SPEC-SY-envelope-encryption for custody, history and the chunk store; SPEC-SY-chunk-access at the chunk-store boundary." | C (追加) |
| `:79-80` "C++ source is placed in existing module membership rather than introducing a cross-domain directory." | 変えない。M01 は D-24 の B で `synergos-net/native/trust/` に置く。新ドメインは chunk-store (転送効率文書) と quality-gates (Q-SY-4) で、どちらも cross-domain directory ではなく 1 ドメインの新モジュール | 変更なし (D-24) |

「Implementation and verification order」(`cpp-migration.md:50-62`) には、手順 2 の後に「署名封筒・LCT codec・golden vector (M01) と decoder 境界 (M23)」を、手順 4 の前提に「受付パイプライン (NR-1) と会員資格ゲート (M03)」を加える。Completion gate (`:64-71`) の「parity evidence」には、§6 の C++ に属する攻撃試験 AT-SY-SEC-01〜20、22〜32、34、35、37〜41 の合格を加える (AT-SY-SEC-21・33・36 はゲート・workspace・運用の試験で C++ の受入には含めない)。旧挙動との parity ではなく、新契約への適合を合格条件とする項目があることを明記する。

### 4.38 `native-contracts.md` へ追加する `[id:]` 節

`spec/feature/native-contracts.md` (現行は SPEC-SY-native-framing と SPEC-SY-native-metadata の 2 節) に、次の節を追加する。各節の本文は本書 §4 の英語の規範文をそのまま移し、移した後は本書の該当箇所を ID と節番号への参照に置き換える (規範文を 2 か所に置かない)。

| 追加する `[id:]` | 本書の節 | 実装の置き場所 (既存の `native/` に従う) |
|---|---|---|
| SPEC-SY-signed-envelope | §4.1 | `synergos-net/native/trust/` (D-24) |
| SPEC-SY-lct-v1 | §4.1 | 同上 |
| SPEC-SY-decode-bounds | §4.20 | 各 decoder の `native/` |
| SPEC-SY-hlo2-session-binding | §4.2 | `synergos-net/native/` |
| SPEC-SY-membership-token | §4.3 | `synergos-net/native/`、`synergos-core/native/` |
| SPEC-SY-node-card / SPEC-SY-invite-v2 | §4.5 | `synergos-core/native/` |
| SPEC-SY-relay-auth | §4.6 | `synergos-relay/native/` (新設) |
| SPEC-SY-ipc-capability | §4.11 | `synergos-ipc/native/`、`synergos-core/native/` |
| SPEC-SY-sandbox-profile | §4.12 | `synergos-core/native/` |
| SPEC-SY-path-policy / SPEC-SY-pull-only-receive | §4.13 | `synergos-core/native/` (既存の `relative_path.cpp` を拡張) |
| SPEC-SY-displaced-preimage / SPEC-SY-retention-ack | §4.14 | `synergos-core/native/` |
| SPEC-SY-retention-lock / SPEC-SY-immutable-replica | §4.15 | `synergos-core/native/`、`synergos-custody/native/` |
| SPEC-SY-node-enrollment / SPEC-SY-revocation-bundle / SPEC-SY-control-admin-auth | §4.4, §4.7, §4.10 | `synergos-control/native/` (新設) |
| SPEC-SY-custody-service-auth / -capability-v2 / -receipt-v2 / -lifecycle / -journal / -topology | §4.8, §4.9, §4.16, §4.22 | `synergos-custody/native/` |
| SPEC-SY-envelope-encryption | §4.24 | `synergos-custody/native/`、`synergos-core/native/`、`synergos-chunkstore/native/` |
| SPEC-SY-chunk-access | §4.24 | `synergos-chunkstore/native/` (新設、ドメイン chunk-store。転送効率文書の SPEC-SY-chunk-store と同じモジュール) |

`spec/feature/opaque-custody.md` は、SPEC-SY-opaque-custody と SPEC-SY-custody-acquisition の本文に v2 への移行注記を足し (`:12`, `:17`, `:21`, `:37`, `:39`, `:40`, `:41-42`, `:53` の各行。§3.22)、`:58` の Infisical の記述を「Excubitor Vault の注入を使用する」に訂正する。

### 4.39 HMAC `signature.cpp` の扱い

`cpp-migration.md:82-83` は最初のネイティブ増分に「receipt/capability signing」を含め、`synergos-custody/native/signature.cpp:23-26` は HMAC-SHA256 の `sign` / `verify` を実装している。再設計では、この移植は**旧受領証の検証専用経路**にする。新たな受領証や capability の発行には使わない。猶予期間 (≤30 日) の後、保存済み v1 行の検証が不要になった時点で削除する。新しい署名はすべて SPEC-SY-signed-envelope の Ed25519 を通す。

### 4.40 crate 別の変更マップ

「現行 Rust」の列は、オーナー決定 6 に従い実行時の変更をしないことを示す (M30 と M34 を除く)。実装はすべて C++ 側。ドメインは §4.36 の帰属に従う。

| crate / 面 | ドメイン | C++ 側で入る機構 | 現行 Rust への変更 |
|---|---|---|---|
| synergos-net (QUIC、gossip、DHT、bitswap、transfer、identity、tunnel) | network-transport | M01 (`native/trust/`)、M02、M03 (gossip と stream の wire 側)、M05 (bootstrap の取得)、M13 (TXF2 の受信側)、M23、NR-1/2/3/5/7。DHT と bitswap は移さない (D-23、X4) | なし (`server/` workspace へ移る、M30) |
| synergos-core (daemon、exchange、manifest、history、hooks、checkout、ipc_server) | mesh-core | M03 (stream 振分け)、M05 (招待)、M11 (サーバ側)、M12、M13、M14、M15、M22、M24、M25 (daemon イベント)、M27 (履歴と冷蔵層)、NR-4/6 | なし。設定の推奨値だけ (M34) |
| synergos-control | control-plane | M04 (Cernere への enrollment・membership 更新の取り次ぎ、card のキャッシュ)、M07 (入力 API を含む。`local` は deny だけ、`roster_status` は Cernere 署名の埋め込み)、M08 (control-admin / control-node 分離)、M10 (会員・名簿のルートは `409 authority_is_cernere`)、M16 (`revoke_dark`)、M26 (canary 登録)、M32 | なし。`bind_addr` は loopback のまま (M34) |
| synergos-custody | opaque-custody | M08 (custody-transfer / custody-admin 分離)、M09、M15 (保管層)、M16 (lifecycle)、M25 (ジャーナル)、M27、M32 | なし |
| synergos-chunkstore (新設、C++ だけ) | chunk-store (新設) | M27 (SPEC-SY-chunk-access)。API は転送効率文書の SPEC-SY-chunk-store | (Rust には無い) |
| synergos-relay | network-transport | M06、M21 (relay 行)。C++ だけ (オーナー決定 7) | なし。Rust の relay は改修も配備もしない。C++ relay が受入ゲートを通るまで到達は Tailscale (M34) |
| synergos-ipc | network-transport | M11 (`IHELLO`、確認 frame)、M21 (IPC 行) | なし |
| synergos-gui (egui) | desktop-gui | なし (C++ desktop-gui が受入ゲートを通ったら撤去) | P0 で `desktop/` workspace へ移し、eframe の `accesskit` を外す (M30) |
| synergos-tauri | desktop-gui | M31 (updater 署名、CSP)、M11 (`client_kind: tauri`) | `desktop/` workspace へ移す (M30) |
| synergos-admin-ui | admin-ui | M10 (cookie セッション)、M31 (署名 manifest)、M32 | なし |
| ars-plugin-synergos | ars-plugin | M11 (`client_kind: plugin`、最小 scope) | なし (`server/` workspace へ移る) |
| `scripts/site/*`、`excubitor.catalog.yaml`、`excubitor.secrets.json` | control-plane | M08 (配備単位ごとの定義)、M20、M28 (bundle v2 と訓練)、M31 (検証のみのサイト) | P0 で `SYNERGOS_SITE_DATA_DIR` の必須化、秘密の手順別注入 (M20)、文書修正 (M34) |
| `scripts/gate/*`、`fuzz/`、`abuse-policy.yaml`、`spec/data/*.json`、`deny.toml`、`deps-approvals.json`、`rust-toolchain.toml` | quality-gates (新設提案、Q-SY-4) | M16、M21、M23、M25、M29、M33 のゲート | P0 でゲートを新設 |
| `.github/workflows/*` | (削除するので割り当てない) | M29 | 置き換えの Revisor ケースが main で 1 回通った後に削除 |

## 5. 実装ステップ・影響範囲・ロールバック方針

### 5.1 フェーズ表

| フェーズ | 内容 | 対象 crate / module | 依存 | 検証方法 (テスト種別) | ロールバック |
|---|---|---|---|---|---|
| P0 (今すぐ。文書・ゲート・設定だけ) | 本書を含む 6 文書と検証済み発見 3 本 (`spec/plan/redesign-analysis/findings/*-verified.json`) を Revisor で取り込む。`cpp-migration.md` / `native-contracts.md` / `opaque-custody.md` の改訂案は Q1 の回答後に別の Revisor 変更で出す。M29 (ゲート新設)、M30 (workspace 分割、eframe の feature、`README.md`・`docs/getting-started.md`・`docs/setup-macos.md`・`docs/platforms.md` の更新)、M33 (`spec/operations.md`)、M34 (設定の暫定補償、`docs/mesh-operations.md:126-130` と `docs/mesh-two-node-checklist.md:48-54` の `0.0.0.0` / `http://` の指示の修正、Codex 経路の serve と ACL の監視ジョブ)、M21 の登録ゲートを報告モードで、M01 の契約登録ゲート | workspace、`scripts/gate/`、`docs/`、`spec/`、Excubitor のサイト上書き設定と監視ジョブ | なし | ゲートの自己試験 AT-SY-SEC-21 (違反を仕込むと赤になる)、AT-SY-SEC-33 | ゲート登録を戻す。設定の上書きを戻す。workspace 分割は 1 commit で戻せる。`.github/workflows/*` の削除は、置き換えの Revisor ケース (Rust の compile と実行、clippy、CMake、native CTest、supply-chain) が main で 1 回以上通った記録を確かめた後の別の変更で行う (`cpp-migration.md:97-98`「Registration alone does not prove they passed」)。それまでは workflow を残す |
| P1 (codec) | M01 (C++ の envelope / LCT / JCS と golden vector)、M23 (境界と fuzz 登録)、M09 の受領証 v2 と LCT の codec。**配線しない** | `synergos-net/native/trust/` (D-24)、`spec/vectors/`、`spec/contracts/registry.json`、`fuzz/registry.json` | P0 | golden vector の同値性 (Synergos と Curare の両 Revisor ケース)、fuzz の短時間実行 (AT-SY-SEC-22) | 配線していないので戻す対象が無い |
| P2 (Curare 実行時) | Curare 文書が持つ。Synergos 側は作業なし。Curare の SSF 受信器 (M07 の入力) と capability 減衰 (M09 の利用側) がここで入る | (Curare) | P1、SPEC-CR-* | Curare 文書の検証 (AT-CU-SEC-*) | Curare 文書のロールバック |
| P3 (C++ custody と control) | M08 (まず 1 プロセスで listener 分離、次に配備分離)、M09 (サービス認証・delegation・capability v2・受領証 v2)、M16 (custody lifecycle)、M04 (enrollment と membership 更新の取り次ぎ)、M07 (control のバンドル・入力 API・事件 API・Cernere 署名の RosterStatus と OfflineLease の配布)、M10、M25 (custody ジャーナル)、M20 / M21 / M32 の control・custody 部分、control-node の多重化 (G11) | `synergos-custody/native/`、`synergos-control/native/` (新設)、`excubitor.catalog.yaml` | P1。**会員資格と enrollment の部分 (M04、M07 の `roster_status`、M10 の会員・名簿の扱い) は Cernere の SPEC-CR-membership-issuer と SPEC-CR-node-roster を待つ** (Actio `actio:2a7af0fa`・`actio:fd0e4de8`)。Synergos 側の暫定は無く、control に会員・名簿の鍵を持たせない。custody 側 (M08、M09、M16、M25) は待たない。Curare の P2 のうち **M09 の利用側 (capability の絞り込みと SPEC-CU-capability-ledger) だけ**。M17 (SSF 受信) は待たない: 無い間は M07 の暫定 (Curare が 60 秒ごとに所属を照会し、入力 API へ送る) で動かす。SPEC-CR-service-principal か mTLS の暫定。SPEC-CR-step-up-assertion か 2 名承認の暫定 | AT-SY-SEC-06・07・08・10・14・18・25・31・32・39、失効の端から端まで ≤60 秒 | 鍵は kid の二重受理。**巻き戻し点は組織ごとの最初の v2 admission の前**: それまで C++ custody は旧 flat 配置を読取専用で配信するだけで、Rust custody に戻しても失うものがない (設定 `custody.v2_admission_enabled = false` が既定)。**最初の v2 admission の後**は完全には戻さない: 新規 admission を Rust custody (v1、flat 配置) へ戻すことはできる (Curare 側の `CURARE_CUSTODY_CAP_MODE=v1`、≤30 日) が、C++ custody は `org/<org>/<shard>/<key>` の実体と v2 受領証の配信のために読取専用で動かし続ける (Rust custody は新配置を読めない)。Curare は v2 受領証を公開鍵で検証するので、受領証の検証は custody の巻き戻しに影響されない。C++ の受入ゲートを通るまで Rust の custody を残す (`cpp-migration.md:71`) |
| P4 (C++ peer データプレーン) | M02、M03、M05、M06、M11、M12 (hook)、M13、M14、M22 (Synergos 側)、NR-1〜8。転送効率文書の TXF2 と同じ flag day | `synergos-net/native/`、`synergos-core/native/`、`synergos-ipc/native/`、`synergos-relay/native/` | P3 (control が Cernere 署名の membership LCT と card を取り次げること)、SPEC-CR-membership-issuer と SPEC-CR-node-roster の提供 (Synergos 側の暫定なし)、C++ relay だけ (オーナー決定 7)、転送効率文書の SPEC-SY-peer-wire | 複数ノードの攻撃試験 (AT-SY-SEC-01〜05、09、12、13、15、16、23、34)、ランサム演習 | ALPN 交渉で `synergos/1` を明示拒否する。**組織ごとの flag day** (D-22) として `spec/operations.md` で告知する。戻す場合はその組織の全ノードを M34 の設定付きで Rust 実行時に戻す (混在させない)。P4 が新しく書くデータ (`displaced/`、招待台帳、`security/*.jsonl`) を Rust は読まないだけで壊さない。manifest format 2 を Rust が読めるかは転送効率文書の TRANSFER-M03 で未確認 (Rust の reader が未知フィールドを拒むか未確認のため) で、確認までは format 2 の書込を flag day の後に限る |
| P5 (データ保護) | M15、M27 (チャンク保管の境界を含む)、M28、M24 (展開ゲート)、M12 (Curare の検査ワーカー側は Curare 文書) | `synergos-core/native/` (history)、`synergos-custody/native/`、`synergos-chunkstore/native/`、`scripts/site/` | P3 と P4、Vault Transit、別アカウントの保管先 (Q5) | 月次の復旧訓練の成果物 (AT-SY-SEC-36)、crypto-shred 試験 (AT-SY-SEC-20)、AT-SY-SEC-30・35・38、AT-SY-EFF-03・06・07 | 組織ごとの有効化から **30 日を巻き戻し窓**にする。窓の間は (1) KEK を破棄しない (破棄は離脱だけ)、(2) `custody export --decrypt --org <org>` (step-up、`operation_id` 付き、別 ID) が暗号化済み実体を平文の blake3 名の flat 配置へ書き出し、P5 前の部品が読めるようにする。窓を過ぎたら、その組織の P5 は一方向として neco が承認し記録する (Q-SY-7)。不変複製は追記専用。KEK のローテーションは包み直しだけ。検査は報告モードから始める |
| P6 (検知と配布完全性) | M26、M31、机上演習 | Excubitor、Concordia、配布物 | P2〜P4 のイベント | canary 発火試験 (AT-SY-SEC-19)、流出予算試験、サイトでの署名検証 (AT-SY-SEC-37) | 閾値は設定。規則を止めても記録は続ける |

### 5.2 互換性と移行

- peer プロトコルの flag day は 1 回だけ (HLO1 → `synergos/2`、D-22)。組織ごとに日程を組み、`spec/operations.md` で告知する。同じ組織の中で Rust ノードと C++ ノードを混在させない (HLO1 と HLO2 は相互接続しない)。期限付きの二重スタックを採らない理由と残存リスクは D-22。
- 鍵のローテーションは常に kid を 2 つ受理する。flag day を作らない。
- 受領証 v1 (HMAC) は保存済みの行の検証専用として猶予期間 (≤30 日) 残し、新規発行は v2 だけにする。Curare の保存済み行は監査のため検証可能なまま残す (Curare 文書)。
- custody の配置変更 (`org/<org>/<shard>/<key>`) は、admission に `{org_id, project_ref}` が付いた新規分から適用する。既存の平坦な配置の実体は C++ custody が読み取り専用で引き継ぎ、組織の付与は Curare の台帳と照合して移す (照合不一致は移さず報告)。巻き戻し点は §5.1 P3。
- 現行の `control-store.json` (`scripts/site/run-control.mjs:19`) とレジストリのノードは、Cernere のノード名簿へ移す (SPEC-CR-node-roster。各ノードは所有者が Cernere で発行した claim code で enrollment し直し、Cernere 署名の card を得る)。C++ の control は Cernere の card のキャッシュと liveness の追記 journal だけを持つ。変換前の JSON は削除せず、読取専用で保持する。
- 既存のプロジェクトは、P2P だけのものも Curare 紐付けのものも、会員が生まれる前に所有者 (Curare 紐付けの scope は Curare が作成者本人として取り次ぐ) が Cernere に `project_ref` を登録する。招待の方針 (`project_policy`) は Cernere の `synergos_projects.policy` に置き、Synergos 側には作らない。
- データ移行は追加だけ。削除 (DROP 相当) はしない。Synergos 自体は DB を持たないが、保存形式の変更も「新形式を足し、旧形式は読取専用で残す」に揃える。
- 秘密の取得元は Excubitor Vault だけ。Infisical は使わない。

## 6. 検証計画

### 6.1 攻撃シナリオの再現試験 (Revisor / Augur のケース)

試験の ID は両文書で共通の体系にそろえる: 本書は AT-SY-SEC-nn、Curare 文書は AT-CU-SEC-nn。本書の旧版の番号 n は AT-SY-SEC-0n / AT-SY-SEC-n にそのまま対応する (旧 1〜8 → 01〜08、旧 10〜12 → 10〜12、旧 17・19・20・21 → 17・19・20・21)。旧版が「Curare が正本」とした試験は次の AT-CU-SEC に対応する: 旧 9 → AT-CU-SEC-05・06・07、旧 13 → AT-CU-SEC-13・14、旧 14 → AT-CU-SEC-17、旧 15 → AT-CU-SEC-22、旧 16 → AT-CU-SEC-16、旧 18 → AT-CU-SEC-23。内容が重なる組は、AT-SY-SEC-17 と AT-CU-SEC-29 (網羅。各 repo が自分のハンドラを検査する)、AT-SY-SEC-21 と AT-CU-SEC-21 (ゲートの自己試験。各 repo のゲートを検査する)、AT-SY-SEC-10 と AT-CU-SEC-10 (失効。P3 で端から端の 1 本にまとめる)。

どれも「修正前の Rust 実行時で攻撃が成立すること」を記録したうえで (成立しない試験は「新機構の適合試験」と明記する)、C++ 実装で失敗することを受入条件にする。資格・鍵は試験の中で毎回生成し、固定値を repo に置かない。

| ID | シナリオ | 試験内容 | 合格条件 | 発見 / 目標 |
|---|---|---|---|---|
| AT-SY-SEC-01 | (a) Hello / SESS の再生 | 別接続で捕獲した HLO1 Hello、または SESS を別の TLS セッションで再送 | 接続が `0x5301 auth_mismatch` で閉じ、被害者の接続は残る | SEC-CR-01, SEC-AZ-02 |
| AT-SY-SEC-02 | (a) 非会員 peer | プロジェクト P の membership LCT を持たない peer が TXF2・SYNC・PLAN・RLY・GSP1 を開き、履歴ノードへ TXF2 の WANT を送る。TXFR v1・BSW1・DHT・FileWant の magic を送る。SESS に LCT の無い session から JOIN1 以外を開く | 本体の 1 バイト目より前に `RESET 0x5303 forbidden` と `peer.denied`。除去した magic は拒否。LCT の無い session は JOIN1 だけ開ける | SEC-AZ-01, SEC-AZ-04, SEC-AZ-06, AGD-AI-16, G4 |
| AT-SY-SEC-03 | (a) 予約パスの書込 | `.GIT/hooks/x`、`.SYNERGOS/manifest.json`、`.vscode/tasks.json` への送信を Windows / macOS / Linux で | すべて拒否、作業ツリーに変化なし | SEC-DA-05, SEC-IN-02 |
| AT-SY-SEC-04 | (a) ランサム型の大量置換 | 60 秒以内に 1,000 ファイルを高エントロピーの内容で置換する送信。対照として 60 秒に 400 ファイル・合計 512 MiB の通常の置換 | 前者は暫定閾値 (60 秒で 500 ファイル以上か 1 GiB 以上、§4.14) でブレーカーが作動し、プロジェクトが隔離され、`undo-receive` で全ファイルがバイト一致で戻る。後者は作動しない。閾値は Q-SY-2 で見直す | SEC-IN-01 |
| AT-SY-SEC-05 | (a-2) 局所 IPC | token 無しのクライアント、plugin token での HistoryPurge | 前者は何も得られない。後者は拒否され `ipc.denied` | SEC-AZ-09, SEC-BR-02, G5 |
| AT-SY-SEC-06 | (b) 鍵差替えの heartbeat | 現在の Cernere 署名 card と違う peer_id・`card_hash` の heartbeat (Cernere での card の再発行を経ない鍵の差替え)、counter を巻き戻した heartbeat、120 秒超のずれ。control に直接出した KeyRotation | heartbeat はすべて 401 と `node.heartbeat_rejected`。heartbeat で名簿は変わらない。KeyRotation は Cernere が card を再発行するまで効かない | SEC-AZ-12, SEC-CR-11 |
| AT-SY-SEC-07 | (c) 静的 admin token | 切替後に旧 admin bearer で操作、step-up と 2 人目の承認無しの `revoke_dark` | どちらも拒否 (403) | SEC-AZ-11, SEC-BR-04 |
| AT-SY-SEC-08 | (d) custody の資格 | 受領証の公開鍵だけで受領証を作る、組織 A の delegation で組織 B の実体を償還、同じ `upload_id` の 2 回目の commit、TTL 上限超の capability、`download_name` を書き換えた capability | 受領証は検証失敗、償還は 403、2 回目の commit は 409、TTL 超は 403、改ざんは署名検証で 403 | SEC-CR-04, SEC-BR-05, SEC-CR-05, G4 |
| AT-SY-SEC-09 | relay の詐称 | membership LCT の無い join、別鍵の署名での join、frame の `from_peer_id` の詐称、`ws://` での接続。QUIC (UDP 443) と WSS (TCP 443) の両方で | すべて拒否 (`relay.join_denied`)。詐称した `from_peer_id` は無視され、受信側には join で確定した値が届く。中継中に relay が保持したバイトに、送った平文の目印が 1 件も現れない | SEC-AZ-08, SEC-CR-14, SEC-DA-09 |
| AT-SY-SEC-10 | (f) 失効の伝播 | Cernere の `session-revoked` を発火 (提供前は Curare の照会暫定で代替し、その場合は G1 未達を記録)、control の事件 API で freeze | custody の 403 と QUIC の close が ≤60 秒。300 秒より古いバンドルの PEP は新規セッションを拒否 | SEC-AZ-17, SEC-CR-09, SEC-BR-12, G1 |
| AT-SY-SEC-11 | (g) 履歴の破壊 | 保持期限内の history purge、履歴ノードの資格で不変複製を削除 | どちらも失敗 | SEC-IN-05, SEC-IN-06 |
| AT-SY-SEC-12 | (i) hook の脱出 | hook からネットワーク接続、出力ディレクトリ外への書込、`SYNERGOS_*` と `AWS_*` 環境変数の読取。同期ツリー内のスクリプトを参照する hook | すべて失敗、参照 hook は拒否 | SEC-AZ-10, SEC-DA-06 |
| AT-SY-SEC-13 | (a) bootstrap と招待と enrollment | 期待と違う `peer_id` の node card、署名の壊れた node card、期限切れの node card、Cernere 以外 (ノード自身・control) が署名した node card による bootstrap。使用済み・期限切れ・`invite revoke` 済みの招待の消費。`exp` が 7 日を超える招待の発行。Cernere が発行した claim code のうち使用済みのもの、発行から 16 分後のもの (control 経由の enrollment で) | bootstrap は `bootstrap.peer_mismatch` か署名検証失敗で接続しない。招待は `invite.rejected{reason}`。7 日超の招待は発行時に拒否。claim code はどちらも Cernere が拒否し、control はその 401 をそのまま返す (control 側に claim code の状態が無い) | SEC-CR-02, SEC-CR-03, SEC-AZ-03, SEC-AZ-07, G2 |
| AT-SY-SEC-14 | (d) custody の配置分離 | custody-transfer に旧 `SYNERGOS_CUSTODY_TOKEN` の bearer、猶予期間後の HMAC URL、membership LCT を提示する。control-node の Vault policy と `excubitor.secrets.json` の consumers を検査する | custody-transfer はすべて 401 / 403 (custody-capability の LCT だけを受ける)。control-node の policy と consumers に custody の鍵・受領証鍵・Cloudflare token が無い | SEC-BR-15, SEC-CR-06 |
| AT-SY-SEC-15 | (a) 未要求の送信 | 相手が WANT を出していない TXF2 の DATA、WANT と違う blake3 の DATA、quota を超える WANT への応答を送る | 本体を 1 バイトも保存せず STOP_SENDING / RESET と `receive.rejected{unsolicited\|quota}`。`.part` が作られない | SEC-DA-04, SEC-BR-01 |
| AT-SY-SEC-16 | 耐久書込のクラッシュ注入 | 置換の各段 (一時ファイル作成・fsync・rename・ディレクトリ fsync・journal 追記) の間で daemon を kill し、再起動する。Windows / Linux で | 再起動後、各ファイルは旧版か新版のどちらか完全な形で、壊れた中間状態が無い。旧版は `displaced/` にあり、manifest は作業ツリーと一致する | SEC-IN-07, SEC-IN-08, SEC-IN-15 |
| AT-SY-SEC-17 | 網羅 | 全ルート・stream 種別・gossip topic・IPC コマンドに AbusePolicy 行と認可宣言がある | 欠けが 0 | SEC-DA-22, G8 |
| AT-SY-SEC-18 | custody のライフサイクル | commit 済みで `retain_until` が未来の実体を reclaim する。一度も受け付けていない id を reclaim する。committed でない実体を reclaim し、`retain_until` の前後で GC を走らせる。custody-admin の資格で物理削除を呼ぶ | 1 つ目は 409 `retention_locked`。2 つ目は 404 で tombstone が作られない。3 つ目は tombstone だけが書かれ、`retain_until` の前の GC は削除せず、後の GC (別 ID) だけが削除する。4 つ目は該当 API が無い | SEC-IN-10, SEC-BR-16, SEC-DR-17 |
| AT-SY-SEC-19 | 検知 | canary 実体への接触、7 日基準線の 5 倍超の償還 | canary 接触は ≤60 秒で主体を freeze。基準線超過は step-up を要求 | SEC-DR-05, SEC-DR-06, G7 |
| AT-SY-SEC-20 | crypto-shred | 組織の KEK を破棄 | その組織の custody・履歴・チャンク保管の blob が復号不能、manifest と監査ハッシュの検証は通る | SEC-CR-19, G10 |
| AT-SY-SEC-21 | ゲートの自己試験 | workflow ファイル、固定されていない toolchain、未承認の依存、仕込んだ秘密 | それぞれでゲートが赤 | SEC-SC-01, SEC-SC-03, SEC-SC-16, SEC-SC-17, G9 |
| AT-SY-SEC-22 | decoder の境界と fuzz | `fuzz/registry.json` と C++ の decoder 一覧を突き合わせる。各 decoder に宣言長が上限を超える入力と、要素数が上限を超える入力を与え、確保量を数える。各 target を 30 秒 fuzz する | 未登録の decoder が 0。上限超の入力で確保が起きず `decode.too_large`。fuzz で crash 0 | SEC-DA-19, SEC-DA-02, G9 |
| AT-SY-SEC-23 | 洪水 | 1 peer から 10,000 本の stream と最大長の frame、半開きのハンドシェイク 1,000 本、control へ 10,000 件の heartbeat、custody へ 10,000 件の PUT、IPC へ 1,000 接続 | 429 / RESET が返る。daemon の RSS は基準 + 256 MiB 以内で OOM しない。別の正当な peer の新規接続が 1 秒以内に認証される (半開き接続が受付を止めない) | SEC-DA-01, SEC-DA-02, SEC-DA-12, SEC-DA-13, SEC-DA-21, G8 |
| AT-SY-SEC-24 | イベントの網羅 | ルート・コマンド・stream 種別の一覧と `spec/data/event-coverage.json` を突き合わせる | 対応の無い機微な項目が 0 | SEC-DR-02, SEC-DR-13, G7 |
| AT-SY-SEC-25 | custody ジャーナルの改ざん検知 | ジャーナルの途中の行を削除・改変・入替えした後に連鎖を検証する。checkpoint の署名を別鍵で作る | 検証が失敗し、該当の `seq` を示す。偽の checkpoint は拒否 | SEC-DR-04 |
| AT-SY-SEC-26 | 起動検査とログ秘匿 | 秘密の欠落、不正な形式、Vault の 403 / 404、loopback 以外の `http://` の各設定で起動する。種の秘密 (token 形、鍵形、`Authorization` ヘッダ) を設定・環境・IPC コマンドに入れて全ログを走査する | 各起動は鍵名を示して非 0 で終了。ログに種の値が 0 件 | SEC-SC-10, SEC-SC-12, SEC-CR-17 |
| AT-SY-SEC-27 | secrets manifest | `excubitor.secrets.json` を検査する | 同じ鍵が 2 サービスの consumers に並ばない。`kid_required` の鍵がすべて kid を持つ。2 つの kid の併用受理を試験で確かめる | SEC-DR-08, SEC-SC-09, G3 |
| AT-SY-SEC-28 | 外向き通信 | Curare の共有 vector (IPv4 射影、NAT64、6to4、メタデータ endpoint、DNS rebinding) を招待・bootstrap・`peer add-url` に与える。分類済みアドレスへの redirect | すべて接続せず `egress.denied`。redirect を追わない | SEC-DA-10, SEC-DA-11 |
| AT-SY-SEC-29 | HTTP の基線 | control・custody・admin-ui の応答ヘッダと、Host / Origin の違反要求、依存停止中の `/v1/health` | CSP・HSTS・`nosniff` が付く。Host 違反は 421、Origin 違反は 403。health は I/O なしで `{ok, service, version}` | SEC-DR-10, SEC-DR-11, SEC-AZ-25 |
| AT-SY-SEC-30 | 保持 (G6) | 単一の資格 (ノード鍵、custody-admin、control の admin、履歴ノード) ごとに、保持期限内の版の削除・上書き・GC を試みる (時計を 30 日進める)。`displaced/` の旧バイトを 13 日目に GC させる | 30 日の間、どの資格でも版が残る。13 日目の GC は旧バイトを消さず、14 日を過ぎたものだけが消える | SEC-IN-01, SEC-IN-05, SEC-IN-06, G6 |
| AT-SY-SEC-31 | 寿命 (G2) | membership LCT の `exp` を 3601 秒、custody capability を 301 秒、URL 形式を 121 秒、delegation を 3601 秒にして提示する | すべて検証で拒否 (`lct.expired` か `lct.ttl_exceeded`) | SEC-CR-03, SEC-CR-05, G2 |
| AT-SY-SEC-32 | control の不達 (D-21、G11) | 2 拠点の試験網で全 control-node を 10 分止める。別の試行で 2 時間止める。オフライン貸与を受け入れたプロジェクトと受け入れていないプロジェクトを並べる。control-node 2 台のうち 1 台だけを止める | 10 分: 5 分までは通常。5 分を過ぎると新規セッション・償還・Curare の発行が 503 `revocation_stale` になり、既存セッションの stream は続く。2 時間: 貸与の無いプロジェクトは 60 分以内に全セッションが閉じる。Cernere 署名の貸与があるプロジェクトは拠点内の列挙 peer だけが続き、失効済みでも列挙された peer は残る (受け入れた残存)。1 台停止: PEP のバンドルが 60 秒を超えて古くならない。別の試行で control は動かしたまま Cernere だけを 10 分止める: RosterStatus の `exp` を過ぎると新規セッションが拒否され、membership LCT の更新と enrollment は 503 で失敗し、control が代わりに LCT や card を出さない | SEC-AZ-17, G1, G11 |
| AT-SY-SEC-33 | 依存経路 (M30) | 両 workspace で `cargo tree -i quick-xml` と audit を実行する | サーバ workspace の audit が緑で、`quick-xml 0.30.0` がサーバの lockfile に無い。desktop の lockfile にも無い (残る場合は egui バイナリが配布物に無いこと) | SEC-SC-05 |
| AT-SY-SEC-34 | 破壊的 IPC の 2 段階 | 確認 nonce 無しの HistoryPurge、61 秒後の nonce、別接続からの nonce | すべて拒否。正しい nonce だけで実行され `ipc.destructive` が記録される | SEC-AZ-09, G5 |
| AT-SY-SEC-35 | 展開ゲート | 判定の無い `.ma`、`quarantined` の `.blend`、`clean` の `.png` を受信する | 前 2 つは `<state_dir>/quarantine/` に置かれ作業ツリーに出ない。実行ビットはどのファイルにも立たない | SEC-DA-20, AGD-AI-11 |
| AT-SY-SEC-36 | 復旧と移行 bundle | 最後の訓練成果物を 36 日前にする。不一致 1 件の成果物を置く。無署名・format 1・未登録鍵の bundle を import する | リリースゲートが赤。bundle の import はすべて拒否 (移行期間後) | SEC-DR-09, SEC-SC-11, SEC-CR-21 |
| AT-SY-SEC-37 | 署名付き成果物 | 署名の無い成果物、checksum の違う成果物、cloudflared の版違いをサイトに配る | 配置を中止して旧版が動き続け、`release.verify_failed` | SEC-SC-06, SEC-SC-07, SEC-SC-08 |
| AT-SY-SEC-38 | チャンク保管の境界 | プロジェクト P の LCT でプロジェクト Q のチャンクを `get_range`・`has` する。平文の blake3 でチャンクを問い合わせる。一覧 API でプロジェクトをまたぐ | すべて拒否か空。wire と一覧に出るチャンク ID はすべて `HMAC(project_dedup_key, ·)` | SEC-BR-13, TRN-DD-23, G4 |
| AT-SY-SEC-39 | 失効の入力 API | mTLS 無しの要求、別の SPKI の要求、同じ `idempotency_key` の再送、同じ key で内容の違う要求 | 前 2 つは 401。再送は最初の `202` を返す。内容違いは 409。受理した jti は `effective_from_seq` のバンドルに載る | SEC-AZ-17 |
| AT-SY-SEC-40 | 会員・名簿の権威 (オーナー決定 8) | control (または control の鍵を奪った攻撃者) が作った membership LCT・node card・RosterStatus・ProjectRoster・OfflineLease を PEP と peer に提示する。Cernere の LCT の `projects` を広げて control の鍵で署名し直す。control の deny 専用 kid で ctx `membership` / `node-card` / `roster-status` に署名する。control-admin で会員の追加・ノードの enrollment 承認・所有者の変更・ノード鍵の変更・宣言済み到達先の変更を呼ぶ (break-glass token でも)。`local` に許可・復帰の項目を入れたバンドル | 署名の検証で全て拒否 (ctx の `signers[]` に無い署名者は無効、`lct.widened` を含む)。control-admin の該当ルートはすべて `409 authority_is_cernere` で、Cernere のページへのリンクを返す。許可・復帰の項目を持つバンドルは拒否 | SEC-AZ-01, SEC-AZ-11, SEC-BR-04, G4 |
| AT-SY-SEC-41 | directory の改竄 | control-node が配る directory に、ProjectRoster の `nodes[]` に無い card、Cernere の署名の無い card、card と `card_hash` や鍵の合わない Presence、署名を壊した ProjectRoster を混ぜる。`endpoints[]` を書き換えた card | ノードはこれらを捨てて接続しない (`directory.entry_dropped` を記録)。正しい要素だけで接続が続く。control は要素の省略はできても、ノード・会員・到達先を足せない | SEC-AZ-01, SEC-AZ-05, SEC-CR-02 |

### 6.2 効率の上限 (E1〜E7) の計測

| ID | 計測対象 | 母集団 | 目標 |
|---|---|---|---|
| AT-SY-EFF-01 | 10,000 実体の clone の認可 (E2) | 合成プロジェクト 1 件、実体 10,000 (サイズは転送効率文書の標準分布) | Curare 呼出 ≤10 回、custody-admin 呼出 0 回 |
| AT-SY-EFF-02 | stream あたりの認可の追加往復 (E1) | 2 ノード間で 1,000 stream | 追加 RTT 0 (認可なしの基準と同じ往復数) |
| AT-SY-EFF-03 | 暗号化ありの重複排除率 (E5) | 固定コーパス (同一プロジェクト内で版を重ねたファイル群) | 暗号化なしと同一の重複排除率 |
| AT-SY-EFF-04 | 検証 CPU (E3) | custody-transfer に 1 秒あたり 1,000 件の範囲要求 | 要求あたり Ed25519 検証 ≤3、キャッシュ命中時 0 |
| AT-SY-EFF-05 | 置換の書込費用 (E6) | 1 GiB のファイルを 100 回置換 | 追加の全体コピー 0、`displaced/` への rename だけ |
| AT-SY-EFF-06 | 暗号化の読取増幅 (E7) | 1 GiB の暗号化済み実体 10 本に、境界に揃えない 1 MiB〜64 MiB の範囲読取を 1,000 回 | 各読取の復号バイト数 ≤ 要求バイト数 + 128 KiB (既定 segment)。1 MiB の範囲で ≤1.125 × |
| AT-SY-EFF-07 | 暗号化のバイト増分 (E4) | 1 MiB〜100 GB の実体を転送効率文書の標準分布で 1,000 本 | 保存バイトの増分 ≤0.1 % (既定 segment で 0.024 % + ヘッダ) |

### 6.3 その他の受入条件

- `spec/plan/redesign-analysis/findings/security-verified.json`・`transfer-verified.json`・`ai-game-verified.json` が本書と同じ変更で repo にあり、本書の参照がすべて repo 相対で辿れる。
- 契約登録ゲート (M01) が両 repo で緑で、Curare 文書の SPEC-SY-* の写しに「非規範の写し」の印がある。
- `.github/workflows/` が空になるのは、置き換えの Revisor ケースが main で 1 回通った記録の後 (M29)。
- `spec/operations.md` が存在し、`excubitor.secrets.json` の全鍵に playbook の見出しがある (M33)。
- Codex 経路の serve と ACL の監視ジョブが Excubitor に登録され、`/` の serve を仕込むと警報が出る (M34)。
- C++ 移行の completion gate (`cpp-migration.md:64-71`) に §4.37 の AT-SY-SEC の一覧が加わっている。

## 7. 未決事項と判断者

Q1〜Q10 は両文書で共通の番号と文面で、Curare 文書と同じ行を持つ。Q-SY-* は Synergos だけの問い (旧版の Q11〜Q14 を改番した)。Curare だけの問いは Curare 文書の Q-CU-*。

| Q | 問い | 判断者 | 本書が置いた既定 |
|---|---|---|---|
| Q1 | [変更提案] の採否。Synergos 側: D-06 (受領証 HMAC → Ed25519。`opaque-custody.md:12,17,21,53`、`cpp-migration.md:45-46`)、D-07 (listener とデプロイ単位の分離。`opaque-custody.md:9`)、D-09 / SPEC-SY-retention-ack (保持を同期モードの義務に。`docs/versioning-design.md:25,111`)、SPEC-SY-custody-capability-v2 (`opaque-custody.md:37` の 900 秒 → 300 / 120 秒、`:39` の取得名の束縛を `download_name` caveat で保つ、`:40` の再開方式は転送効率文書の提案)、D-25 (`opaque-custody.md:41-42` の削除手順。保持の所有者は Curare のまま)、`cpp-migration.md` の `:35-37`・`:38`・`:39-40`・`:41-43`・`:44-47`・`:48` の改訂 (§4.37)。Curare 側: Curare 文書 §2.3 の 13 項目 (`server-client-architecture.md:41`・`:46`、ML-INV-03・-06、WH-INV-01・-02・-03・-05、AU-INV-06、OB-INV-03、`site-data-transfer.md:22-23`・`:33`・format 1、`docs/architecture.md:327`) | neco | 変更提案として記載。`cpp-migration.md`・`opaque-custody.md`・Curare の spec/feature の改訂は回答を待って別の Revisor 変更で出す。旧 D-19 (i) (P2P だけのプロジェクトの会員資格方針を control に置く) はオーナー決定 8 (会員と名簿の正本は Cernere) で撤回し、Q1 から外した。`opaque-custody.md:3-5` の境界は決定 8・9 で締め直した (§1.1) |
| Q2 | オーナー決定 6 は Curare TS の認証強化も止めるか | neco | Curare 文書が持つ |
| Q3 | egui の `synergos-gui` を今使っている利用者はいるか。暫定のデスクトップ面を Tauri とする推定は正しいか | neco | D-13 の E: `desktop/` workspace へ移して accesskit を外し、C++ desktop-gui の受入まで残す。利用者がいなければ先に撤去してよい。除外指定はしない |
| Q4 | 公開 DHT は対象か | neco | 対象外。DHT は堅牢化せず削除する (D-23) |
| Q5 | 不変層用の別アカウントの保管先 (AWS Object Lock か別拠点の MinIO Lock) を用意できるか | neco / インフラ担当 | 未確認 (現時点で確保の記録が無い)。確保まで P5 は始めない |
| Q6 | 重複排除の範囲: プロジェクトだけ (既定) か、組織内の存在オラクルを代償に組織 opt-in を許すか。転送効率文書のチャンク保管と合わせる | neco | プロジェクトだけ |
| Q7 | SPEC-CR-service-principal と SPEC-CR-guest-identity を Cernere への依頼に足すか (前者の根拠に AGD-AI-12・AGD-GW-13)。SSF 拡張イベント `org-membership-removed` の確認。オーナー決定 8 で SPEC-CR-membership-issuer と SPEC-CR-node-roster を Cernere へ依頼済み (Actio `actio:2a7af0fa`・`actio:fd0e4de8`) で、SSF には `project-membership-changed`・`node-revoked`・`roster-changed` を足す。`synergos-node` を動かすエージェントやサービスは SPEC-CR-node-roster で enrollment した Cernere のサービス主体でなければならないので、SPEC-CR-service-principal の必要度が上がる | neco / Cernere オーナー | mTLS とメール OTP の暫定。会員と名簿には暫定を置かない |
| Q8 | `project_ref` と Curare space の対応 (SPEC-CU-project-descriptor が導出を固定し、その `project_ref` を Cernere に登録する前提。SPEC-CU-membership-export は取り次ぎだけの契約になった) | neco (AI・ゲーム開発文書) | 参照だけ |
| Q9 | 提案値の確認: 400 日のセキュリティイベント保持、Curare の RPO ≤15 分・RTO ≤4 時間 (G6)、G11 の可用性 99.9 % | neco | 提案値 |
| Q10 | Curare main 744f013 (Codex クラウドから Tailscale 経由で本社を使う) の露出への影響 | セキュリティ担当 | Curare 文書が持つ。Synergos 側の分は本書が扱った: custody は transfer の経路だけが tailnet に出る (§2.8 l、M34)。残存は HMAC URL の期限内再生で、C++ の M09 まで設定では塞げない |
| Q-SY-1 | 型付きの close / reset code の値: `synergos/1` と HLO1 の拒否、`0x5305 limited` | セキュリティ担当 | 未決。`0x5301`〜`0x5304` (`0x5304` は転送効率文書の `unsupported_version`) と衝突しない値を割り当てる |
| Q-SY-2 | 大量変更ブレーカーの閾値 (窓の長さ、ファイル数、バイト数、エントロピー変化) | セキュリティ担当 / プロダクトオーナー | 暫定値 60 秒・500 ファイル・1 GiB・30 % (§4.14、AT-SY-SEC-04)。P4 のランサム演習の実測で見直す |
| Q-SY-3 | 封筒暗号の segment 長の確認 | neco (転送効率文書と合わせて) | 既定 64 KiB (16 KiB〜1 MiB の 2 の累乗、実体ごとに固定)。E4・E7 と Bao の 16 KiB group に整合する。転送効率文書の側で範囲読取の粒度と矛盾が無いことを確かめる |
| Q-SY-4 | ドメイン: `cpp-migration.md:79-80`「C++ source is placed in existing module membership rather than introducing a cross-domain directory.」の下で、(a) 署名封筒・LCT を network-transport に置く (D-24 の B) か新ドメイン `trust-primitives` (A) か、(b) 新ドメイン `chunk-store` (転送効率文書) と `quality-gates` を設けるか | neco | (a) は B。(b) はどちらも設ける (1 ドメイン 1 モジュールで、cross-domain directory ではない) |
| Q-SY-5 | ctx 登録表: `release-manifest` の追加 (Curare 文書 §4.7、Q-CU-9 の要求)、Curare が自分の kid で `journal-checkpoint` と `release-manifest` に署名してよいか (`signers[]`)、`spec/vectors/ctx-registry.json` を変更できるのは誰か | neco / プロダクトオーナー | 追加を受ける。`synergos/` の接頭辞は形式を表し署名者を表さない。変更は Synergos の Revisor 変更だけで、golden vector を同じ変更で足す |
| Q-SY-6 | オフライン貸与 (D-21) の上限 (`max_stale` ≤24 時間、`exp` ≤72 時間) と、Cernere での発行の承認者 (プロジェクト所有者の step-up か、neco の承認も要るか)。`offline_lease_allowed` の既定値 | neco / プロダクトオーナー / Cernere オーナー | 上限は記載のとおり。貸与は Cernere が署名し (SPEC-CR-membership-issuer)、発行はプロジェクト所有者の step-up で、`offline_lease_allowed` が真のときだけ。Synergos は配布・検証と、使用を security イベント `revocation.lease_used` として記録する |
| Q-SY-7 | P5 の巻き戻し窓 (30 日) を過ぎた組織の暗号化を一方向として承認するか | neco | 組織ごとに承認を記録してから窓を閉じる。承認が無い間は窓を延長し KEK を破棄しない |
| Q-SY-8 | control-node を別拠点に 2 台目として置けるか (G11) | neco / インフラ担当 | 未確認 (2 台目のホストの記録が無い)。置けるまで G11 は未達として記録し、オフライン貸与の受け入れを優先する |
| Q-SY-9 | (解消済み) SPEC-CR-membership-issuer と SPEC-CR-node-roster は Actio `actio:fd0e4de8` が持つ (SPEC-CR-membership-issuer と SPEC-CR-node-roster。鍵の公開・SSF/CAEP・step-up は `actio:2a7af0fa`) | — | — |

## 8. 参考資料

設計の根拠にした外部資料だけを載せる。出典は `spec/plan/redesign-analysis/research/breach-containment.md` と `spec/plan/redesign-analysis/research/transfer-efficiency.md`。38 の RFC 8785 だけは調査ノートに URL が無く、本書で直接確かめて足した。

1. NIST SP 800-207 Zero Trust Architecture — https://nvlpubs.nist.gov/nistpubs/specialpublications/NIST.SP.800-207.pdf
2. Tailscale ACL (Codex 経路の `tag:codex` 制限) — https://tailscale.com/kb/1018-acls
3. OpenID CAEP 1.0 — https://openid.net/specs/openid-caep-1_0.html
4. OpenID Shared Signals WG — https://openid.net/wg/sharedsignals/
5. Google Workspace SSF API (SSF 受信の実装例) — https://developers.google.com/workspace/shared-signals/api/ssf-api
6. Biscuit 仕様 (LCT の代替案) — https://doc.biscuitsec.org/reference/specifications.html
7. biscuit-rust (Rust 実装しか無いことの確認) — https://github.com/eclipse-biscuit/biscuit-rust
8. SPIFFE / SPIRE server (X.509 SVID の代替案) — https://spiffe.io/docs/latest/deploying/spire_server/
9. RFC 9001 QUIC-TLS (0-RTT の再生) — https://www.rfc-editor.org/rfc/rfc9001.html
10. quinn transport config (窓・ストリーム上限) — https://github.com/quinn-rs/quinn/blob/main/quinn-proto/src/config/transport.rs
11. S3 Object Lock — https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html
12. S3 Object Lock の構成 — https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-configure.html
13. S3 の KMS 封筒暗号 — https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html
14. AWS KMS のデータ保護の推奨 — https://docs.aws.amazon.com/prescriptive-guidance/latest/aws-kms-best-practices/data-protection-encryption.html
15. Tailscale grants と ACL の違い — https://tailscale.com/docs/reference/grants-vs-acls.md
16. 3-2-1 バックアップ規則 — https://veeam.com/blog/321-backup-rule.html
17. Bao の仕様 (16 KiB group の検証範囲。E7 の segment 整合) — https://github.com/oconnor663/bao/blob/master/docs/spec.md
18. 改ざん検知ログ (Crosby & Wallach) — https://static.usenix.org/events/sec09/tech/slides/crosby.pdf
19. Sigstore のセキュリティモデル — https://docs.sigstore.dev/about/security/
20. SLSA v1.0 levels — https://slsa.dev/spec/v1.0/levels
21. GitHub Actions の安全な利用 — https://docs.github.com/en/actions/reference/security/secure-use
22. tj-actions/changed-files の供給網攻撃 (CVE-2025-30066) — https://www.wiz.io/it-it/blog/github-action-tj-actions-changed-files-supply-chain-attack-cve-2025-30066
23. Codecov bash uploader 事件 — https://www.cybersecuritydive.com/news/codecov-bash-uploader-supply-chain-attack/601757/
24. RustSec advisory DB (quick-xml の勧告の参照元) — https://github.com/matrix-org/rustsec-advisory-db
25. Unity UPM の dependency confusion — https://unity.com/blog/engine-platform/upm-dependency-confusion-assetbundle-security-in-the-editor
26. gVisor の構成 — https://gvisor.dev/docs/architecture_guide/intro/
27. gVisor と seccomp — https://gvisor.dev/blog/2024/02/01/seccomp/
28. Blender のスクリプト実行とセキュリティ — https://docs.blender.org/manual/nl/5.2/advanced/scripting/security.html
29. Maya の悪性スクリプト (Autodesk フォーラム) — https://forums.autodesk.com/t5/maya-forum/maya-malicious-script-security-update-and-resources/td-p/9597247
30. Maya マルウェアの配布事例 — https://av-comparatives.org/maya-malware-fab-com/
31. OWASP SSRF 防止 cheat sheet — https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html
32. OWASP ASVS 5.0 — https://github.com/OWASP/ASVS/tree/master/5.0/en
33. honeytoken の設置 — https://zeltser.com/plant-honeytokens
34. Insomniac Games のランサム被害 — https://Techraptor.net/gaming/news/insomniac-games-ransomware-attack-report
35. GTA6 漏えい (フィッシング起点) — https://80.lv/articles/gta-6-files-were-allegedly-leaked-because-of-phishing-email
36. CircleCI 2023-01 事件報告 (長寿命資格) — https://circleci.com/blog/jan-4-2023-incident-report
37. TPN 評価 (発注元のセキュリティ要件) — https://www.ttpn.org/2024/07/tpn-assessment-what-you-need-to-know/

38. RFC 8785 JSON Canonicalization Scheme (D-04、SPEC-SY-signed-envelope) — https://www.rfc-editor.org/rfc/rfc8785
39. IETF HTTP Resumable Upload (upload_id 単位の capability。正本は転送効率文書) — https://github.com/httpwg/http-extensions/blob/main/draft-ietf-httpbis-resumable-upload.md
