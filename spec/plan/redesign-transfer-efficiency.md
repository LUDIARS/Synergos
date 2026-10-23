# Synergos 再設計 — 転送速度と効率

- 作成日: 2026-10-23
- 対象: Synergos main @ d404f20
- 関連テーマ: 本書は 3 テーマのうち transfer-efficiency (転送速度と効率) の Synergos 側正本。同 repo の他テーマは `spec/plan/redesign-security.md` (脆弱性と侵害の歯止め。資格・失効・暗号・チャンク保管のセキュリティ不変条件を持つ) と `spec/plan/redesign-ai-game-dev.md` (AI 開発・ゲーム開発のワークフロー。VFS 展開と `project_ref` ↔ Curare space の対応を持つ)。相手 repo の同名文書は Curare 側: spec/plan/redesign-transfer-efficiency.md (本書からの相対パス `../../../Curare/spec/plan/redesign-transfer-efficiency.md`。Curare 側文書から本書へは `../../../Synergos/spec/plan/redesign-transfer-efficiency.md`)。契約 ID・機構 ID (TRANSFER-Mxx)・判断 ID (D-Txx)・段階 (T0〜T5)・未決事項の Q 番号は両文書で同一。
- 根拠資料: 本 repo の `docs/versioning-design.md`、`spec/feature/opaque-custody.md`、`spec/feature/native-contracts.md`、`spec/plan/cpp-migration.md`、`DESIGN.md` §15、`docs/mesh-operations.md`、`excubitor.catalog.yaml`。解析成果 `spec/plan/redesign-analysis/` の転送テーマの発見 (`findings/transfer-*.json` の 4 ファイル。検証で採用した 79 件のうち Synergos に関わる 65 件を §2 に載せ、棄却された発見は使わない)、読解マップ (`maps/sy-*.md`)、調査ノート (`research/transfer-efficiency.md`、`research/vcs-game-ai.md`、`research/ai-dev-data.md`)。外部資料は §8。
- 判断の前提: 2026-10-23 のオーナー決定 6 点 (`spec/plan/redesign-analysis/HANDOFF.md` §7.1)。決定 3 により CDC を再検討し、チャンク保管を独立モジュールにする案を 4 軸で比べる (§3.3、§4.19)。決定 6 により、本書の Synergos 実行時変更はすべて **C++ 移行の互換契約 / ネイティブ実装要件** として書き、現行 Rust 実装へは入れない (試験専用の計測ハーネスを除く)。

## 0. 結論 (先に)

- **足りないもの**: Synergos は内容を 1 ファイルずつ、1 バイトにつき何度も読み、宣言されていないトポロジの上で運ぶ。同一性は manifest の CRC32 + size で、転送のたびに計算する BLAKE3 は捨てている (TRN-DD-05)。peer の wire は版なし・要求されない全体 push で、quinn 既定窓のため 100 ms RTT で 1 stream 約 100 Mbps が上限 (TRN-WP-01)。再開・範囲・送信元選択が無く、保持者 N で N 部が届く (TRN-WP-04, TRN-WP-19)。受信側のディスク I/O は実体の約 6 倍 (TRN-WP-07)。変更は 1 hop で止まり、中継のデータ経路も rendezvous も無い (TRN-TP-01, TRN-TP-03, TRN-TP-14)。custody は単発 PUT・Range なし・verify のたびに全再読込で、100 GB のチェックポイントはどの経路も通れない (TRN-DD-19)。
- **作り直すもの**: (1) BLAKE3-256 を正準の内容同一性にし、Bao outboard で範囲検証する。読込はバイトごとに 1 回、書込も 1 回に揃える (reflink の無い NTFS では作業コピーの書込 1 回を足し、利用者が編集するファイルにハードリンクは張らない。M01, M02)。(2) 受信側主導・範囲指定・再開可能・複数送信元の pull (TXF2) と、署名 Head + HaveSet による状態ベースの複製に置き換える (M07, M09)。(3) control が Cernere 署名の node card と ProjectRoster を束ねた directory を配り (control は署名せず、会員もノードも決めない。オーナー決定 8)、拠点 seed・DERP 型 relay (C++ 版だけ。それまでは Tailscale、オーナー決定 7)・edge を 1 バイナリの役割として動かす (M10〜M13, M17)。(4) custody を再開可能 upload・Range GET・受領証 1 回生成に変える (M14)。(5) CDC は **独立モジュール `synergos-chunkstore`** として常時稼働の役割 (seed / archive / custody / edge) にだけ入れ、メンバーは chunker を持たずに range plan で恩恵を受ける (M27, M28)。
- **着地点**: すべて `cpp-migration.md` の "Compatibility contracts" 改訂と `native-contracts.md` の新 `[id:]` 節として書く (§4.31, §4.32)。Rust には計測ハーネスと設定上の暫定運用だけを入れる (M31, M33)。
- **順序**: T0 契約・ベースライン計測 → T1 Curare 側フロー (Curare 文書) → T2 C++ ネイティブ中核 (security P3/P4 と同じ組織単位の flag day。flag day 後も 30 日の巻き戻し期間を置く、§5.3) → T3 トポロジ → T4 配置と edge → T5 チャンク層。
- **現状のまま運用した場合の最悪シナリオ**: 100 GB のチェックポイントは custody の 1800 秒上限・verify の 60 秒上限・S3 単発 PUT 5 GB のどれかで止まり、版にならない (TRN-DD-19, TRN-DD-10)。招待で作った星形では、アーティスト 1 人の 500 MB PSD 保存が他メンバーへ届かないか、N 部の全量で届く (TRN-TP-01, TRN-WP-19)。

## 1. 背景・ゴール・非ゴール

### 1.1 背景

現状設計の要点 (既存 spec の引用):

- `docs/versioning-design.md:11`: 「バイナリ / 大きいファイルの **差分転送** をするか | **しない**。転送は Phase 1 のまま全体転送 (blake3 検証)。差分の検知は `version` + size + CRC で足りる。CDC チャンク重複排除案は neco 決定 (2026-08-16) で取り下げ」。
- `docs/versioning-design.md:318-319` (§3.7): 「差分転送は帯域の最適化であり、LAN / Cloudflare Mesh の帯域では必須ではない」。同 `:324-325`: 「将来『大容量の一部更新が頻繁で帯域が足りない』が実測で出たら、履歴ノードの objects をチャンク化する形で案 B を**履歴ノードの内側だけに**後付けできる (通常ノードの挙動は変えない)」。
- `docs/versioning-design.md:151`: 「版の実体は **チャンク化しない**。ファイル全体を objects に置く。重複排除はファイル単位のみ」。
- `spec/feature/native-contracts.md:24` (`SPEC-SY-native-metadata`): 「Unchanged size/CRC retains its existing version, matching the Rust contract.」
- `spec/plan/cpp-migration.md:39-40`: 「Peer protocol: preserve protobuf schemas, QUIC ALPN, peer authentication, CRC/hash validation, transfer cancellation, and catalog exchange.」
- `spec/feature/opaque-custody.md:40`: 「中断後は同じ受付から再PUTできる。バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送。」
- `synergos-net/src/transfer/mod.rs:11-12`: 「再送 / 並列取得 / resume は追って拡張する。現バージョンは単一ストリームで順次送る最小仕様。」

境界 (2026-09-08 決定、`spec/feature/opaque-custody.md:3-5`) は維持する。Curare が版・提出・許可・配置方針を持ち、Synergos は不透明な実体を運んで耐久受領証を出し、Cernere が本人・組織を持つ。本書の配置 (placement) も方針は Curare、実行は Synergos と分ける。

2026-10-23 夕の決定 8・9 で、この境界を次のように締める。Cernere は本人・組織・所属に加えて、**Synergos のプロジェクト会員 (誰がどのプロジェクトで read / publish / history か) とノード名簿 (どのノードが誰のもので、どこに到達できるか) の正本**である。Synergos は転送と同期の本体で、Curare が無くても動く。Curare はデザイナー向けの版・レビュー・承認を持ち、それが要る場面でだけ capability を出してバイトの移動を認可する。Curare はバイトを中継せず、会員資格を輸出・発行・判定しない。

ユーザー要求は、AI 開発・ゲーム開発の課題解決、セキュリティ、転送速度と効率の 3 条件を同時に満たすことである。本書は転送速度と効率を扱い、セキュリティ文書の上限 E1〜E6 (認可の追加往復 0、取得の Curare 呼出 ⌈N/1000⌉、暗号化の負荷 ≤0.1 %、暗号化の有無で重複排除率が同じ など) と失効 60 秒の目標 (security G1) を破らない。

オーナー決定 (2026-10-23) のうち本書を拘束するもの:

| # | 決定 | 本書での扱い |
|---|---|---|
| 2 | 提出経路は Revisor のみ | 全ゲートは Revisor のマージ前ケース。GitHub CI は置かない (security D-12) |
| 3 | CDC を再検討。チャンク保管の独立モジュール化もありうる | §3.3 で「Synergos 本体に入れる案」と「独立モジュール案」を 4 軸で比べ、独立モジュールを採用。境界は SPEC-SY-chunk-store (§4.19) |
| 4 | Cernere 側の機能は SPEC-CR-* として依頼済み | 署名鍵・失効通知を依存として参照し、Synergos には実装しない (§4.27) |
| 5 | quick-xml 0.30 は「依存自体が要るか」から判断 | 転送機能は `synergos-gui` 経路に依存しない事実を示し、security D-13 / SECURITY-M30 (P0 で `desktop/` workspace へ移し `accesskit` feature を外す) を支持 (§3.18) |
| 6 | HLO2・custody 強化・認証強化は C++ 移行契約として再仕様化 | 各機構に「互換契約の変更 / ネイティブ実装要件 / 運用」の区分を付ける (§4.28。区別の定義は `spec/plan/redesign-analysis/HANDOFF.md` §6) |
| 7 | 外向きの DERP 型 relay は C++ 版でだけ作る。それまで拠点と外部環境は Tailscale (`synergos-node` は tailnet IP にだけ bind) | SPEC-SY-relay-auth / SPEC-SY-relay-v2 は C++ 移行の契約。現行 Rust には何も足さない (`design/authority-amendment.md` §3) |
| 8 | プロジェクト会員とノード名簿の正本は Cernere。synergos-control は Cernere 署名の会員証・名簿を配布・キャッシュ・検証するだけ | 会員証 (membership LCT)・node card・ProjectRoster・RosterStatus・OfflineLease は Cernere が署名する。control は会員を決めず、その発行鍵を持たない。enrollment の承認は Cernere (step-up)。失効は Cernere から SSF/CAEP で流れる (SPEC-CR-membership-issuer、SPEC-CR-node-roster。Actio `actio:2a7af0fa`、`actio:fd0e4de8`) |
| 9 | Curare はデザイナー向けの版管理で、Synergos のハコ | 転送・同期の本体は Synergos 単体。Curare が関わるのは版・レビュー・承認が要る場面だけ。Curare はバイトの経路でも会員資格の権威でもない |

### 1.2 ゴール (測定可能な到達条件)

「(推定)」はコード定数や調査から導いた値で、T0 のベースライン計測で置き換える。計測プロファイル: `lan` (1 ms, 10 GbE)、`wan30` (30 ms, 1 Gbps)、`wan100` (100 ms, 1 Gbps)、`wan100-loss` (100 ms, 1 Gbps, 0.1 % ランダム損失)。

| # | ゴール | 目標値 | 現状 | 発見 |
|---|---|---|---|---|
| TG-01 | 単一実体の throughput (損失なし WAN) | `wan100` で 4 GiB 1 本・送信元 1・QUIC 接続 1 で **≥800 Mbps** | stream あたり約 100 Mbps (1.25 MB 窓) | TRN-WP-01, TRN-WP-03 |
| TG-02 | 単一実体の throughput (損失あり WAN) | `wan100-loss` で送信元 k=4 か BBR 有効で **≥300 Mbps** (推定。損失ベースの単一フローは Mathis 上限でより低く抑えられるため、T0 のベースラインで再設定) | 未計測 | TRN-WP-01, TRN-DD-19 |
| TG-03 | LAN | `lan` で 4 GiB 1 本 **≥5 Gbps**。同一拠点の組は overlay を折り返さない | overlay 折返し | TRN-TP-05, TRN-WP-01 |
| TG-04a | ディスク増幅 (reflink 可能な FS: Btrfs・XFS・APFS・ReFS) | 受信側の書込 ≤ **1.1×**、余分な全量読込 **0**。publisher の読込 ≤ **1.05×** | 受信約 6×、publish 4〜5× | TRN-WP-06, TRN-WP-07, TRN-DD-06 |
| TG-04b | ディスク増幅 (reflink の無い FS: ReFS でない NTFS、ext4) | 受信側の書込 ≤ **2.05×** (object 1 回 + 作業コピー 1 回)、余分な全量読込 **0**。publisher の I/O は読込 ≤ **1.05×** + 書込 ≤ **1.05×** (ハッシュしながらのコピー 1 回、合計 ≤ **2.05×**)。利用者が編集するファイルにハードリンクを使わない (§4.2) | 同上 | 同上 |
| TG-05 | 再開 | 50 % で kill 後の再送 ≤ **55 %**。経路は 4 つ: (1) P2P の TXF2、(2) custody upload (IETF 再開可能 upload)、(3) custody / edge / seed の範囲 GET (SPEC-SY-object-get)、(4) Curare client の download (`.part` 再開)。MinIO の S3 multipart は Curare が正本で Curare 側文書 §6 が測る | 100 % | TRN-WP-04, TRN-DD-03, TRN-DD-09, TRN-DD-13 |
| TG-06 | サイズ上限 | **100 GB** の実体が upload・受領証・版作成・clone を idle timeout だけで完了。24 時間未満の wall-clock 上限なし。計測用組織の `maxObjectBytes` は ≥100 GB に設定し、custody 全体の上限 `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` は T2 で引き上げる (§4.14) | 不可能 (custody 上限は 10 GiB、`excubitor.catalog.yaml:31-32`) | TRN-DD-19, TRN-DD-10 |
| TG-14 | offload のメモリ | 10〜100 GB の実体の S3 / GDrive への offload で daemon の最大 RSS ≤ **256 MiB** | 実体全体を RAM に読む | TRN-DD-08 |
| TG-07 | **500 MB PSD の再転送率** | 全体転送プロジェクト: 設計上 100 % だが**拠点あたり 1 回** (TG-08)。チャンク有効プロジェクト: レイヤの ≤25 % を触る保存 1 回で、seed↔custody とメンバーの range plan 取得の再送 ≤ **35 %** (暫定・推定。公開された PSD の比率が無い)。セキュリティ由来の増加は ≤1 ポイント (security E5) | 100 % | TRN-DD-01, TRN-WP-12 |
| TG-08 | publisher の送出量 | 拠点 seed がある場合、メンバー数によらず拠点あたり ≤ **1.2×**。保持者 3 で各受信者の流入 ≤ **1.05×** | N 部 | TRN-TP-02, TRN-WP-19 |
| TG-09 | 伝播 | publish が全オンラインメンバーに **60 秒 p95** 以内で見える (星形を含む全トポロジ)。フレーム 1 個損失で最悪 **90 秒**。再参加ノードは照合 1 回で追い付く | 1 hop で停止 | TRN-TP-01, TRN-WP-02, TRN-WP-18 |
| TG-10 | 中継 | RelayOnly の組の **100 %** にデータ経路。中継 throughput ≥ relay ホストの回線速度の **80 %**。relay 停止から **10 秒**以内に別 relay へ移る (C++ relay でだけ測る。Rust には relay の経路を作らない、オーナー決定 7) | データ経路なし | TRN-TP-03, TRN-WP-16 |
| TG-11 | 再接続の嵐 | seed 1 台が **500** クライアントを **10 秒**以内に受け入れる | accept が直列 | TRN-TP-15 |
| TG-12 | daemon の準備完了 | 10 GB / 20 万ファイルのプロジェクトで ≤ **5 秒**。`status` は変更のないファイルを **0** 回読む | 数分〜1 時間 | TRN-WP-14, TRN-DD-21, TRN-E2-15 |
| TG-13 | Synergos メタデータ | 受信バッチあたりの書込が O(変更行数)。10^5 ファイルの checkout のメタデータ書込 ≤ 最終 manifest サイズの **2×** | O(n²) | TRN-E2-13, TRN-WP-13, TRN-DD-16 |
| CG-06 | IPC | 結果はサイズによらず ≤1 MiB のフレームで分割して流す。長い操作は **100 ms** 以内に job id を返す | 合計 1 MiB 上限、投げっぱなし | TRN-E2-14 |

Curare 側の制御経路のゴール CG-01〜CG-05 (保存の往復 ≤2+⌈N/10 000⌉、clone の往復 ≤⌈N/1000⌉、本人確認の費用、API p95 <1 秒、依存登録) は Curare 側: spec/plan/redesign-transfer-efficiency.md が正本。Synergos は CG-01 と CG-02 を満たすための custody 契約 (M14, M15, M17) を提供する。

失効のガードレール (security G1 が正本): 進行中の TXF2 転送・relay セッション・custody / edge の範囲応答は、失効から **60 秒**以内に切る。custody と edge は配信中も 30 秒以下の間隔で失効バンドルを再確認し、クライアントの範囲要求は ≤1 GiB にして要求ごとに再検証させる。

### 1.3 非ゴール

| 範囲 | 扱い | 判断者 |
|---|---|---|
| 資格・署名・失効・暗号化の方式そのもの | セキュリティ文書 (`spec/plan/redesign-security.md`) が正本。本書は参照のみ | セキュリティ担当 |
| プロジェクト会員とノード名簿の決定・署名 (会員証 = membership LCT、node card、ProjectRoster、RosterStatus、OfflineLease、enrollment の承認) | Cernere が正本で唯一の発行者 (オーナー決定 8、SPEC-CR-membership-issuer、SPEC-CR-node-roster)。synergos-control と本書の機構は検証・配布・キャッシュだけを行い、会員・所有・到達性を決めず、その発行鍵を持たない (§4.27) | Cernere 担当 (依頼は Actio `actio:2a7af0fa`、`actio:fd0e4de8`) |
| VFS による遅延展開 (ProjFS / NFS loopback / FUSE) | ai-game 文書 (`spec/plan/redesign-ai-game-dev.md`) が持つ。本書は SPEC-SY-object-get を土台として提供するだけ | neco |
| Curare の API・テーブル・トランザクション | Curare 側: spec/plan/redesign-transfer-efficiency.md | neco |
| 現行 Rust 実行時の機能追加・不具合修正 | オーナー決定 6。P0 級の 3 件 (TRN-WP-05, TRN-WP-18, 偽の `Ok(Relay)`) の扱いは Q4 | neco |
| manifest の削除 (tombstone)・rename の表現 | ai-game 文書 (`spec/plan/redesign-ai-game-dev.md` の AIGAME-M07 と SPEC-SY-tombstone) が持つ。本書の manifest v2 と TXF2 は削除を運ばない。rename の内容転送 0 バイトは M02 の内容索引で満たす (§2.4、Q21) | neco |
| テキストの差分・マージ | git の仕事 (`docs/versioning-design.md:349`)。変更しない | — |
| 公開 DHT・インターネット規模の P2P | 採用しない (security Q4 と同じ) | — |

## 2. 現状分析 — 足りない要素と再設計すべき点

### 2.1 発見一覧 (Synergos 関係の検証済み 65 件)

区分は template の 5 種へ写す (design-gap → 設計ギャップ、inconsistency → 設計ギャップ (不整合)、inefficiency → 非効率、missing-feature → 未実装機能、missing-control → 欠落機構)。両 repo の転送文書はこの写し方を共通に使う (Curare 側文書の「欠落機能」「不整合」の表記はこの 5 種へ揃えるよう Curare 側文書の担当へ依頼する)。重大度は発見の severity (高・中・低) で、検証時の優先度 P0〜P2 は Curare 側文書の表に併記がある。行番号は d404f20 で再確認した。Curare の行は「Curare:」を付ける。

#### 2.1.1 トポロジ・中継 (TP)

| ID | 区分 | 現状 (file:line 根拠) | 影響 (AI/ゲーム制作での実害) | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| TRN-TP-01 | 設計ギャップ | gossip は局所配信だけで他ピアへ転送しない (`synergos-net/src/gossip/node.rs:244-251`)。`hops: 0` (`synergos-core/src/presence/mod.rs:208-209`)。受信後に再 offer しない (`synergos-core/src/exchange/mod.rs:800-805`)。push は既存の直接接続が必須 (`synergos-net/src/quic/mod.rs:359-363`) | 招待で作る星形ではハブ以外の publish が他メンバーへ届かず、アーティスト間で版が黙って分かれる | 高 | M09 署名 Head + HaveSet の反エントロピー |
| TRN-TP-02 | 非効率 | `fetch_file` が `source_peer` を無視して FileWant を全体送信 (`synergos-core/src/exchange/mod.rs:1249-1252`)。全保持者が全量 push (`exchange/mod.rs:933-954`)。同一版の重複 push を受理 (`exchange/mod.rs:764-769`) | publisher が N 部送出、再参加ノードは最大 K 部受信。始業時の同期で帯域を浪費 | 高 | M07 受信側主導 pull、M08 送信元選択 |
| TRN-TP-03 | 未実装機能 | Conduit が中継試行で偽の `Ok(RouteKind::Relay)` を返す (`synergos-net/src/conduit/mod.rs:421-427`)。RelaySession は構築されない (`synergos-net/src/relay/mod.rs:137-140`)。relay は room 放送で背圧時に黙って捨てる (`synergos-relay/src/server.rs:80-87`) | NAT 越しのメンバーにデータ経路が無いのに、状態表示は中継成功 | 高 | M12 relay v2 |
| TRN-TP-04 | 設計ギャップ | 大域 IPv6・UPnP 応答・PATH 上の cloudflared で FullNode と推測 (`synergos-net/src/promotion.rs:88-94`, `:177-195`, `:199-205`) | 到達できないノードを直接接続可能と広告し、接続失敗が続く | 高 | M11 候補収集と疎通確認、M10 `candidates` |
| TRN-TP-05 | 設計ギャップ (不整合) | P2P は Cloudflare Mesh、Curare / custody は Tailscale に移った (`docs/mesh-operations.md:16-18`, `docs/mesh-two-node-checklist.md:19-20`)。設定した UDP payload 長は適用されない | 同じ LAN のピアがインターネットを折り返す。メンバーは VPN クライアントを 2 つ動かす | 中 | M11 overlay は下層の選択肢、M33 運用 |
| TRN-TP-06 | 設計ギャップ | 割当は設定した 1 node id だけ (Curare: `curare-server/src/db/storage-node-allocation.ts:10,16,21`)。Synergos catalog も custody 1 台 (`excubitor.catalog.yaml:27-30`) | 全拠点の全席が HQ から全バイトを取る | 高 | M16 (配置は Curare 正本、複製は SPEC-SY-custody-replication)、M17 |
| TRN-TP-07 | 設計ギャップ (不整合) | Curare は capability URL の origin を `SYNERGOS_STORAGE_URL` と同一に強制 (Curare: `curare-server/src/storage/synergos-storage.ts:23-26`)。Synergos catalog は `SYNERGOS_CONTROL_BIND: 127.0.0.1:4250` と `SYNERGOS_CUSTODY_PUBLIC_URL: http://localhost:4250` の単一 origin (`excubitor.catalog.yaml:24-29`)。admin / transfer の分割は Curare `spec/feature/codex-cloud-access.md:10-13` にあり (管理 API は 127.0.0.1:4250、受け渡しは tailnet:4443)、d404f20 の catalog は単一 origin のまま。CORS も localhost だけ (`excubitor.catalog.yaml:33`) | 分割構成では upload / download が 503。edge や拠点別 URL を置けない | 高 | M16 split origins (T0、Curare 正本) |
| TRN-TP-09 | 設計ギャップ | 履歴ノードは既定無効 (`synergos-net/src/config.rs:288-292`)、広告も発見もされない (`synergos-core/src/daemon.rs:380-386`)、応答は push 1 本扱い (`exchange/mod.rs:959-971`) | 常時稼働の供給元が無く、publisher の電源断で取得不能 | 中 | M13 seed / archive、M10 |
| TRN-TP-10 | 設計ギャップ | DHT の announce はローカル表だけ (`synergos-net/src/dht/node.rs:293-296`)、`find_peer` もローカルだけ (`dht/node.rs:121-123`)、presence は使わない (`synergos-core/src/presence/mod.rs:219-227`) | 発見に寄与しないのに、接続中の全ピアへ DHT1 ハンドラが露出 | 中 | M10 directory。DHT1 は移植しない |
| TRN-TP-11 | 非効率 | mesh が graft されず全接続へフォールバック (`synergos-net/src/gossip/node.rs:171-173`、daemon 側の全接続フォールバックと (message, peer) ごとの Control stream は `synergos-core/src/daemon.rs:870-885`)。publish ごとに全ピアが catalog を取得 (`synergos-core/src/daemon.rs:1009-1027`、`synergos-core/src/catalog_sync.rs:175-178`) | gossip 量が N×M (mesh で N²×M)。大きなツリーの publish で制御トラフィックが膨らむ | 中 | M09 1 publish バッチ 1 Head、M10 プロジェクト単位のピア集合 |
| TRN-TP-12 | 設計ギャップ | 再接続 URL はメモリだけ (`synergos-core/src/presence/mod.rs:132`)、projects に永続しない (`synergos-core/src/project.rs:231-239`)、再接続は 1 ピア 10 秒の逐次 (`synergos-core/src/peer_link.rs:24-26`) | 招待で参加したノードは daemon 再起動後に戻れない | 中 | M10 Cernere 署名 card の永続キャッシュ、M09 接続時の照合 |
| TRN-TP-13 | 未実装機能 | RTT はハンドシェイク時に固定 (`synergos-net/src/quic/mod.rs:274,455`)、帯域推定は書かれない (`quic/mod.rs:421-430`, `:504-506`) | 送信元選択と窓調整の入力が存在しない | 中 | M08 計測と送信元スコア |
| TRN-TP-14 | 設計ギャップ | heartbeat 応答の本文を読まない (`synergos-core/src/control_heartbeat.rs:116-121`)。node 向けの directory が無い (`synergos-control/src/api/mod.rs:43-90`)。データ面は TOML と招待で構成 (`synergos-core/src/daemon.rs:445-448`) | control の知識がデータ面の構成に反映されない | 中 | M10 SPEC-SY-directory |
| TRN-TP-15 | 非効率 | accept と Hello の 5 秒待ちが 1 本ずつ (`synergos-net/src/quic/mod.rs:445-465`, `synergos-net/src/quic/hello.rs:31`)。accept した接続は 1 本ずつ spawn に渡る (`synergos-core/src/daemon.rs:750-753`)。再接続は固定 15 秒・jitter なし (`synergos-core/src/peer_link.rs:24`、`RECONNECT_INTERVAL = Duration::from_secs(15)`) | ハブ再起動後、300 席の再接続が直列に並ぶ | 中 | M08 (security NR-1 と同じ改修) |
| TRN-TP-16 | 設計ギャップ (不整合) | 移行計画は「preserve protobuf schemas, QUIC ALPN」と書く (`spec/plan/cpp-migration.md:39-40`) が、実体は ALPN なし・版なしの stream magic (`synergos-net/src/quic/mod.rs:543-546,578-580`, `synergos-net/src/quic/hello.rs:28`) | relay framing・役割・Head・範囲取得を互換を保って導入できない | 中 | M06 SPEC-SY-peer-wire、M32 |
| TRN-TP-17 | 設計ギャップ | 目標トポロジが宣言されていない (傘の発見)。偽の中継 (`synergos-net/src/conduit/mod.rs:421-427`)、gossip の非転送 (`synergos-net/src/gossip/node.rs:244-251`)、Curare の単一割当 (Curare: `curare-server/src/db/storage-node-allocation.ts:21`) | 役割・配置・発見・中継・経路選択が暗黙か不在。2 製品が別の overlay を使いトポロジ情報を共有しない | 高 | D-T07 採用案 E (M09〜M17) |

#### 2.1.2 経路全体とメタデータ (E2)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| TRN-E2-02 | 非効率 | Curare は受領証の署名を捨て、版作成時に custody verify を再度呼ぶ (Curare: `curare-server/src/routes/versions.ts:61`, `curare-server/src/storage/synergos-storage.ts:42-45`)。verify は実体を全量再読込 (`synergos-custody/src/store.rs:141-168`) | upload 1 回で全量再読込が 2 回。大きな実体は 60 秒の client timeout で失敗 | 高 | M14 受領証の 1 回生成、Curare M23 |
| TRN-E2-05 | 非効率 | 取得計画が閉包を逐次 BFS で解き、capability を一括発行 (Curare: `curare-server/src/db/acquisition-repository.ts:76-90`, `:102-103`)。custody は単体の admin 呼出しか持たない (Curare: `curare-server/src/storage/synergos-storage.ts:48-50`) | 1 計画で最大 1000 本の並列 HTTP が custody へ | 高 | Curare M21 (SPEC-CU-fetch-session)。Synergos は M14 / M17 の SPEC-SY-object-get で set-scoped capability 1 本を受け、集合内の全実体を認可する (security D-18) |
| TRN-E2-13 | 設計ギャップ | manifest と state を 1 ファイルごとに複製・全書換 (`synergos-core/src/project.rs:451-456`, `:484-501`, `synergos-core/src/manifest.rs:162-167`) | 10^5 ファイルの checkout でメタデータ書込が O(n²) | 高 | M04 SQLite live store |
| TRN-E2-14 | 設計ギャップ | IPC はフレーム 1 MiB (`synergos-ipc/src/transport.rs:39`)、ページングなし。checkout は job id なしの 1 往復 (`synergos-ipc/src/command.rs:93` の `ProjectCheckout`、結果は `synergos-ipc/src/response.rs:101-110` の `CheckoutReportDto`)。publish は 1 呼出 1024 パスまで (`synergos-ipc/src/command.rs:227`) | 大きなツリーの一覧が返らず、進捗を追えない | 中 | M26 IPC v2 |
| TRN-E2-15 | 非効率 | checkout と daemon 起動で全ファイルの CRC を再計算 (`synergos-core/src/checkout.rs:214-221`, `synergos-core/src/daemon.rs:265`, `synergos-core/src/restore.rs:44`) | 起動と clone のたびに全量読込 | 中 | M05 local index |
| TRN-E2-16 | 未実装機能 | `sync_mode` は保存されるだけで参照されない (`synergos-core/src/checkout.rs:80-109`)。Curare は per-file URL で全量取得 (Curare: `curare-server/src/client/local-acquisition.ts:68-71`, `curare-server/src/storage/port.ts:79-86`) | 疎な作業ツリーやデータセットを全量展開する必要がある | 高 | M17 object-get、M30、Curare M21 |
| TRN-E2-18 | 非効率 | egui GUI は 2 秒ごとに再描画して daemon を問い合わせる (`synergos-gui/src/app.rs:134`)。Subscribe はどの GUI も使わない | GUI がポーリングで daemon と IPC を占有 | 低 | M26 `Subscribe` |

#### 2.1.3 wire と I/O 経路 (WP)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| TRN-WP-01 | 非効率 | `build_transport` は stream 数・idle・keep-alive 以外を quinn 既定のまま (`synergos-net/src/quic/mod.rs:558-573`)。quinn-proto 0.11.19 (`Cargo.lock:5238`) の既定 stream 窓は 1.25 MB。`max_udp_payload_size: 1452` は設定のみ (`synergos-net/src/config.rs:694-699`) | 100 ms RTT で 1 ファイル約 100 Mbps が上限。チェックポイントやシネマ素材が WAN で律速 | 高 | M08 transport profile |
| TRN-WP-02 | 設計ギャップ | keep-alive と再接続はあるが、切断中の offer / want と転送を再送しない (`synergos-net/src/quic/mod.rs:159-178,571`, `synergos-core/src/peer_link.rs:24,130-136`, `synergos-core/src/daemon.rs:884-901`) | 切断窓に起きた更新が欠けたまま残る | 中 | M09 状態ベースの照合 |
| TRN-WP-03 | 非効率 | 1 ファイル 1 stream、64 KiB 固定の msgpack frame (`synergos-net/src/transfer/mod.rs:11-12`, `:93-99`, `:324-330`) | 送信で frame ごとに 2 コピー・2 書込。ディスクと網の重なりは 64 KiB 1 本 | 高 | M07 frame 階級、M08 パイプライン |
| TRN-WP-04 | 未実装機能 | header に範囲が無い (`synergos-net/src/transfer/mod.rs:27-46`)。失敗で部分ファイルを消す (`synergos-core/src/exchange/mod.rs:731-738`)。受信側の確認なしで Completed (`exchange/mod.rs:638,670-674`) | 中断すると数時間の転送が 0 から | 高 | M07 TXF2 の `.part` + bitmap と RECEIPT |
| TRN-WP-05 | 設計ギャップ | 取得 pin は 5 分の定数 (`synergos-core/src/exchange/mod.rs:26`、`PINNED_FETCH_TTL_MS`) で、検査は全量受信の後 (`exchange/mod.rs:731,740,754-755`, `:764-781`) | 遅い回線での大きな restore / checkout が全量受信後に stale として削除される | 中 | M07 pin を `req_id` に束縛、M33 暫定運用 |
| TRN-WP-06 | 非効率 | publish で CRC・履歴コピー・コピーの CRC・BLAKE3 と 4〜5 回読み、要求者ごとに再ハッシュ (`synergos-core/src/ipc_server.rs:1323-1327`, `:1386-1402,1417`, `synergos-core/src/history/store.rs:681,685,699`) | 大きな PSD の publish で初バイト送出までが長い | 高 | M01 単一パス、M02 reflink |
| TRN-WP-07 | 非効率 | 受信で 2 回書いて 4 回読む (`synergos-net/src/transfer/mod.rs:166,195-196,232`, `synergos-core/src/exchange/mod.rs:740`, `:795,821-833,834-843`) | 100 GB 受信で約 600 GB のディスク I/O | 高 | M02 単一書込の受信パイプライン |
| TRN-WP-08 | 非効率 | BLAKE3 を chunk ごとと全体で 2 回計算、期待値は送信者の申告 (`synergos-net/src/transfer/mod.rs:93,182,195`, `:40-41,207-219`, `synergos-net/src/types.rs:133-134`) | 破損や不一致を全量受信後にしか検出できない | 中 | M07 Bao 検証、期待値は署名 Head から |
| TRN-WP-09 | 非効率 | 1 publish ごとに gossip 3 hop と (message, peer) ごとの stream (`synergos-core/src/exchange/mod.rs:1343-1390`, `synergos-core/src/daemon.rs:868-901`, `synergos-net/src/gossip/transport.rs:50-73`) | 初バイトまでの遅延。256 枠のチャネルが offer を落とす | 中 | M09 |
| TRN-WP-10 | 未実装機能 | 転送スケジューラが無い。並行数・帯域の上限なし (`synergos-core/src/exchange/mod.rs:940-956`, `synergos-core/src/daemon.rs:806-814`) | 対話的な取得が背景の大転送に埋もれ、制御 stream がデータと 100 本の枠を奪い合う | 中 | M08 スケジューラ |
| TRN-WP-11 | 設計ギャップ (不整合) | DESIGN §15 の速度試験・サイズ階級・帯域による選択は死コード (`DESIGN.md:1571-1587`, `synergos-net/src/quic/mod.rs:106-130,504-532`) | GUI が効かない設定を動くものとして表示 | 中 | M08 計測。死に設定は移植しない |
| TRN-WP-12 | 設計ギャップ | 全版が全体転送で、版間の再利用 0 % (`docs/versioning-design.md:10`, `synergos-core/src/exchange/mod.rs:895-956`) | 500 MB PSD を 1 日 20 回保存すると受信者ごとに 10 GB/日 | 高 | M27 chunk-store、M28 range plan |
| TRN-WP-13 | 非効率 | 受信・publish のたびに manifest・state・履歴 index を全書換 (`synergos-core/src/project.rs:473-503`, `:425-456`, `synergos-core/src/manifest.rs:162-169`) | 小ファイルの throughput の上限がメタデータ書込で決まる | 中 | M04 |
| TRN-WP-14 | 非効率 | daemon 起動時に全 manifest の全ファイルを CRC (`synergos-core/src/daemon.rs:265`, `synergos-core/src/restore.rs:44-58`, `synergos-core/src/checkout.rs:214-221`) | 大きなプロジェクトで再起動のたびに数分〜1 時間使えない | 中 | M05 |
| TRN-WP-15 | 非効率 | custody は単発 PUT・1800 秒上限 (`synergos-custody/src/handlers.rs:218,235-240`)。PUT 時の SHA-256 (`synergos-custody/src/store.rs:186-197`) と verify の全再読込 (`store.rs:141-168`) | 再開できず、範囲取得もできない | 高 | M14 |
| TRN-WP-16 | 未実装機能 | relay クライアントは未配線で、経路フォールバックは中身なしに成功を返す (`synergos-net/src/conduit/mod.rs:413-426`, `synergos-relay/src/server.rs:85`) | NAT 越しのピアにデータ経路が無い | 高 | M11, M12 |
| TRN-WP-17 | 欠落機構 | 転送の fixture は loopback 上の数 KB だけ (`synergos-core/tests/transfer_e2e.rs:106`) | 大容量・遅い回線・中断・再試行の後退を検出できない | 中 | M31 計測ゲート |
| TRN-WP-18 | 設計ギャップ | gossip の MessageId は内容由来で nonce も TTL も無い (`synergos-net/src/gossip/node.rs:310-317`, `:182-191`, `:29-39`) | 同じ (project, file, version) の再試行が自分のキャッシュで黙って捨てられ、checkout が止まる | 高 | M09 再試行は新しい `req_id`、M33 |
| TRN-WP-19 | 非効率 | FileWant に全保持者が全量で応える (`synergos-core/src/exchange/mod.rs:895-956`, `synergos-core/src/daemon.rs:868-876`, `exchange/mod.rs:764-767`) | 保持者 N で N 部を書いて CRC し、rename して archive する | 高 | M07 |
| TRN-WP-20 | 設計ギャップ | 送信は可変の作業ファイルから読む (`synergos-core/src/exchange/mod.rs:1366-1376`, `:1135-1153`) | 送信中の編集で転送が全損するか、未 publish の内容が公開版として流れる | 中 | M02 不変の object から送る |
| TRN-WP-21 | 設計ギャップ | 移行計画が版なしの wire を凍結し「protobuf」と誤記 (`spec/plan/cpp-migration.md:39-40`)。TXFR に版も chunk 長も無い (`synergos-net/src/transfer/mod.rs:130-142`) | C++ 移植の後にもう 1 回 flag day が要る | 中 | M06, M32 |

#### 2.1.4 重複排除・差分・再開 (DD)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| TRN-DD-01 | 設計ギャップ | 2026-08-16 決定は LAN 帯域を前提にする (`docs/versioning-design.md:11`, `:25`, `:151`) | テクスチャや派生データでは損失 0 に近いが、反復保存の大容量ファイルとチェックポイントで 30〜85 % のバイトが無駄 (推定)。どちらの状況かを測る手段も無い | 高 | M27, M28, M31 (Q1) |
| TRN-DD-02 | 設計ギャップ | FileOffer の `content_hash` は 0 (`synergos-core/src/exchange/mod.rs:569-572`)。取得可否は (project, path, version) (`synergos-core/src/daemon.rs:942`, `:984-986`) | 同じバイトを path・プロジェクト・restore ごとに再転送 | 中 | M02 ローカル内容索引、M03 |
| TRN-DD-03 | 設計ギャップ | TXFR は再開できない (`synergos-net/src/transfer/mod.rs:71-112`, `:25`, `synergos-core/src/exchange/mod.rs:730-737`) | 中断 = 全再送 | 高 | M07 |
| TRN-DD-04 | 設計ギャップ | 完全性と stale / 衝突の判定は全量受信後、期待ハッシュは送信者の header (`synergos-net/src/transfer/mod.rs:208-219`, `synergos-core/src/exchange/mod.rs:740,765-767`) | 早期に拒否できない | 中 | M07 |
| TRN-DD-05 | 設計ギャップ | `ManifestEntry` は `{version, size, crc, updated_at, publisher}` (`synergos-core/src/manifest.rs:35-46`)。同一判定は (size, crc) (`manifest.rs:203`, `synergos-core/src/checkout.rs:214-219`) | 内容アドレスが使えず、全量読込が増える | 高 | M01, M03 |
| TRN-DD-06 | 非効率 | 送信で 3 回、archive で 3 回読み、履歴から応答するたびに再ハッシュ (`synergos-core/src/exchange/mod.rs:649`, `:1136`, `synergos-core/src/history/store.rs:681-699`) | ディスク帯域が網より先に律速 | 中 | M01, M02 |
| TRN-DD-07 | 設計ギャップ | 履歴の重複排除はファイル全体・プロジェクト単位 (`synergos-core/src/history/store.rs:81-87`, `:146`, `:164-177`) | 保管量が全ファイルの全版の総和で増え続ける | 高 | M27, M13 |
| TRN-DD-08 | 設計ギャップ | 履歴 offload が実体を丸ごと RAM へ読む (`synergos-core/src/history/rotation/s3.rs:66`, `:83-87`, `synergos-core/src/history/rotation/gdrive.rs:210`) | 数 GB の素材で OOM かバックエンド上限 | 高 | M13 streaming multipart |
| TRN-DD-09 | 設計ギャップ | custody PUT は 1800 秒、GET は Range を無視 (`synergos-custody/src/handlers.rs:235-240`, `:243-255`, `synergos-custody/src/store.rs:171-207`)。仕様も全体再送 (`spec/feature/opaque-custody.md:40`) | 100 GB は通らない | 高 | M14 |
| TRN-DD-10 | 非効率 | verify が全量を再読込 (`synergos-custody/src/store.rs:141-168`, `synergos-custody/src/handlers.rs:105-115`)、Curare の timeout は 60 秒 (Curare: `curare-server/src/storage/synergos-storage.ts:17`) | 読込に 60 秒超かかる実体は版にならない | 高 | M14 O(1) verify |
| TRN-DD-11 | 設計ギャップ | custody のキーは sha256(opaqueObjectId) (`synergos-custody/src/signature.rs:5-7`)。Curare の blobRef は受付ごとの UUID (Curare: `curare-server/src/routes/upload.ts:30`) | 同じバイトを保存のたびに upload・保管 | 中 | M14 内容由来の保管キーと `alreadyStored`、M15 `present` 応答、Curare M20 |
| TRN-DD-14 | 設計ギャップ | Curare / custody は sha256 だけ (Curare: `curare-server/src/db/types.ts:9`, `curare-server/src/storage/durable-receipt.ts:9`)。C++ custody の digest も SHA-256 だけ (`synergos-custody/native/digest.hpp`) | 境界を越えるたびに別アルゴリズムで再ハッシュし、範囲検証もできない | 中 | M01 |
| TRN-DD-15 | 設計ギャップ | 休眠中の chunker は固定 256 KiB・平坦 DAG (`synergos-net/src/content/chunker.rs:36-41`, `:55-67`, `:22-27`)。DESIGN §15.1 は Rabin と書く | 将来の chunk 経路の形が誤っている (root block 16 MiB で約 56 GiB が上限) | 中 | M27 (DESIGN §15.1 を superseded とする) |
| TRN-DD-16 | 非効率 | ファイルごとの固定費: message × peer ごとの stream (`synergos-core/src/daemon.rs:878-885`)、ファイルごとの記録 (`synergos-core/src/exchange/mod.rs:1101-1110`)、全書換 (`synergos-core/src/project.rs:453-455`) | 10^4〜10^6 ファイルのツリーが実用にならない | 高 | M04, M07 `WANT_MANY`, M09 |
| TRN-DD-17 | 未実装機能 | 形式ごとの保管方針が無い。差分検知は version + size + CRC (`docs/versioning-design.md:40`)、種別を区別しない (`:349`) | deflate 圧縮の FBX や派生 GPU テクスチャが重複排除を無効にする | 中 | M29 |
| TRN-DD-18 | 未実装機能 | auto-pull は `sync_mode` を見ない (`synergos-core/src/daemon.rs:984-986`)。custody GET は範囲なし (`synergos-custody/src/handlers.rs:243-255`) | 疎な取得ができない | 中 | M30, M17 |
| TRN-DD-19 | 設計ギャップ | custody 1800 秒 (`synergos-custody/src/handlers.rs:236`)、Curare client 30 分 (Curare: `curare-server/src/client/curare-client.ts:44`)、Cu→Sy verify 60 秒、S3 単発 PUT 5 GB、QUIC 既定窓 | 100 GB のチェックポイントを運ぶ経路が無い | 高 | M14, M08, Curare M22 |
| TRN-DD-20 | 非効率 | publish ごとに全ピアが RootCatalog を bitswap で取って捨てる (`synergos-core/src/exchange/mod.rs:1395-1408`, `synergos-core/src/catalog_sync.rs:175-178`, `synergos-core/src/daemon.rs:1014-1018`) | 無駄なトラフィックと、publisher 側の RAM が際限なく増える | 低 | M09 (catalog 交換を Head に置換) |
| TRN-DD-21 | 非効率 | `project status` / `publish --all` が同じサイズの全ファイルを毎回 CRC 全読込 (`synergos-core/src/worktree_status.rs:133-135`, `synergos-core/src/cli_worktree.rs:24-26`) | 大きなツリーの status が遅い | 中 | M05 |
| TRN-DD-22 | 設計ギャップ | 移行契約が CRC 同一性・平坦 TXFR・全体再送の custody を凍結 (`spec/plan/cpp-migration.md:39-40`, `:44-47`, `spec/feature/native-contracts.md:24`) | 再設計が移植後の 2 回目の flag day になる | 高 | M32 |
| TRN-DD-23 | 設計ギャップ | 重複排除の範囲を定める契約が無い。コールド保管のキーは平文の内容ハッシュ (`synergos-core/src/history/rotation/mod.rs:97-98`, `synergos-core/src/history/store.rs:660-661`) | 「既にある」応答やチャンク保管が組織をまたぐ存在オラクルになる | 中 | M27 + security SPEC-SY-envelope-encryption、Curare M20 |

Curare だけに属する 14 件 (TRN-E2-01, -03, -04, -06〜-12, -17, TRN-TP-08, TRN-DD-12, -13) は Curare 側: spec/plan/redesign-transfer-efficiency.md §2 に載る。Synergos はそのうち TRN-E2-01 と TRN-E2-10 に SPEC-SY-custody-batch-admission で、TRN-TP-08 に edge mode (M17) で、TRN-DD-13 に SPEC-SY-object-get で応える。

### 2.2 ワークロード別コストモデル

前提: WAN 1 Gbps・RTT 100 ms、LAN 10 GbE、NVMe 2 GB/s、HDD 200 MB/s。wire は受信者あたり。目標値は T0 のベースラインまで推定。両文書で同じ表を使う。

| ワークロード | 現状 (wire / disk / 阻害要因) | 目標の経路 | 目標 (wire / disk) | 機構 |
|---|---|---|---|---|
| PSD 500 MB、1 日 20 回保存、レイヤの 10〜25 % を変更 | 受信者ごとに 10 GB/日。publisher は N 回 upload。受信ディスク約 60 GB/日 | 全体転送プロジェクト: seed 経由で拠点あたり 1 回。チャンクプロジェクト: seed / custody がチャンクで保管し、メンバーは range plan で取得 | 全体: 拠点あたり 10 GB/日を 1 回、ディスク 11 GB。チャンク: 1.5〜3.5 GB/日 | M01, M02, M07, M13, M27, M28 |
| deflate 有効で書き出した FBX | CDC で再利用がほぼ 0 | チャンクプロジェクトでは形式方針 hook が圧縮なしの書き出しを要求 | 準拠後は PSD と同じ | M29 |
| テクスチャ: PNG/TGA/EXR の原本、BCn/ASTC/KTX2 の派生 | 全体転送、派生も版に入る | PNG は全体、EXR/TGA はチャンク対象。派生は derived 区分として (原本 BLAKE3, エンコーダ設定) でキャッシュし版にしない | 派生のバイトは版保管から外れる | M29 |
| Unity / Unreal のツリー、20 万ファイル、10 GB | Synergos はファイルごとに manifest 全体を書換 (メタデータ書込は TB 級、推定)。Curare は 1 ファイル 4 RTT で 40 ms なら約 9 時間 | Head の差分 → seed から `WANT_MANY` で一括取得。SQLite の live store。Curare は 1 万件 × 20 の changeset か collection 版 1 件 | メタデータ O(変更)。Curare ≤22 回 | M04, M07, M09, M18, M19 |
| データセット、1000 万の小ファイル | 登録不能 (500 / 1000 の上限) | 1 GB の WebDataset 型 shard を版の単位にし、collection 版と shard 内の Bao 範囲読み | shard あたりの費用だけ | M19, M07, M21 |
| チェックポイント 10〜100 GB | custody 1800 秒上限、verify の再ハッシュ 50〜500 秒に対し timeout 60 秒、S3 単発 PUT ≤5 GB、stream 約 100 Mbps | 再開可能 part ≥ max(10 MiB, size/10 000)、受領証 1 回、送信元 k≤4 の Bao 分割範囲、チャンク層で系列の重複排除 + BG4-LZ4 | 100 GB を ≤20 分・≥800 Mbps (推定)。チャンクプロジェクトの系列重複排除 25〜54 % (調査: ZipLLM 54.1 %) | M08, M14, M22, M27 |
| 300 席の拠点が 2 GB のレベルを取得 | 1 origin から 300 × WAN か N 回 push | 拠点 seed へ WAN 1 回、その後 seed とピアから LAN で展開 | 拠点あたり WAN 2 GB | M09, M10, M13 |
| CGNAT 越しの在宅アーティスト | 経路なし | hole punch、だめなら home relay。拠点 seed が 1 回だけ拾う。暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。 | relay 経由 1 回 | M11, M12 |
| 外部ベンダーやクラウド GPU エージェントが 50〜100 GB を取得 | 到達不能 (tailnet のみ) か userspace DERP | HQ 外の egress edge、fetch session、範囲指定で再開可能。暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。 | edge ごとに origin から 1 回の充填 | M16, M17, M21 |

メタデータの付随量: 100 GB の Bao outboard は約 400 MB、チャンク表は約 160 万 × 48 B で約 77 MB。500 MB の PSD は 64 KiB 目標で約 7 600 チャンク。BLAKE3 は 1 コアで数 GB/s、FastCDC は約 1.5 GB/s なので、NVMe 上の単一パスはディスクが律速する。

### 2.3 根本原因の整理

1. **同一性が 2 系統で、どちらも範囲を検証できない** (TRN-DD-05, TRN-DD-14)。そのため再開・複数送信元・内容アドレス・重複排除のどれも作れない。
2. **転送がイベント駆動の push** (TRN-TP-01, TRN-TP-02, TRN-WP-18, TRN-WP-19)。通知が失われると更新も失われ、送信元を受信側が選べない。
3. **I/O と記録がファイル単位の全量処理** (TRN-WP-06, TRN-WP-07, TRN-E2-13, TRN-WP-14)。ファイル数と実体サイズの両方に対して費用が線形以上に増える。
4. **トポロジ・役割・計測が宣言されていない** (TRN-TP-17, TRN-TP-13, TRN-WP-11)。control は観測するだけで、データ面の構成に効かない。
5. **C++ 移行契約が現状の欠陥を互換対象として凍結する** (TRN-DD-22, TRN-WP-21, TRN-TP-16)。今契約を版付けしなければ、移植の後にもう 1 回 flag day が要る。

### 2.4 他テーマとの相互参照

- セキュリティ (`spec/plan/redesign-security.md`): SEC-DA-01 と TRN-TP-15 は同じ改修 (NR-1)。SEC-DA-02 の stream 予算 (NR-2) は M08 の窓設定と両立させる (窓は転送側、予算の下限はセキュリティ側)。SEC-AZ-05 (DHT による会員情報の露出) は M10 の DHT1 削除で同時に解消する。チャンク保管の不変条件 SPEC-SY-chunk-access と重複排除範囲 = KEK 範囲 (SPEC-SY-envelope-encryption) はセキュリティ文書が正本。
- AI・ゲーム開発 (`spec/plan/redesign-ai-game-dev.md`): ai-game テーマの検証済み発見 (AGD-*) のうち、本書の機構が解くか範囲を決めるものを次に挙げる。発見の本文と根拠は同文書 §2 にある。

| AGD ID | 発見の要点 | 本書の機構 |
|---|---|---|
| AGD-AI-07 | 大容量 (チェックポイント) の層が無い。単発 PUT、custody 1800 秒、再開・Range なし | M14 (custody 転送 v2)、M08 (窓と送信元) |
| AGD-AI-15, AGD-DM-04 | 同一性が CRC32 だけ、ファイル事象ごとに manifest 全体を書き換える | M01 (BLAKE3 正準)、M03 (manifest v2)、M04 (SQLite live store) |
| AGD-AI-20 | C++ 移行が CRC32 だけで墓標の無い manifest を互換契約として凍結する | M32 (Compatibility contracts の改訂)。墓標そのものは ai-game 文書の SPEC-SY-tombstone |
| AGD-DM-13, AGD-GW-06 | 疎な作業領域 (sparse checkout) のデータモデルが無い | M30 (`sync_mode` で wanted-set を絞る)。選択の正本は Curare、VFS は ai-game 文書 |
| AGD-GW-10 | 離れていたノードが見逃した publish に追い付かない | M09 (HaveSet 照合で再参加時に追い付く) |
| AGD-GW-19 | 小ファイル大量 (OFPA、Unity の meta、データセット) で 1 ファイル 1 stream | M07 `WANT_MANY` |
| AGD-OP-11 | 計測値・トレースが無い | M08 (`NetworkSnapshot`)、M31 (`transfer.bytes`) |
| AGD-DM-06, AGD-GW-15 | rename は全再転送、削除は復活する | rename: M02 の内容索引により、同じ BLAKE3 の実体は wire 0 バイトでリンク / reflink で満たす。削除と墓標: ai-game 文書の AIGAME-M07 と SPEC-SY-tombstone が正本で、本書の manifest v2 は削除を表さない (Q21) |

  VFS 展開・データセットの shard 方針・`project_ref` と Curare space の対応 (Q10) は同文書が持つ。SPEC-SY-format-policy の契約文 (§4.21) は Curare の版種別が `policy_class` を運ぶことの根拠として AGD-AI-01 (データセット集約が無い) を挙げる。
- Curare 側: spec/plan/redesign-transfer-efficiency.md: changeset・collection・object registry・fetch session・配置・API 予算の正本。

## 3. 設計判断と選択肢の比較

4 軸は HARNESS §3.5 に従う。**AI 学習量** (5 = 新規学習が最も少ない)、**作業コスト** (5 = 最も少ない)、**解決度**、**主目的との一致度** を 1〜5 で付け、高いほど良い。現状維持の案は新しく学ぶものが無いので AI 学習量を 5 とする。[変更提案] は既存決定の変更で、判断者は neco。

### 3.1 D-T01 正準の内容同一性 [変更提案: Curare `versioning-types.md:59`、`SPEC-SY-native-metadata`、`versioning-design.md` §0]

既存記述: 「差分の検知は `version` + size + CRC で足りる」(`docs/versioning-design.md:11`)、「Unchanged size/CRC retains its existing version」(`spec/feature/native-contracts.md:24`)。

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 2 系統のまま (Cu SHA-256、Sy CRC32 + 使い捨ての BLAKE3) | 5 | 5 | 1 | 1 |
| B. 全部 SHA-256 | 4 | 3 | 2 (木構造が無く範囲検証不能) | 2 |
| **C. BLAKE3-256 を正準にし、16 KiB グループの Bao outboard を持つ。S3 や v1 受領証が要る所では `digests[{alg, value}]` に SHA-256 も載せ、多重ダイジェストを 1 パスで計算** | 3 | 3 | 5 | 5 |
| D. Xet 型の term ハッシュ | 2 | 2 | 4 | 3 |

**採用: C。** Curare は `content_hash` (SHA-256) の列を残し、repo をまたぐ同一判定は BLAKE3 で行う。CRC32 は旧形式のヒントにだけ残す。Bao の root は BLAKE3 ハッシュと同一なので、root 用の別フィールドは置かない (16 KiB のチャンクグループは outboard の保存の段だけを変え、root の値は変えない)。

### 3.2 D-T02 CDC の再検討 (オーナー決定 3) [変更提案: 2026-08-16 決定の部分変更]

既存記述: 「差分転送は帯域の最適化であり、LAN / Cloudflare Mesh の帯域では必須ではない」(`docs/versioning-design.md:318-319`)。

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 全体転送のまま、計測だけ追加 | 5 | 5 | 1 (反復保存の大容量ファイルで再利用 0 %) | 3 |
| B. 履歴ノードでの保管専用 `zstd --patch-from` | 4 | 4 | 2 (ディスクだけで帯域は減らない) | 4 |
| C. 転送ごとの rolling delta (rsync / zsync) | 3 | 3 | 2 (受信者ごとに送信側 CPU、保管の重複排除なし、custody には効かない) | 2 |
| D. バイト差分 (xdelta / bsdiff) | 3 | 3 | 2 (500 MB の PSD で bsdiff は約 8.5 GB の RAM) | 1 |
| **E. 保管層と WAN 層に内容定義チャンク化 (Xet 定数の FastCDC)。同一性はファイル全体のまま** | 3 | 3 | 4 (PSD / EXR / チェックポイントで 30〜85 % 削減。テクスチャはほぼ 0 で、これは正しい) | 5 |
| F. AI チェックポイント / データセットだけチャンク化し、ゲームメディアは全体のまま (Xet 型の固定分割) | 3 | 3 | 3 (AI 系列は E と同じ削減。PSD・EXR の反復保存で失う 30〜85 % を取り逃す) | 4 |

**採用: E。計測で削減率が 30 % を下回るプロジェクトは B を代替にする。** F は E の部分集合として扱える。E は形式ごとの区分 SPEC-SY-format-policy (`policy_class` = chunk / whole / derived、§4.21) とプロジェクトごとの `project.chunking = off | measure | on` (§4.19) で有効範囲を決めるので、AI のプロジェクトだけを `on` にすれば F と同じ動作になる。媒体の種類で固定に分けるより、形式の区分とプロジェクト単位の計測ゲートで分ける方が良い理由は、削減の大きい所がゲームメディアの側にもあるからである。PSD と EXR の反復保存は変わらないレイヤやタイルが大半で、そこが 30〜85 % の無駄の発生源になる (TRN-DD-01)。一方 PNG や BCn のテクスチャは削減がほぼ 0 なので `whole` に置く。2026-08-16 決定から維持するもの: 同一性 = ファイル全体のハッシュ、git の manifest = ポインタ集合、メンバーはファイル全体を展開する、どの実体もいつでも丸ごと書き出せる (`docs/versioning-design.md` §3.7 の復旧性)。見直すもの: 「CDC をどこにも入れない」と「重複排除はファイル全体だけ」。`docs/versioning-design.md` §3.7 自身が「実測で出たら履歴ノードの内側だけに後付けできる」と書いており、本案はその条件 (計測ゲート、§4.23) と配置 (常時稼働の役割だけ) をそのまま具体化する。

### 3.3 D-T03 チャンク保管の置き場 (オーナー決定 3 の分離判断)

オーナー決定 3 が求めた比較である。

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. **Synergos 本体に入れる** (`synergos-core` の履歴コードと custody にプロセス内で同居。DESIGN §15.1 の経路) | 3 | 3 | 3 (チャンク索引が custody admin とプロセス・資格を共有し、security D-11 の評価は 3。全 daemon が GC と索引の費用を負う) | 2 (転送層が厚くなり、メンバーは全体ファイルという不変条件を壊す) |
| **B. Synergos repo 内の独立チャンク保管モジュール**: ネイティブライブラリ `synergos-chunkstore`。CMake target とドメイン `chunk-store` を独自に持つ。seed / archive / edge はプロセス内でリンクし (API と PEP の境界)、custody はローカル socket 越しの別プロセスとしてだけ使う。`gc` と KEK の unwrap は別の OS ユーザーと Vault policy で動く Excubitor の定期ジョブにする。API は狭い SPEC-SY-chunk-store、セキュリティの PEP は SPEC-SY-chunk-access | 3 | 3 | 5 (プロセス分離の条件付き。§4.19) | 5 |
| C. 別サービスか別 repo として独自に配備 | 2 | 1 (需要を計測する前に 4 つ目の可動部品と新しい配備が増える) | 5 | 3 |
| D. 全経路でクライアント側 CDC (Xet 型のメンバー wire) | 2 | 1 (全クライアントが chunker を持つ) | 5 | 3 |

**採用: B。分離が勝つ。** 4 軸の理由:

- 解決度: security D-11 の不変条件 (SPEC-SY-chunk-access) を 1 つの狭い境界で満たせる。ただしプロセス内リンクだけでは API と PEP の境界にしかならず、同じプロセスと OS 資格を共有する点は A と同じである。そこで B の 5 点は次の 2 条件を前提にする: (1) custody はモジュールを別プロセス (ローカル socket) としてだけ使い、custody admin の資格とチャンク索引を同じプロセスに置かない、(2) `gc` と KEK の unwrap は別の OS ユーザーと Vault policy を持つ Excubitor ジョブで動く (不変条件「GC は別の主体で動く」)。この 2 条件を外すと B の解決度は A と同じ 3 になる。A はモジュール境界が無いので、条件を足しても API の狭さを強制できない。
- 主目的との一致度: チャンクの複雑さを、バイトと WAN 回線を持つ常時稼働の役割に閉じ込め、メンバーには手を入れない。
- 作業コスト: C++ 移植でいずれ書くコードなので A と同等。需要を測るまで新しい配備物が無いので C より安い。
- AI 学習量: 同等。FastCDC・BLAKE3・Bao には参照実装があり、定数は test vector で固定する。

境界と契約は SPEC-SY-chunk-store (API、§4.19) と SPEC-SY-chunk-access (セキュリティ不変条件。本判断で無条件になる、追補 X5、security §4.0.2 で受入済み)。後の計測でチャンク層を独立に拡張する必要が出た場合は、C を文書化済みの次の経路とする。

### 3.4 D-T04 メンバーがチャンク層から恩恵を受ける方法

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 恩恵なし (メンバーの wire は全体ファイル) | 5 | 5 | 2 | 3 |
| B. `chunk_have` の wire 拡張。メンバーがローカルでチャンク化 | 3 | 2 | 4 | 3 (全メンバーが chunker を持つ) |
| **C. seed が range plan を計算する。両版のチャンク表を持つ seed が `{copy(old_off → new_off, len), fetch(new_off, len)}` を返し、メンバーはローカルでコピーし、残りを TXF2 で取得し、全体の Bao で検証** | 4 | 3 | 4 | 5 |

**採用: C** (SPEC-SY-range-plan)。メンバーは chunker もチャンク索引も持たず、同一性の検証はファイル全体のまま。B は両端がモジュールを持つ seed↔seed と seed↔custody の回線でだけ許す。メンバーの wire は変更範囲だけを運ぶようになり、「通常ノードは全体ファイルを受け取る」を部分的に見直すので、neco の再判断事項とする (Q1)。

### 3.5 D-T05 転送の方向 (契約変更)

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. want を全体送信し保持者が push (現状) | 5 | 5 | 1 | 2 |
| B. want を 1 つの送信元に宛てる | 5 | 5 | 2 | 3 |
| **C. 受信側主導の範囲 pull。Bao 検証、再開可能、送信元スコア、k≤4 の分割と endgame (SPEC-SY-txf2)** | 3 | 3 | 5 | 5 |

**採用: C。** B は C の部分集合で、単独では再開も範囲も解かない。

### 3.6 D-T06 伝播

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 1 hop のイベントを補修 (受信後に再 offer) | 4 | 4 | 2 | 2 |
| B. gossip を直す (転送、graft、nonce) が push は残す | 3 | 3 | 3 (通知が引き続き必須で、送出は N 部) | 2 |
| C. 全変更を control から push | 4 | 3 | 3 (control がデータ経路の単一障害点) | 2 |
| **D. 署名 Head + HaveSet の反エントロピーをピアごと 1 本の長寿命 stream で行い、wanted-set を永続化して hash で pull (SPEC-SY-replication-stream)** | 3 | 3 | 5 | 5 |

**採用: D。**

### 3.7 D-T07 目標トポロジ (傘、TRN-TP-17)

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. ハブだけ (HQ の custody と Curare の URL) | 5 | 4 | 1 | 2 |
| B. overlay だけ (Tailscale / Cloudflare Mesh をデータ面に) | 4 | 4 | 2 (折返し、展開なし) | 2 |
| C. CDN 中心 (Cloudflare 経由の署名 URL) | 4 | 3 | 3 (本文サイズ上限、P2P と LAN なし) | 2 |
| D. フレームワーク採用 (iroh / libp2p) | 2 | 3 | 4 | 2 (Rust に縛られ C++ 移植と衝突) |
| **E. directory (Cernere 署名の node card と ProjectRoster を control が配る) で協調する拠点 seed + 受信側主導の swarm + DERP 型 relay (C++ 版だけ) + 拠点 / egress edge** | 3 | 2 | 5 | 5 |

**採用: E。** M09〜M17 がこれを構成する。どのノードがどのプロジェクトの会員で、どの役割と宣言到達性を持つかは Cernere が決めて署名し (オーナー決定 8)、control はそれを配るだけで構成を決めない。relay は C++ 版でだけ配備し、それまでの NAT 越えは Tailscale が担う (オーナー決定 7)。

### 3.8 D-T08 発見

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. Kademlia を署名レコード付きで動かす | 2 | 2 | 3 | 2 (会員情報の露出、SEC-AZ-05) |
| **B. control が配るプロジェクト単位の directory (Cernere 署名の ProjectRoster と node card、node 署名の Presence を束ねる。control は署名しない。キャッシュ ≤24 時間) と同一拠点の mDNS フォールバック。C++ 移植では DHT1 を削除 (SPEC-SY-directory)** | 4 | 3 | 5 | 5 |
| C. overlay (Tailscale) を発見に使う | 5 | 4 | 2 (データ面が 1 つの VPN に結び付く) | 2 |

**採用: B。** security Q4 (公開 DHT を持たない) と一致する (security D-23)。発見は Cernere 署名の card を control が配る directory で行い、各要素は単独で検証できるので、control は選ぶ・並べる・省くことはできても、ノード・会員・endpoint を足せない (オーナー決定 8)。

### 3.9 D-T09 中継と NAT 越え

- **着地 (オーナー決定 7、2026-10-23 夕)。** 外向きのセキュアな中継 (DERP 型 relay: 両端が 443 へ outbound で接続し、relay は end-to-end 暗号化された QUIC を中継するだけ) は C++ 版でだけ作る。SPEC-SY-relay-auth と SPEC-SY-relay-v2 は C++ 移行の契約で、現行 Rust には何も足さない (`synergos-relay` の Rust 実装は改修も配備もしない)。C++ relay が受入ゲートを通るまで、拠点と外部環境 (Codex クラウドなど) は Tailscale でつなぎ、`synergos-node` は tailnet IP にだけ bind する (Synergos `docs/site-operations.md`)。

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. Cloudflare Tunnel をデータ経路に | 5 | 4 | 1 (QUIC の着信なし、本文 100 MB 上限) | 1 |
| B. 標準 TURN (RFC 8656) | 3 | 3 | 3 (企業網の TCP/443 に弱い) | 3 |
| C. Tailscale DERP だけ | 5 | 5 | 2 (低 throughput の部類、第三者依存) | 2 |
| **D. DERP 型の relay mode: UDP 443 の QUIC datagram、TCP 443 の WSS をフォールバック、フレームは `dst_peer_id` 宛て、内容を見ず中で end-to-end の mTLS、拠点 relay を優先 (SPEC-SY-relay-v2)** | 3 | 2 | 5 | 5 |

**採用: D。** TURN 互換の mode は後から `relay_v1` の capability bit の下に足せる。

### 3.10 D-T10 役割のまとめ方

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 役割ごとに別バイナリ | 3 | 2 | 4 | 3 |
| **B. 1 バイナリ。役割はプロセスごとの mode。禁止された同居 (custody-admin か control-admin とインターネット向け mode) は起動時に失敗** | 4 | 4 | 5 | 5 |
| C. 1 プロセスで全役割 | 5 | 5 | 3 | 1 (SPEC-SY-custody-topology に反する) |

**採用: B。** security D-07 (同じバイナリでも配備単位・OS ユーザー・Vault policy を分ける) と両立する。

### 3.11 D-T11 upload の方式 (custody と MinIO)

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 単発 PUT (現状) | 5 | 5 | 1 | 2 |
| B. tus 1.0 | 4 | 3 | 4 | 3 |
| **C. custody に IETF `draft-ietf-httpbis-resumable-upload`、MinIO に S3 multipart (part ごとの `x-amz-checksum-sha256`)。Curare の 1 つの storage port の裏に置く** | 3 | 3 | 5 | 5 |

**採用: C。** IETF 版は tus と互換の考え方で、HTTP の標準化経路に乗る。

### 3.12 D-T12 受領証の検証

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. verify のたびに再ハッシュ (現状、upload 1 回で 2 回) | 5 | 5 | 1 | 1 |
| B. 一括 HEAD の endpoint。検査は都度のまま | 4 | 4 | 2 | 2 |
| **C. PUT 中にダイジェストを計算し fsync の証跡と保存。署名受領証 (SPEC-SY-custody-receipt-v2) を Curare がそのまま保存し、署名でローカル検証。全量再読込は非同期の scrub だけ** | 4 | 3 | 5 | 5 |

**採用: C。** 署名アルゴリズムは security D-06 が持つ。本書は「1 回生成し、再計算しない」を加える。

### 3.13 D-T15 Synergos の live メタデータ保管

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 文書全体の JSON (現状) | 5 | 5 | 1 | 3 |
| B. 追記ジャーナル + compaction | 3 | 3 | 4 | 4 |
| **C. プロジェクトごとの SQLite WAL (C++ ネイティブ。redb は Rust 専用)。git が pointer 集合を持ち続けるよう、snapshot 時点で `manifest.json` format 2 を書き出す** | 4 | 3 | 5 | 5 |

**採用: C。**

### 3.14 D-T18 圧縮方針

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 形式の許可表 | 4 | 4 | 2 (壊れやすい) | 3 |
| **B. チャンク保管の中でチャンクごとに試行 (LZ4 の試し圧縮 → none / LZ4 / zstd-3 / bf16・fp16 向け BG4-LZ4)。原本 / 派生の区分は publish 前・提出前の hook** | 3 | 3 | 5 | 5 |

**採用: B。** QUIC 上の wire 圧縮は原則オフ (データは既に圧縮済みか圧縮不能)。例外は 1 Gbps 未満の回線での `WANT_MANY` 小ファイル一括に zstd-1 (`transfer.wire_compress = auto`)。

### 3.15 D-T19 Rust の範囲 (オーナー決定 6 の転送への適用)

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. 先に Rust で直してから移植 | 3 | 1 (切替が 2 回、parity 試験が二重。TRN-DD-22) | 4 | 1 |
| B. Rust を全面凍結 | 5 | 5 | 2 | 3 |
| **C. Rust には試験専用の計測ハーネス (C++ parity ゲートのベースライン) と設定上の運用緩和だけ。P0 級のデータ損失 2 件 (TRN-WP-05, TRN-WP-18) と偽の `Ok(Relay)` 状態は neco に挙げ、回答があるまでコード修正しない** | 5 | 4 | 3 | 5 |

**採用: C。**

### 3.16 D-T20 輻輳制御の既定

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| **A. 既定は Cubic、BBR はフラグの裏。計測後に経路の種類ごとに選ぶ** | 4 | 4 | 4 | 5 |
| B. 既定 BBR | 3 | 4 | 4 (損失に強い) | 3 (quinn は BBR を実験扱い、実運用での panic 報告あり) |

**採用: A。** C++ のスタックは接続ごとに制御器を差し替えられる形にし、計測ハーネスから切り替える。Cubic で TG-02 を満たさなければ、`wan` 系の経路で BBR を既定にする設定変更を行う (Q9)。

### 3.17 D-T21 LAN と DC 経路の大量転送

| 案 | AI 学習量 | 作業コスト | 解決度 | 主目的との一致度 |
|---|---|---|---|---|
| A. どこでも QUIC | 4 | 4 | 3 (GSO/GRO が無いとユーザー空間 QUIC は 1 コア 500 Mbps 超で TCP に負ける) | 4 |
| **B. WAN・NAT・P2P は GSO/GRO 付きの QUIC。LAN 10 GbE と DC の大量転送はカーネル TCP 上の custody HTTP も許す** | 4 | 4 | 5 | 5 |

**採用: B。**

### 3.18 D-T22 quick-xml 0.30 (オーナー決定 5。正本は security D-13)

転送からの寄与は 1 つの事実である。SPEC-SY-ipc-v2 の `Subscribe` ができれば GUI のポーリングは不要になり (TRN-E2-18、`synergos-gui/src/app.rs:134`)、転送機能のどれも `synergos-gui` → eframe 0.31 (`synergos-gui/Cargo.toml:16`) → accesskit → atspi → zbus_xml → quick-xml 0.30.0 (`Cargo.lock:9374-9379`) の経路に依存しない。よって本書は security D-13 / SECURITY-M30 を支持する: P0 で `synergos-gui` を `desktop/` workspace へ移し、eframe の `accesskit` feature を外して quick-xml 0.30 の経路を消す (除外扱いにはしない)。egui 本体の退役は C++ desktop-gui が受入ゲートを通った後である。`cpp-migration.md:48`「Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces.」の受入面を C++ desktop-gui に改める変更提案は security 文書側 (同文書 §3 の変更提案一覧) が持ち、本書はそれに従う。

### 3.19 Curare が正本の判断 (要約。全文は Curare 側: spec/plan/redesign-transfer-efficiency.md §3)

| 判断 | 採用案 | Synergos への影響 |
|---|---|---|
| D-T13 保存の粒度 | 2 段の changeset: 一括受付 (≤10 000 件) → データ → 1 回の原子的 commit | custody は changeset 1 件につき SPEC-SY-custody-batch-admission を 1 回受ける (M15) |
| D-T14 clone の認可 | manifest ハッシュに束縛した fetch session。≤1000 件のページごとに set-scoped の custody-capability LCT 1 本 | custody / edge / seed は 1 本の capability で集合内の全実体の全範囲を許す (SPEC-SY-object-get) |
| D-T16 配置と egress [変更提案: Curare INV-1] | Curare 所有の配置 (origin / replica / edge / egress)、release 確定前に rf≥2、HQ 外の egress edge | Synergos は SPEC-SY-custody-replication と edge mode (M17) で実行する。egress edge は Q3 |
| D-T17 遅延展開 | サーバが知る疎な選択 + hash 指定の範囲 API を今、VFS は同じ API の上に後で (ai-game 文書) | Synergos は `sync_mode` を wanted-set の絞り込みで尊重し (M30)、範囲 API (SPEC-SY-object-get) を提供 |

### 3.20 既存決定への変更提案の一覧 (neco の再判断事項)

| # | 既存記述 | 変更提案 | 判断 |
|---|---|---|---|
| 1 | `docs/versioning-design.md:11`「差分の検知は `version` + size + CRC で足りる」、`spec/feature/native-contracts.md:24`「Unchanged size/CRC retains its existing version」 | 同一 = BLAKE3 一致。size + CRC32 は format 1 の項目のフォールバック | D-T01 (Q2) |
| 2 | `docs/versioning-design.md:11` / `:151` / `:318-319`「CDC チャンク重複排除案は … 取り下げ」「版の実体は チャンク化しない」 | 常時稼働の役割に限り独立モジュールでチャンク保管。メンバーは range plan | D-T02 / D-T03 / D-T04 (Q1) |
| 3 | `spec/plan/cpp-migration.md:39-40`「preserve protobuf schemas, QUIC ALPN … CRC/hash validation」 | MessagePack と明記し、`synergos/2` の SPEC-SY-peer-wire / -txf2 / -replication-stream / -content-identity に置換 | M32 |
| 4 | `spec/feature/opaque-custody.md:40`「バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送」 | IETF 再開可能 upload、Range GET、受領証 1 回生成 | D-T11 / D-T12 (M14) |
| 5 | Curare `codex-cloud-access.md:34` (INV-1)「本社はインターネットへ何も公開しない」 | HQ 外の egress edge を例外として置く (HQ には何も公開しないまま) | D-T16 (Q3、Curare 正本) |
| 6 | `spec/feature/opaque-custody.md:28` (SPEC-SY-custody-durability)「receipt発行時にも実ファイルを再検証しsync_all成功を要求する」 | 受領証は commit の時に 1 回だけ発行する。根拠は PUT 中に計算したダイジェストと、fdatasync およびディレクトリ fsync の成功。実ファイルの全量再読込は非同期の scrub へ移す | D-T12 (M14)、判断者 neco |
| 7 | `synergos-custody/src/signature.rs:5-7` (`object_key(id)` = `sha256(opaqueObjectId)`)、`spec/feature/opaque-custody.md:3-5`「Synergosはopaque objectの転送と保管の実行を担当し、制作履歴や承認を作らない」 | custody は内容由来のキー `HMAC(project_key, blake3)` で保管し、同じ内容は `alreadyStored` で 1 回だけ置く。custody は内容の同一性を知るが、認可と版の正本は Curare のままとする (2026-09-08 決定の境界は維持し、「不透明」の意味を「意味を解釈しない」に絞る) | M14、判断者 neco |
| 8 | `docs/versioning-design.md:191`「観測最大版は node-local の `.synergos/state.json` に永続化し」、同 `:345`「単調増加発番用の観測高水位だけは node-local の `.synergos/state.json` に持つ」 | 観測高水位の置き場を `.synergos/state.db` に移す。単調性と node-local の意味は変えないので `cpp-migration.md:41` の「local high-water marks」は保たれる。flag day 後の巻き戻し期間は `state.json` へも書き出す (§5.3) | M04、判断者 neco |
| 9 | `docs/versioning-design.md:346`「`.synergos/history/` `.synergos/incoming/` `.synergos/state.json` は **ignore**」、同 `:71-79` の `.gitignore` 例 | ignore 対象に `.synergos/objects/`、`.synergos/index.db*`、`.synergos/state.db*` を加える (`.synergos/incoming/` は既存) | M04 / M05、判断者 neco |

`docs/versioning-design.md:156-157` の規則「ハードリンクは使わない (publisher の作業ツリーは人が in-place 編集するので、リンクだと保管した旧版が後から書き換わる)」は**変更しない**。本書の object store はこの規則を守る (§4.2)。

## 4. 再設計の内容

### 4.0 読み方

- 各節の順序: 区分 → 目的 → 契約の定義 (ある場合) → 仕組み → 不変条件 → 失敗時の振る舞い → 変更対象 → 担保手段。
- 節の並び: §4.1〜§4.23 は機構番号の小さい順に本体の機構 (M01〜M17、M26〜M31) を置く。M33 (Rust の暫定措置) は機構番号では最後だが、Rust に何を入れないかを先に示すため §4.24 に置く。M32 (`cpp-migration.md` の改訂) は他の全機構の契約を集める節なので、契約の一覧の後の §4.31 に置く。M18〜M25 は Curare 側文書にある。
- **区分** (`spec/plan/redesign-analysis/HANDOFF.md` §6): 「互換契約の変更」は `cpp-migration.md` Compatibility contracts の改訂と `native-contracts.md` の新 `[id:]` 節を伴う。「ネイティブ実装要件」は C++ が最初から避ける実装の非効率で、契約は変えない。「運用」は設定・配備・計測。
- **契約の種別** (区分の括弧内と §4.32 の「種別」列): **W** = wire または保存形式の互換契約の変更。`cpp-migration.md` Compatibility contracts を改め、`native-contracts.md` に `[id:]` 節を足す。golden vector を必須とし、組織ごとの flag day を伴う。**A** = 追加的な新しい契約 (API・設定・計測)。旧版と共存できる。**X** = 外部依存 (Cernere の SPEC-CR-*)。Curare 側文書も同じ記号を使う。
- **担保手段**: 記憶 (文書)、フック、ローカル検証ゲート (Revisor のマージ前ケース)、ランタイム強制。
- **変更対象**: C++ の置き場は `cpp-migration.md:26-27` に従い既存モジュールの `native/` 配下。現在 `native/` があるのは `synergos-core`・`synergos-custody`・`synergos-ipc` で、`synergos-net/native/`・`synergos-relay/native/`・`synergos-control/native/` は新設になる。チャンク保管は新ディレクトリ `synergos-chunkstore/`。Rust 側は「置き換え対象」として名前を挙げるだけで、変更しない。
- 契約の定義は Curare 側文書と同じ文面とし、本書では各契約を 1 回だけ引用する。両文書で共通の契約 ID と文面の一覧を「契約の登録簿」と呼び、その文面の変更は neco が確定する。

### 4.1 TRANSFER-M01 正準の内容同一性と多重ダイジェストの単一パス

- 区分: 互換契約の変更 (SPEC-SY-content-identity、W)。対象発見: TRN-DD-05, TRN-DD-14, TRN-DD-06, TRN-WP-06, TRN-WP-08, TRN-E2-17 (Curare)。フェーズ: T0 仕様と vector、T2 C++。
- 目的: 1 回の読込で、P2P・custody・Curare のすべてが使う同一性と範囲検証の構造を作る。

> **SPEC-SY-content-identity** (Sy (spec + vectors) → Sy all, Cu): The canonical content identity is BLAKE3-256 of the whole object. The verified-range structure is a Bao outboard tree at 16 KiB chunk groups (≈ 0.4 % of size); its root equals the BLAKE3 hash, so no separate root field exists. A digest record is `{size, digests[{alg: "blake3" | "sha256" | "crc32", value}]}` produced by **one streaming multi-digest pass** (multithreaded at ≥ 128 MiB) at publish, upload and receive. SHA-256 is computed when the destination is custody, S3 or a v1 receipt; CRC32 only for legacy readers. Equality is BLAKE3 only; size + CRC32 is a fallback for format-1 entries. Golden vectors live in Synergos `spec/vectors/content-identity/` and are imported by a Curare Revisor case. Replaces the cpp-migration line "CRC/hash validation".

- 仕組み:
  - C++ の `MultiDigest` (ストリーム型) が BLAKE3 tree・SHA-256・CRC32 を同じバッファから計算し、Bao outboard を同時に書く。128 MiB 以上は BLAKE3 の多スレッド版を使う。
  - 出力 `DigestRecord{size, digests[]}` と `.bao` ファイル。`digests[].alg` の綴りは security 文書の契約 (SPEC-SY-custody-receipt-v2) に合わせる。
  - 記録は Head・TXF2 の `WANT`・custody 受付・受領証へそのまま載り、どこでも再計算しない。
  - golden vector: `spec/vectors/content-identity/` (0 B、1 B、16 KiB 境界 ±1、1 MiB、128 MiB 境界の擬似乱数列で、期待値は BLAKE3 公式実装で生成)。Curare の Revisor ケースが同じ vector を読み込む。
- 不変条件: 同一判定は BLAKE3 だけ。期待値は署名済みの Head か manifest の項目から取り、送信者の header からは取らない。1 回の publish / upload / 受信で実体を読むのは 1 回。
- 失敗時: vector 不一致はマージ前に赤。実行時に Bao の不一致が出たら、その 16 KiB グループで中止 (M07)。
- 変更対象: `synergos-core/native/` (新 `digest/multi_digest.{hpp,cpp}`)、`synergos-custody/native/digest.hpp` (SHA-256 だけの現行を `MultiDigest` の利用に置換)。Rust 側で置き換わるもの: `synergos-net/src/transfer/mod.rs` の `hash_file`、`synergos-core/src/manifest.rs` の CRC、`synergos-custody/src/store.rs` の SHA-256。
- 担保手段: ローカル検証ゲート (vector 自己試験、両 repo の Revisor ケース) + ランタイム強制 (受信パイプライン内の検証)。

### 4.2 TRANSFER-M02 不変のローカル object store と単一書込の受信パイプライン

- 区分: ネイティブ実装要件 (契約は変えない。`docs/versioning-design.md:156-157` のハードリンク禁止を守る)。対象発見: TRN-WP-06, TRN-WP-07, TRN-WP-20, TRN-DD-06, TRN-DD-02。フェーズ T2。
- 目的: TG-04a / TG-04b を満たす (reflink 可能な FS で publish 読込 ≤1.05×・受信書込 ≤1.1×、reflink の無い FS で ≤2.05×)。送信中の編集による全損と未 publish 内容の流出を無くし、保管した object を作業ツリーの編集から守る。
- 既存規則の扱い: `docs/versioning-design.md:156-157`「ハードリンクは使わない (publisher の作業ツリーは人が in-place 編集するので、リンクだと保管した旧版が後から書き換わる)」を維持する。Photoshop や DCC ツールの上書き保存はリンク先へそのまま書き込むので、利用者が編集しうるファイルと object の間にハードリンクを張ると `.synergos/objects/<b3>` と、それを指す全ての版が黙って壊れる。
- 仕組み:
  - 配置: `.synergos/objects/<b3[0..2]>/<b3>` と `<b3>.bao`。
  - publish (作業ファイル → object): (1) reflink (Linux `FICLONE`、macOS `clonefile`、Windows ReFS の block clone) で不変 snapshot を作り、snapshot を 1 回読んでハッシュする。(2) reflink が使えなければ、作業ファイルを 1 回読み、ハッシュしながら object へコピーする。読込の途中で作業ファイルの size か mtime が変われば、途中の object を捨てて publish をやり直す。どちらの場合もハードリンクは使わない。
  - 送信は object からだけ行い、作業ファイルからは送らない (TRN-WP-20)。
  - 受信 (object → 作業ツリー): パイプライン内で Bao 検証 → `.part` を fdatasync → store へ rename → 作業ツリーへ reflink、使えなければコピー (この 2 回目の書込が TG-04b の 2.05×)。置き換えられた旧ファイルは SPEC-SY-displaced-preimage に従い rename で退避し、コピーしない (security E6)。
  - ハードリンクを許すのは、Synergos だけが書き利用者が編集しないファイルどうしに限る: object store と履歴 archive、seed / archive / custody の保管領域、§5.2 の旧履歴 root の取り込み、§5.3 の巻き戻し用の書き出し。作業ツリーのファイルには張らない。
  - 履歴の archive は object への参照 (`state.db` の `history` 行) で、コピーではない。
  - offer の BLAKE3 が手元の object にあれば、転送せずに作業ツリーへ reflink かコピーで満たし、wire は 0 バイト (TRN-DD-02)。rename / move された実体も同じ BLAKE3 なので同じ経路で 0 バイトになる (AGD-DM-06)。
  - 比べて採らなかった案: reflink の無い NTFS で object store を持たず、publish 時の検証済み snapshot から配信する案。snapshot そのものが 1 回のコピーなので I/O は減らず、受信側で旧版を守る仕組みが無くなる。
- 不変条件: object は書込後に変更されない (利用者が触るパスと inode を共有しない)。作業ツリーへの反映は検証済み object からだけ。公開されるのは読み切った時点の内容だけ。
- 失敗時: reflink が使えない FS では「ハッシュしながらコピー」になり、計測値に `copy_fallback` を記録する。ハードリンクへは落ちない。publish 中に作業ファイルが変われば publish を中止して再試行する。rename に失敗したら `.part` を残して再開できる状態に保つ。scrub で object の不一致が出たら隔離し、他の保持者から取り直す。
- 変更対象: `synergos-core/native/` (新 `objects/`、`receive_pipeline/`)。置き換え対象: `synergos-core/src/exchange/mod.rs` (受信と archive の連鎖、`:740`, `:795`, `:821-843`)、`synergos-core/src/history/store.rs:681-699`。
- 担保手段: ローカル検証ゲート (ディスク計数 §6.2 試験 13、in-place 編集試験 18、送信中編集試験 19、手元ハッシュ試験 20) + ランタイム強制 + 記憶 (ハードリンク禁止を `native-contracts.md` のネイティブ実装要件にも書く)。

### 4.3 TRANSFER-M03 manifest format 2

- 区分: 互換契約の変更 (SPEC-SY-manifest-v2、W。`SPEC-SY-native-metadata` を改訂)。対象発見: TRN-DD-05, TRN-DD-02, TRN-DD-22。フェーズ: T0 仕様、T2。
- 目的: 内容の同一性 (BLAKE3) を git の成果物である manifest に残し、format 1 の読み手を壊さずに size + CRC32 だけの同一性 (TRN-DD-05) をやめる。大きなツリーの差分を O(変わった部分木) にする (TG-13)。

> **SPEC-SY-manifest-v2** (Sy daemon → git, Sy peers, Cu (collection registration)): `manifest.json` format 2 = format 1 + per-entry `blake3` (hex) and optional `chunk_map_id`, `digests[]`. Entries are sorted and the file is deterministic. Trees over 50 000 entries also carry `manifest_root`: a BLAKE3 Merkle root over content-addressed ≈ 64 KiB sorted entry chunks `{path, size, blake3, mode}`, so two manifests diff in O(changed subtrees); the same entry-chunk encoding is used by SPEC-CU-collection-version. Readers accept formats 1 and 2 forever; writers switch to 2 at the `synergos/2` cut-over of the org. "Unchanged" means equal BLAKE3. Amends `SPEC-SY-native-metadata` ("Unchanged size/CRC retains its existing version").

- 仕組み: `ManifestEntry` に `blake3` を足し、`crc` はヒントとして残す。書き手は format 2 の項目でも `crc` を埋め続ける (§5.3 の降格に使う)。50 000 項目を超えるツリーは `manifest_root` と entry chunk を持つ。
- 不変条件: 読み手は format 1 を永久に受け付ける。書き手が format 2 に変わるのは、その組織の `synergos/2` 切替と同時。
- 失敗時: **Rust の読み手は format 2 を拒否する** (`synergos-core/src/manifest.rs:138-141` で確認。`m.format != 1` を `InvalidData`「unsupported manifest format」で返す)。したがって組織内に Rust ノードが残っている間に format 2 を書いてはならない。組織単位の flag day (§5.2) がこれを保証する。
- 変更対象: `synergos-core/native/` (既存 manifest codec の拡張)、`spec/feature/native-contracts.md` (`SPEC-SY-native-metadata` の改訂)。置き換え対象: `synergos-core/src/manifest.rs:35-46`。
- 担保手段: ローカル検証ゲート (format 1 / 2 の往復 vector、決定的な直列化) + 記憶 (flag day 手順)。

### 4.4 TRANSFER-M04 SQLite の live メタデータ保管

- 区分: ネイティブ実装要件 (保管の形式) + 互換契約の変更 (観測高水位の置き場と ignore 対象。§3.20 変更提案 8・9)。`cpp-migration.md:41` の「local high-water marks」は、`state.db` が同じ単調・node-local の意味を保つので維持する。対象発見: TRN-E2-13, TRN-WP-13, TRN-DD-16。フェーズ T2。
- 目的: 受信バッチあたりの書込を O(変更行数) にする (TG-13)。
- 仕組み:
  - `.synergos/state.db` (WAL) に `entries(rel PK, version, size, blake3, crc32, publisher, updated_at)`、`high_water(rel PK, max_version)` (観測高水位。現 `.synergos/state.json` の置き換え)、`history(rel, version, blake3, size, stored_at, offloaded)`、`wanted(project_ref, blake3, size, prio)` (M09 の wanted-set)。
  - 受信の commit は 200 行か 500 ms でまとめる。hook は stream の経路から外す。`manifest.json` は publish バッチの終わり・`snapshot`・`checkout` の時だけ書き出す。
  - 初回起動時に `high_water` を max(`state.json` の値, manifest の版) から作る。元の `state.json` は消さない。巻き戻し期間 (§5.3) の間は snapshot のたびに `state.json` へも書き出す。
  - ignore: daemon は `.synergos/.gitignore` を管理し、`objects/`、`index.db*`、`state.db*`、`incoming/`、`history/`、`state.json` を書く (`manifest.json` は含めない)。同じ一覧を `docs/versioning-design.md:71-79` の例と `:346` に反映する (§3.20 変更提案 9)。
- 不変条件: `manifest.json` が git の成果物であり続ける。`state.db` と `index.db` と `objects/` はコミットされない。`high_water` は path ごとに単調で、移行や再構築で下がらない。`state.db` は manifest と objects から再構築できる。
- 失敗時: `state.db` が壊れたら起動時に検出し、manifest・objects・残っている `state.json` から作り直す (高水位は 3 つの最大値)。作り直し中は配信しない (M05 と同じ規則)。`.synergos/.gitignore` が消されていれば起動時に書き直し、警告を出す。
- 変更対象: `synergos-core/native/` (新 `state_store/`)、`docs/versioning-design.md` (§3.20 変更提案 8・9)。置き換え対象: `synergos-core/src/project.rs:425-503`、`synergos-core/src/manifest.rs:162-169`、`synergos-core/src/history/` の `index.json`。
- 担保手段: ローカル検証ゲート (10^5 ファイル checkout の書込量計測、受信後の `git status` に新しいローカル成果物が出ない試験 §6.2 試験 28) + ランタイム強制。

### 4.5 TRANSFER-M05 検証済み状態のローカル索引と遅延起動

- 区分: 互換契約の変更 (SPEC-SY-local-index、A。IPC から見える部分) + ネイティブ実装要件 (索引の内部)。対象発見: TRN-E2-15, TRN-WP-14, TRN-DD-21。フェーズ T2。
- 目的: daemon の準備完了を 20 万ファイルで ≤5 秒にし、`status` が変わらないファイルを読まないようにする (TG-12。TRN-E2-15, TRN-WP-14, TRN-DD-21)。

> **SPEC-SY-local-index** (Sy daemon → Sy CLI/GUI, Cu node client (via IPC)): `.synergos/index.db` (SQLite, never committed): `files(rel PK, dev, ino_or_fileid, size, mtime_ns, ctime_ns, blake3, sha256?, crc32, verified_at)`. A row is trusted when the stat tuple matches and `mtime < verified_at − 2 s` (git racy-clean rule). Daemon start registers entries and serves immediately; full verification runs lazily at a rate limit (`verify.sweep_mbps`, default 200). A file is never *served* to a peer until its row is verified. The index is rebuildable from the manifest and objects and is never trusted across devices. The Curare client reads it through SPEC-SY-ipc-v2 `Status` to derive `have[]`.

- 仕組み: 起動の準備完了と検証を分ける。`status` と `publish --all` は変更のないファイルを stat だけで判定する。Curare の node client は clone の再ハッシュをやめ、IPC の `Status` から `have[]` を得る。
- 不変条件: 未検証の行のファイルは peer に配信しない。デバイスをまたいで索引を信用しない (`dev` が変われば全行を未検証にする)。
- 失敗時: racy 規則に引っかかる行は再ハッシュする。索引が壊れたら作り直す。
- 変更対象: `synergos-core/native/` (新 `local_index/`)。置き換え対象: `synergos-core/src/daemon.rs:265`、`synergos-core/src/restore.rs:44-58`、`synergos-core/src/checkout.rs:214-221`、`synergos-core/src/worktree_status.rs:133-135`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (TG-12: 20 万ファイルで準備完了 ≤5 秒、`status` の読込 0)。

### 4.6 TRANSFER-M06 peer wire v2 (ALPN の機能 bit と stream の版)

- 区分: 互換契約の変更 (SPEC-SY-peer-wire、W)。対象発見: TRN-TP-16, TRN-WP-21, TRN-DD-22。フェーズ: T0 仕様、T2。
- 目的: wire に版と機能 bit を入れ、TXF2・Head・relay・range plan を flag day 1 回で導入し、移植後の 2 回目の flag day を無くす (TRN-TP-16, TRN-WP-21, TRN-DD-22)。

> **SPEC-SY-peer-wire** (Sy daemon ↔ Sy daemon): The normative peer protocol under ALPN `synergos/2` (the session itself is SPEC-SY-hlo2-session-binding). The SESS frame carries `caps: u64` = {txf2, range_plan_v1, heads_v1, relay_v1, roles_v1, chunk_have_v1} and `wire_min/wire_max`; peers use the intersection. Every stream starts with `magic[4] | ver u8 | flags u8 | rsv u16`; an unknown version resets the stream with app error `0x5304 unsupported_version`. The spec lists every magic and encoding (MessagePack frames with fixed binary digests). TXFR v1 is documented and frozen for golden-vector interop tests only; it is not accepted in production on `synergos/2`. Corrects the cpp-migration text "preserve protobuf schemas" → MessagePack.

- 仕組み: 全 frame 種別を golden vector 付きで仕様に列挙し、CTest で検査する。Rust の v1 encoder は interop vector の生成にだけ使う。移植計画から未使用の prost 依存を外す。
- 不変条件: 未知の版の stream は `0x5304` で reset。機能は両端の `caps` の積集合だけを使う。
- 失敗時: SESS の交渉で積集合に `txf2` が無ければ接続を閉じる (組織内で混在しない前提。§5.2)。
- 変更対象: `synergos-net/native/` (新設、wire codec)、`spec/feature/native-contracts.md` (新節)。置き換え対象: `synergos-net/src/quic/mod.rs:543-546,578-580`、`synergos-net/src/transfer/mod.rs:130-142`。
- 担保手段: ローカル検証ゲート (frame vector、decoder の上限は security SPEC-SY-decode-bounds)。

### 4.7 TRANSFER-M07 TXF2 受信側主導の範囲 pull

- 区分: 互換契約の変更 (SPEC-SY-txf2、W)。対象発見: TRN-WP-03, TRN-WP-04, TRN-WP-05, TRN-WP-08, TRN-WP-19, TRN-DD-03, TRN-DD-04, TRN-TP-02, TRN-DD-16。フェーズ T2。
- 目的: 受信側が送信元と範囲を選び、中断から再開し、保持者が N 人でも 1 部だけ受け取る (TG-05, TG-08。TRN-WP-04, TRN-WP-19, TRN-TP-02)。小ファイル大量の固定費を減らす (TRN-DD-16、AGD-GW-19)。

> **SPEC-SY-txf2** (Sy daemon (receiver) ↔ Sy holders (member, seed, archive, edge)): Receiver-driven pull. `WANT{req_id u32, project_ref[16], blake3[32], size u64, ranges[(off u64, len u64)] ≤ 64, prio u8}`; `WANT_MANY{req_id, project_ref, items[(blake3, size)] ≤ 4096}` for objects < 1 MiB, answered as one concatenated stream; `HAVE{req_id, blake3, size, est_bps u32, serve_slots u8}` answers a source probe; `DATA{req_id, off u64, len u32, bao_slice_len u32} + bao_slice + bytes` with frames of 1 MiB (objects ≥ 64 MiB), 256 KiB or 64 KiB by size class; `RECEIPT{req_id, blake3, verified_bytes u64, status: accepted | rejected(code)}`; `CANCEL{req_id, code}`. The expected `blake3`/`size` come from a signed Head or manifest entry, never from the holder. Pin, stale, quota and size decisions are made on WANT or the first DATA, before any body byte is persisted (else STOP_SENDING). Pins bind to `req_id` and expire only when idle. Partial state is `.synergos/incoming/<blake3>.part` plus a bitmap of verified 16 KiB groups, persisted every 8 MiB; resume requests only missing ranges. The RECEIPT is the only completion signal for the holder. Unsolicited pushes do not exist.

- 仕組み:
  - 送信元候補は HaveSet・Head・directory から得て、`HAVE` で確かめる。
  - 1 実体を最大 `transfer.max_sources` (既定 4) の互いに重ならない stripe に分け、残り 5 % 未満で endgame (同じ範囲を複数に要求し、先着以外を `CANCEL`)。
  - Bao グループの破損は 16 KiB 以内で中止し、その実体についてその送信元を除外して範囲をキューに戻す。
  - 小さい実体 (<1 MiB) は `WANT_MANY` 1 本で取り、ファイルごとの stream を作らない。
  - 送信側は object store から読む (M02)。読込先行 4 frame、`bytes` を pool から `write_chunk` で渡す。
- 不変条件: 要求されない push は存在しない (security SPEC-SY-pull-only-receive)。期待値は署名源から取る。保持者にとって完了の合図は `RECEIPT` だけ。転送中の pin は失効しない (TRN-WP-05 の解消)。
- 失敗時: 全送信元が失敗したら wanted-set に残し、次の HaveSet 照合で再試行する。`.part` と bitmap は保持者の変更をまたいで有効 (内容アドレスなので)。失効 (security revocation bundle) を受けたら 60 秒以内に該当 `req_id` を全部 `CANCEL`。
- 変更対象: `synergos-net/native/` (TXF2 codec と stream 処理)、`synergos-core/native/` (受信状態、wanted-set との結合)。置き換え対象: `synergos-net/src/transfer/mod.rs` 全体、`synergos-core/src/exchange/mod.rs` の push 経路 (`:638-674`, `:731-781`, `:895-956`, `:1249-1252`)。
- 担保手段: ランタイム強制 + ローカル検証ゲート (§6.2 試験 1〜5)。

### 4.8 TRANSFER-M08 ネイティブ転送プロファイル、swarm スケジューラ、計測

- 区分: **ネイティブ実装要件** (契約は変えない)。対象発見: TRN-WP-01, TRN-WP-03, TRN-WP-10, TRN-WP-11, TRN-TP-02, TRN-TP-13, TRN-TP-15, TRN-DD-19。フェーズ T2。security NR-1 (accept を直列にしない) と NR-2 (peer 単位・種別単位の stream 予算) と同じ改修で、窓の値は本書、予算の下限はセキュリティ文書が持つ。
- 目的: TG-01〜TG-03 と TG-11。
- 仕組み (設定 `[transport.quic]`、C++ の QUIC スタックが同等の操作を公開すること):

| キー | 既定値 | 根拠 |
|---|---|---|
| `stream_receive_window` | 16 MiB (1.25〜32 MiB に clamp。目標帯域 × 最大 RTT。100 ms で約 1.3 Gbps) | quinn 既定 1.25 MB で約 100 Mbps (TRN-WP-01) |
| `send_window` | 64 MiB | 同上 |
| `receive_window` | min(streams × stream 窓, 256 MiB) | 受信側メモリの上限 (security NR-2) |
| `congestion` | `cubic` (`bbr` はフラグ) | D-T20 |
| `initial_mtu` / `mtu_upper_bound` | 1200 / 1452 (relay・overlay 経路では下げる) | `synergos-net/src/config.rs:694-699` の設定が適用されていない (TRN-TP-05) |
| `gso` / `gro` | on | D-T21 |
| `max_pending_handshakes` | 256、負荷時は retry によるアドレス検証、`Incoming` ごとに 1 task | TRN-TP-15、security NR-1 (セキュリティ側の既定 64 との差は Q18 で揃える) |
| `transfer.max_sources` | 4 | D-T05 |
| `transfer.rate.global` / `per_peer` / `per_project` | 無制限 / 無制限 / 無制限 (token bucket、運用で設定) | TRN-WP-10 |
| `transfer.wire_compress` | `auto` | D-T18 |

  - スケジューラ: 優先度 Interactive > Background > Prefetch に aging。peer あたりの並行数 Large 2 / Medium 4 / Small 16。制御 stream の予約枠。HDD では書込 1〜2 本。
  - 送信元スコア: 拠点の一致 > 完了した範囲から出す EWMA throughput > RTT (5 秒ごとに接続統計を採取) > relay 経由。
  - 計測: `NetworkSnapshot` を IPC とイベントで公開。帯域推定は実転送の EWMA で、試験用トラフィックは流さない。
  - 死に設定 (`speed_test`、`peer_selection`、`stream_allocation`、Conduit、Monitor) は移植せず削除する。
- 不変条件: 窓 ≥ BDP を満たさない設定は起動時に警告する。制御 stream はデータで枯渇しない。
- 失敗時: BBR で実装の異常が出たら接続単位で Cubic に戻す。計測が取れない経路はスコア最下位にする。
- 変更対象: `synergos-net/native/` (transport 設定、scheduler、metrics)。置き換え対象: `synergos-net/src/quic/mod.rs:445-465`, `:558-573`、`synergos-core/src/exchange/mod.rs:940-956`、`synergos-net/src/conduit/`。
- 担保手段: ローカル検証ゲート (TG-01〜TG-03、TG-11 の計測ケース) + ランタイム強制。

### 4.9 TRANSFER-M09 複製 stream (Head、HaveSet、wanted-set)

- 区分: 互換契約の変更 (SPEC-SY-replication-stream、W)。対象発見: TRN-TP-01, TRN-TP-11, TRN-TP-12, TRN-WP-02, TRN-WP-09, TRN-WP-18, TRN-DD-16, TRN-DD-20。フェーズ T3。
- 目的: 星形を含む全トポロジで publish を 60 秒 p95 以内に全オンラインメンバーへ届け、切断や通知の欠落があっても照合 1 回で追い付く (TG-09。TRN-TP-01, TRN-WP-02, TRN-WP-18、AGD-GW-10)。

> **SPEC-SY-replication-stream** (Sy daemon ↔ Sy daemon): One long-lived bidirectional `SYNC` stream per peer pair, opened after SESS. Frames: `HEAD{project_ref, manifest_root, seq u64, prev_root, publisher_peer, sig}` (one per publish batch, signed under the security envelope); `HAVESET{project_ref, root, iblt | bloom, count}` exchanged on connect and every 60 s ± 20 % jitter; `DIFF_REQ{root_a, root_b, subtree_cursor}` / `DIFF{entries[]}`. Only Heads signed by a publish-role member are accepted; `seq` is monotone per publisher. Receiving content re-advertises it through HAVESET, so propagation is transitive. The per-project wanted-set is durable; pulls use SPEC-SY-txf2. Gossip carries only Heads and directory objects (hop limit 3, nonce + TTL in the message id); FileWant over gossip and the RootCatalog/bitswap round trip are removed. A lost frame costs at most one reconciliation interval.

- 仕組み: publish バッチ 1 回で Head 1 件 (M 件の FileOffer を置き換える)。60 秒 ±20 % の HaveSet 照合で切断中の欠落を直すので、どの通知も必須ではない (TRN-WP-02)。再試行は新しい `req_id` で行い gossip の再送ではないので、内容由来の message id 抑止に飲まれない (TRN-WP-18)。
- HaveSet の符号化: IBLT を必須、bloom を任意とする。bloom を使うのは両端の SESS の `caps` に bloom 対応の bit がある時だけで、無ければ IBLT を使う。IBLT の復号に失敗したら `DIFF_REQ` へ進む。上の契約文は `iblt | bloom` の選び方を定めていないので、caps bit の名前 (案: `haveset_bloom_v1`) と必須・任意の文言を契約の登録簿の所有者へ挙げる (Q20)。
- 不変条件: wanted-set は `state.db` に永続化。Head の `seq` は publisher ごとに単調。受信した内容は HaveSet で再広告され、伝播は推移的になる。
- 失敗時: 署名が通らない Head は捨てて security イベントに記録する。HaveSet が大きく食い違えば `DIFF_REQ` で部分木ごとに詰める。
- 変更対象: `synergos-net/native/` (SYNC stream)、`synergos-core/native/` (wanted-set)。置き換え対象: `synergos-net/src/gossip/` (FileWant / FileOffer の経路)、`synergos-core/src/catalog_sync.rs`、`synergos-core/src/exchange/mod.rs:1343-1408`、`synergos-net/src/content/bitswap.rs`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (星形伝播試験 §6.2 試験 6、HaveSet 符号化の相互運用試験 31)。

### 4.10 TRANSFER-M10 ノードの役割と directory (Cernere 署名の card と名簿)、DHT の削除

- 区分: 互換契約の変更 (SPEC-SY-node-roles、SPEC-SY-directory。どちらも A)。対象発見: TRN-TP-09, TRN-TP-10, TRN-TP-12, TRN-TP-14, TRN-TP-17。フェーズ T3 (SPEC-CR-membership-issuer と SPEC-CR-node-roster の稼働が前提、§4.27)。
- 目的: Cernere が署名した役割・宣言到達性 (node card) と会員 (ProjectRoster)、ノードが署名した計測到達性 (Presence) を control が directory として配り、データ面の構成をその検証済みの要素で決める。どのノードが誰のもので、どのプロジェクトの会員かを control は決めない (オーナー決定 8)。daemon 再起動後も招待で参加したノードが戻れるようにする (TRN-TP-09, TRN-TP-12, TRN-TP-14, TRN-TP-17)。

> **SPEC-SY-node-roles** (Cernere-signed card → Sy control, Sy daemons): The `topo` member inside the Cernere-signed SPEC-SY-node-card payload: `{roles ⊆ {member, seed, archive, relay, edge, custody-transfer, chunk-store}, site_id, relays[{url, region}], capacity{egress_mbps, cache_bytes}, caps}`, approved at enrollment or card re-issue (SPEC-CR-node-roster). Measured reachability `candidates[{type: host | srflx | mapped | relay, addr, prio, exp}]` is carried in the node-signed `Presence` of SPEC-SY-node-card, never in the card. Config `[node] modes = [...]`, `site_id`; a node whose configured modes are not a subset of its card's `roles` refuses to start. Start-up validation rejects forbidden co-residence (custody-admin or control-admin with any Internet-facing mode), per SPEC-SY-custody-topology. Reachability (`candidates`) is measured, never guessed from a global IPv6 address, an answering UPnP gateway or a `cloudflared` binary on PATH. The `relay` role is served only by the C++ relay (SPEC-SY-relay-v2).

> **SPEC-SY-directory** (Sy control-node → Sy daemons, relays, edges): `GET /v1/nodes/self/directory?project=<ref>&etag=` authenticated by a membership LCT returns `{project_ref, roster (the Cernere-signed ProjectRoster of SPEC-CR-membership-issuer), seeds[card], relays[card], edges[card], members_sample[card ≤ 32, chosen by site and RTT], presences[], site_map{site_id: [cidr]} (taken from the cards), revocation_seq, roster_epoch, exp}`, where every card is Cernere-signed (SPEC-SY-node-card) and every presence is node-signed. Control does not sign the directory: it selects, caches, orders and may omit entries, but it cannot add a node, a member or an endpoint, because each element verifies on its own; a node drops any card absent from the ProjectRoster's `nodes[]`. `exp` = the smallest `exp` of the roster and the listed cards, at most 24 h. Refresh every 5 min; the signed heartbeat response carries `{directory_etag, revocation_seq, roster_epoch}`. Membership is derived only from the Cernere ProjectRoster and membership LCTs (SPEC-CR-membership-issuer). Nodes persist the cards next to `projects.json`, dial seeds and relays first (8 in parallel, exponential backoff with full jitter capped at 5 min, immediate retry on network change), and fall back to an mDNS beacon `{peer_id, card_hash}` on the same site. After `exp` a node may only keep already-authenticated sessions. DHT1 is not ported.

- 仕組み: control が rendezvous になる。ただし control は Cernere 署名の card・ProjectRoster と node 署名の Presence を選び・キャッシュし・並べるだけで、directory に署名せず、ノード・会員・endpoint を足せない。heartbeat の応答を読み (TRN-TP-14)、directory は seed を先頭に並べる。enrollment と会員証の更新は heartbeat 経由で control が Cernere へそのまま中継する (security SPEC-SY-node-enrollment、SPEC-SY-membership-token)。役割・project scope・宣言 endpoint の変更は Cernere での card 再発行で、control では変えられない。gossip の fan-out はプロジェクトごとに上限 (mesh 次数 6、seed は常に mesh に入る)。
- 不変条件: directory の各要素は Cernere (card、ProjectRoster) か node (Presence) の署名で単独に検証でき、`exp` は roster と card の最小値で ≤24 時間。ProjectRoster の `nodes[]` に無い card と、Cernere 署名でない card・roster は捨てる。control は会員・所有・到達性を決めず、会員証・名簿の発行鍵を持たない。期限後は認証済みのセッションだけを保つ。会員情報はプロジェクトの会員にしか見えない。
- 失敗時: control が落ちても、永続化した Cernere 署名 card で seed と relay に再接続できる。同一拠点は mDNS で見つけ合う。Cernere が落ちた場合は、手元の card と roster の `exp` (≤24 時間) まで新規接続を続けられ、その後は認証済みのセッションだけを保つ (Cernere が署名した OfflineLease があれば、その拠点では security SPEC-SY-revocation-bundle の範囲で延びる)。
- 変更対象: `synergos-control/native/` (新設、directory endpoint、Cernere への enrollment・会員証更新の中継、card・roster のキャッシュ。発行鍵は持たない)、`synergos-core/native/` (card・roster の検証とキャッシュ、Presence の署名、再接続)。置き換え対象: `synergos-core/src/control_heartbeat.rs:116-121`、`synergos-core/src/presence/`、`synergos-core/src/peer_link.rs`、`synergos-net/src/dht/` (移植しない)、`synergos-net/src/promotion.rs`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (directory 期限切れ試験と roster 外・非 Cernere 署名の card の破棄試験 §6.2 試験 15、再起動後の再接続試験)。

### 4.11 TRANSFER-M11 経路確立と NAT 越え

- 区分: ネイティブ実装要件 (+ SPEC-SY-node-roles の `candidates` を通じた契約)。対象発見: TRN-TP-04, TRN-TP-05, TRN-WP-16。フェーズ T3。
- 目的: 到達性を推測ではなく計測し、同一拠点の組は LAN で直結する。
- 仕組み: 候補は QUIC の socket 上で集める。STUN の magic cookie (0x2112A442) で demux して binding 要求に答える。PCP / NAT-PMP / UPnP の対応付けを作って更新する。接続は 3 秒の予算内の happy-eyeballs: 同一拠点の LAN host 候補 → IPv6 → IPv4 → server-reflexive + `CALL_ME_MAYBE` の同時 open → relay。昇格はネットワーク変化時と 10 分ごとに再評価する。Cloudflare Tunnel は HTTP の制御と card の配布だけを運び、データは運ばない。overlay は任意の下層にする。C++ relay までは Tailscale が唯一の NAT 越え経路で、`synergos-node` は tailnet IP にだけ bind する (オーナー決定 7)。計測した候補は node 署名の Presence で広告し、Cernere 署名の card には入れない (SPEC-SY-node-card)。
- 不変条件: 候補には有効期限があり、計測されたものだけを広告する。
- 失敗時: すべて失敗したら home relay (M12)。状態表示は実際の経路を示す (偽の成功を返さない)。
- 変更対象: `synergos-net/native/` (候補収集、STUN demux、ポート対応付け)。置き換え対象: `synergos-net/src/promotion.rs:88-205`、`synergos-net/src/conduit/`、`synergos-net/src/tunnel/`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (network namespace での NAT 行列: full-cone、symmetric、CGNAT。§6.2 試験 26)。

### 4.12 TRANSFER-M12 relay v2

- **着地 (オーナー決定 7、2026-10-23 夕)。** 外向きのセキュアな中継 (DERP 型 relay: 両端が 443 へ outbound で接続し、relay は end-to-end 暗号化された QUIC を中継するだけ) は C++ 版でだけ作る。SPEC-SY-relay-auth と SPEC-SY-relay-v2 は C++ 移行の契約で、現行 Rust には何も足さない (`synergos-relay` の Rust 実装は改修も配備もしない)。C++ relay が受入ゲートを通るまで、拠点と外部環境 (Codex クラウドなど) は Tailscale でつなぎ、`synergos-node` は tailnet IP にだけ bind する (Synergos `docs/site-operations.md`)。
- 区分: 互換契約の変更 (SPEC-SY-relay-v2、W)。対象発見: TRN-TP-03, TRN-WP-16。フェーズ T3。
- 目的: RelayOnly の組の 100 % にデータ経路を作り、偽の中継成功を無くす (TG-10。TRN-TP-03, TRN-WP-16)。

> **SPEC-SY-relay-v2** (Sy relay ↔ Sy daemons): Transport framing for relayed sessions. Primary listener QUIC on UDP 443; fallback WSS on TCP 443 (HTTP/2 CONNECT where a proxy requires it). Frames are `{dst_peer_id[32], datagram}` within a room joined per SPEC-SY-relay-auth; no broadcast. The relay is content-blind: it forwards end-to-end QUIC ciphertext of SPEC-SY-hlo2-session-binding. The same socket answers STUN binding requests (magic cookie 0x2112A442) with the observed address, and signals `CALL_ME_MAYBE{candidates}` for simultaneous open. Under backpressure the relay drops datagrams (QUIC congestion control recovers); it never queues unboundedly. Per-client byte meter and idle timeout. The Rust WS room broadcaster is retired.

本契約は C++ でだけ実装する (オーナー決定 7)。`native-contracts.md` の `[id:]` 節として移し、Rust 実行時には入れない。暫定の到達経路は Tailscale (tailnet IP への bind) で、relay の代替ではなく、C++ relay が無い間の唯一の経路である。

- 仕組み: peer の home relay は directory の中で RTT 最小の relay (自拠点を優先)。relay の候補は Cernere 署名の card で `relay` 役割を持つノードだけで、control は relay を足せない。参加の認証は security SPEC-SY-relay-auth (追補 X1 で UDP 443 の QUIC も対象にする)。
- 不変条件: relay は内容を見ない。放送しない。キューを無制限に溜めない。
- 失敗時: relay が止まったら 10 秒以内に directory の次の relay へ移る (TG-10)。
- 変更対象: `synergos-relay/native/` (新設)。置き換え対象: `synergos-relay/src/server.rs` (room 放送、`:80-87`)、`synergos-net/src/relay/`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (relay 停止からの移行 ≤10 秒、中継 throughput ≥80 %)。

### 4.13 TRANSFER-M13 seed と archive の mode、ストリーム型 offload

- 区分: ネイティブ実装要件 + 運用。対象発見: TRN-TP-09, TRN-DD-07, TRN-DD-08。フェーズ T3。
- 目的: 常時稼働の供給元を拠点ごとに置き、publisher の送出を拠点あたり ≤1.2× にする (TG-08)。履歴の offload で最大 RSS を ≤256 MiB に抑える (TG-14、TRN-DD-08)。
- 仕組み:
  - seed は常時稼働で、対象プロジェクトの Head をすべて自動 pull し、directory の先頭に載り、メンバーごとの上限の下で範囲を配信する。`seed.projects` は本番では明示列挙が必須で、`"*"` は拒否する (security)。
  - archive は現在の履歴保管で、旧版は認可された restore にだけ返す。
  - offload は project lock の外で multipart のストリームとして流す (part ≥ max(10 MiB, size/10 000))。実体全体を RAM に載せない (TRN-DD-08)。チャンク層が on なら archive の保管はチャンク保管へ移る。
  - 保持の義務は security SPEC-SY-retention-ack と SPEC-SY-retention-lock。
  - 拠点 seed の配備は Excubitor catalog の項目として定義する (運用文書に記録)。
- 不変条件: seed が持つのは対象プロジェクトだけ。offload 中も project lock を保持しない。
- 失敗時: offload の part 失敗は part 単位で再試行。seed が落ちたら directory の次の seed かメンバーから取る。
- 変更対象: `synergos-core/native/` (mode 処理、offload)。置き換え対象: `synergos-core/src/history/rotation/s3.rs:66,83-87`、`synergos-core/src/history/rotation/gdrive.rs:210`、`synergos-core/src/history/rotation/local.rs`。運用: `excubitor.catalog.yaml` に拠点 seed の項目。
- 担保手段: ランタイム強制 + 記憶 (配備手順) + ローカル検証ゲート (100 GB offload の RSS 試験 §6.2 試験 21)。

### 4.14 TRANSFER-M14 custody 転送 v2 と内容アドレスの範囲取得

- 区分: 互換契約の変更 (SPEC-SY-custody-transfer-v2、SPEC-SY-object-get。どちらも W)。改訂する既存契約: SPEC-SY-opaque-custody、SPEC-SY-custody-durability (`opaque-custody.md:28` の受領証時の再検証、§3.20 変更提案 6)、SPEC-SY-custody-acquisition (`opaque-custody.md:40`、§3.20 変更提案 4)。保管キーを内容由来にする点は 2026-09-08 決定の解釈の変更 (§3.20 変更提案 7)。対象発見: TRN-WP-15, TRN-DD-09, TRN-DD-10, TRN-DD-11, TRN-E2-02, TRN-E2-05, TRN-DD-19。フェーズ: T0 仕様、T2 C++ (security P3 と同時)。
- 目的: custody 経路の再開 (TG-05)、100 GB の完走 (TG-06)、保存の往復 (CG-01 の custody 側) を満たす。verify の全量再読込 (TRN-DD-10、TRN-E2-02) と同じバイトの重複保管 (TRN-DD-11) を無くす。clone の認可を集合単位 1 本にする (TRN-E2-05)。

> **SPEC-SY-custody-transfer-v2** (Clients, Cu, Sy seeds/edges → Sy custody-transfer): Amends SPEC-SY-opaque-custody and SPEC-SY-custody-acquisition for the byte path. **Upload:** `draft-ietf-httpbis-resumable-upload` — `POST /v2/uploads` → `201 Location` (+ `104` interim); `PATCH` with `Upload-Offset`; `HEAD` → offset; `Upload-Limit: min-append-size=10MiB, max-size=<org maxObjectBytes>`; offset mismatch → `409` with the offset. Custody keeps the `.part` and the serialized hasher state, so resume needs no re-hash; the per-key lock covers only commit. **Commit:** compute and compare digests, fdatasync, exclusive publish under the content key (`HMAC(project_key, blake3)`, per SPEC-SY-envelope-encryption); an existing key returns `200 {alreadyStored: true}` without storing bytes again. The response is a SPEC-SY-custody-receipt-v2 receipt, produced **once**; `GET /v2/receipts/{key}` returns it without reading data. `verify` is O(1) (size + inode + stored receipt). Full re-hash is `POST /v2/scrub` → `202 {jobId}` plus a scheduled scrub (`custody.scrub.interval_days = 30`, rate-limited). **Download:** see SPEC-SY-object-get. Wall-clock caps (1800 s, 60 s) become idle timeouts (`custody.idle_timeout_s = 120`). v1 single PUT stays behind `custody.v1_compat` until Curare migrates.

> **SPEC-SY-object-get** (Clients, seeds, edges → Sy custody, edge, seed HTTP): Content-addressed range read: `GET /v2/objects/{blake3}` with `Accept-Ranges`, `206`, `If-Range`, `ETag = "b3:<hex>"`, ≥ 1 MiB reads or sendfile; `GET /v2/objects/{blake3}.bao?range=` returns the Bao slice for a verified range. Authorization is a custody-capability LCT whose `object_keys[]`/`prefix` caveat covers the object (one capability authorizes all ranges of all objects in its set). Range requests are ≤ 1 GiB; in-flight responses are aborted within 60 s of a revocation. Served by custody, site edges, egress edges and seeds alike, so clients resolve location without changing protocol.

- 仕組み:
  - hasher の状態を直列化して `.part` と一緒に保つので、再開しても再ハッシュしない。per-key lock は commit の間だけ。100 GB の sparse fixture で upload → 50 % で kill → 再開 → commit → O(1) verify → 範囲読みを通す。
  - 受領証の保管 (receipt store): commit で生成した SPEC-SY-custody-receipt-v2 を `receipts(key PK, receipt_bytes, sig, locations[], durability, committed_at)` に置く。`GET /v2/receipts/{key}` と `verify` はこの行と `stat` だけで答え、データを読まない。確定後は追記だけで、複製 (M16) が `locations[]` と `durability` を更新する。
  - 内容由来の保管キーと重複排除 (TRN-DD-11): 同じ重複排除範囲の同じ内容は `alreadyStored` で 1 回だけ置く。範囲の外には存在を答えない (SPEC-SY-envelope-encryption、Curare M20)。
  - 集合単位の認可 (TRN-E2-05): SPEC-SY-object-get は Curare fetch session のページ capability 1 本 (`object_keys[]` か `prefix`) で集合内の全実体の全範囲を許し、実体ごとの custody admin 呼出を不要にする。
  - v1 キーの索引: `custody.v1_compat` が on の間、C++ custody は `v1_keys(v1_key PK = sha256(opaqueObjectId), content_key)` を保ち、v1 の GET・verify・reclaim を content key へ引く。§5.3 の巻き戻しもこの索引を使う。
  - 運用 (大きさの上限): custody 全体の `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` は天井で、実効上限は組織ごとの `maxObjectBytes` (Curare の `object_size_policies.max_object_bytes`、Curare 側文書 §4.7、既定 10 GiB) とし、`Upload-Limit` の `max-size` に載せる。現在の catalog は 10 GiB で「Curare の単発 PUT 上限と合わせる」と注記している (`excubitor.catalog.yaml:31-32`)。T2 で custody v2 を有効にする時に catalog の値を 1 TiB に上げ、注記を「組織ごとの上限の天井」に改める。v1 の単発 PUT は 10 GiB のまま。TG-06 の計測用組織は `maxObjectBytes` ≥100 GB に設定する。
- 不変条件: 受領証は再計算しない。全量の再読込は scrub だけ。wall-clock の上限を置かず idle timeout だけ。
- 失敗時: offset が合わなければ `409` と正しい offset を返す。`.part` は `Upload-Limit` の max-age を過ぎたら GC。scrub で不一致が出たら security のイベントを出し、placement 状態を `evicted` 候補にする (Curare が判断)。
- 変更対象: `synergos-custody/native/` (既存の `durable_*`, `partial_file.*`, `signature.*` を土台に、upload resource・range GET・receipt store を追加)。置き換え対象: `synergos-custody/src/handlers.rs:218-255`、`synergos-custody/src/store.rs:141-207`。`spec/feature/opaque-custody.md:40` の記述を改訂 (§3.20 変更提案 4)。
- 担保手段: ランタイム強制 + ローカル検証ゲート (100 GB 再開、TG-05 / TG-06、§6.2 試験 1、O(1) verify の読込 0 バイト試験 22)。

### 4.15 TRANSFER-M15 custody の一括受付

- 区分: 互換契約の変更 (SPEC-SY-custody-batch-admission、W)。対象発見: TRN-E2-01 (Curare), TRN-E2-10 (Curare), TRN-DD-11。フェーズ T2 (Curare の利用側は T1 で MinIO を先に)。
- 目的: Curare の changeset 1 件 (≤10 000 件) の custody 受付を 1 回の呼出にし (CG-01 の custody 側)、既に保管済みの内容を `present` で答えて再 upload を無くす (TRN-DD-11)。

> **SPEC-SY-custody-batch-admission** (Cu → Sy custody-admin): `POST /v2/custody/admissions:batch {operationId, items[≤ 10 000]{org, project_ref, size, digests?, purpose}}` → `{items[{index, state: present | upload, uploadUrl?, partSize?}]}`. One call per Curare changeset; `present` is answered only within the dedup domain of the item. Reservations follow SPEC-CU-quota-reservation.

- 仕組み: Curare の changeset 1 件で custody 呼出 1 回。C++ custody ができるまでは、Curare の changeset は MinIO と、v1 facade 経由の Rust custody (サーバ側で 1 件ずつ受付。クライアント往復は増えない) に対して動く。
- 不変条件: `present` は重複排除範囲 (= KEK 範囲) の中でだけ答える。`operationId` で冪等。
- 失敗時: 一部の item だけ失敗した場合も index ごとの状態を返す。容量の予約は SPEC-CU-quota-reservation に従って戻す。
- 変更対象: `synergos-custody/native/` (admin 側 endpoint)。置き換え対象: `synergos-custody/src/handlers.rs` の単体受付。
- 担保手段: ローカル検証ゲート (§6.2 試験 23: 1 万件の受付が 1 回で終わり、`operationId` で冪等。Curare 側の CG-01 ケースと共通)。

### 4.16 TRANSFER-M16 (Synergos 側) custody 間の複製

- 区分: 互換契約の変更 (SPEC-SY-custody-replication、A) + 運用。配置方針 (SPEC-CU-storage-placement) と T0 の split origins は Curare 正本。対象発見: TRN-TP-06, TRN-TP-07, TRN-TP-08 (Curare)。フェーズ: T0 (split origins、Curare)、T4。
- 目的: (T0) admin と transfer の origin を分けた構成で、ブラウザーと遠隔のクライアントの upload / download を通す (TRN-TP-07、P0)。(T4) HQ の custody 1 台から全拠点が全バイトを取る構成をやめ、拠点ごとの複製を置く (TRN-TP-06)。

> **SPEC-SY-custody-replication** (Sy custody ↔ Sy custody): Custody nodes replicate committed objects custody-to-custody by content hash over SPEC-SY-object-get, driven by SPEC-CU-storage-placement. A replica commit appends its location to the `locations[]` of the stored receipt and updates `durability` to `durable_replicated` (security receipt-v2 fields). Replication never deletes; eviction is a placement-state change followed by the security lifecycle.

- 仕組み: Curare が配置を決め、Synergos custody が SPEC-SY-object-get で複製を実行する。release 確定には 2 拠点の `durable` が要る (Curare 側の規則)。
- 不変条件: 複製は削除しない。削除は placement 状態の変更と security SPEC-SY-custody-lifecycle の手順だけ。
- 失敗時: 複製の失敗は placement を `pending` のまま残し、Curare が再指示する。
- T0 で Synergos が行うこと (設定と文書の変更で、Rust のコードは変えない。TRN-TP-07 は P0 判定): (1) `excubitor.catalog.yaml` の admin / transfer 分割を Curare の `SYNERGOS_STORAGE_ADMIN_URL` と `SYNERGOS_TRANSFER_ORIGINS` に合わせて記述する (`excubitor.catalog.yaml:24-29`)。(2) `SYNERGOS_CUSTODY_CLIENT_ORIGINS` (現在は localhost だけ、`excubitor.catalog.yaml:33`) を、Curare の Web origin と `SYNERGOS_TRANSFER_ORIGINS` に合わせて拠点ごとに設定する。Curare 側文書 §4.1 がこの項目を本書へ回している。
- 変更対象: `synergos-custody/native/` (複製ワーカー)、`excubitor.catalog.yaml`。
- 担保手段: ランタイム強制 + ローカル検証ゲート (admin / transfer の origin を分けたブラウザーと CLI の upload / download と CORS の統合試験 §6.2 試験 7、複製が `locations[]` に追記し削除しない試験 25)。

### 4.17 TRANSFER-M17 edge mode (拠点 edge と egress edge)

- 区分: 互換契約の変更 (SPEC-SY-object-get の提供者として)。対象発見: TRN-TP-08 (Curare), TRN-E2-05 (集合単位 capability の検証), TRN-E2-16, TRN-DD-18。フェーズ T4。
- 目的: ベンダーとクラウドのエージェントが大量バイトを userspace の tailnet proxy 経由で取らないようにする。
- 仕組み: edge は BLAKE3 をキーにした read-through キャッシュで、SPEC-SY-object-get を提供し、fetch session のページ capability (集合単位の LCT 1 本) を検証し、内部 overlay 経由で origin から充填し、admin の秘密を持たない。拠点 edge はオフィスに置く。egress edge は同じ mode を HQ の外に置くもので (D-T16、Q3)、magic link の `/dl` トークン交換 (SPEC-CU-magic-link-v2) を受け、ベンダーとクラウドエージェントに地域的に近い所から配信する。100 MB を超える本文は Cloudflare の HTTP proxy を通さない (grey-cloud か直結)。
- 不変条件: edge に admin 資格を置かない。失効を受けたら配信中の範囲応答を 60 秒以内に中止。
- 失敗時: edge が使えなければ Curare は origin の location に戻す。
- 変更対象: `synergos-custody/native/` (edge mode。custody-transfer と同じ HTTP 面を共有)。
- 担保手段: ランタイム強制 + ローカル検証ゲート (egress edge からの 100 GB clone の kill / resume、失効による中止 ≤60 秒)。

### 4.18 TRANSFER-M26 IPC v2 (ページング、job、Subscribe)

- 区分: 互換契約の変更 (SPEC-SY-ipc-v2、W。`SPEC-SY-native-framing` の意味を改訂)。対象発見: TRN-E2-14, TRN-E2-18。フェーズ T2。
- 目的: 大きなツリーの結果を ≤1 MiB のフレームに分けて返し、長い操作は 100 ms 以内に job id を返す (CG-06。TRN-E2-14)。GUI のポーリングを無くす (TRN-E2-18)。

> **SPEC-SY-ipc-v2** (Sy daemon → CLI, GUIs, Cu node client): Amends the semantics of `SPEC-SY-native-framing` (the 1 MiB per-frame cap is kept; total result size becomes unbounded). Adds request ids, `ResponseChunk{requestId, items[], cursor?, done}`, `JobStarted{jobId}` → `JobProgress` → `JobFinished` for Checkout/Restore/Fetch/Publish, `JobGet`, `JobCancel`, selectors `{paths[] | prefix | tag | manifestHash}`, `Status{prefix?, sinceCursor?}` over SPEC-SY-local-index, and one `Subscribe` connection with coalesced progress instead of GUI polling. Authorization and the first frame are SPEC-SY-ipc-capability (security).

- 仕組み: checkout・restore・fetch・publish を job にし、進捗を流す。GUI はポーリングをやめて `Subscribe` を使う。これが security D-13 で `synergos-gui` を外しても転送機能を失わない根拠 (§3.18)。
- 不変条件: 1 フレーム ≤1 MiB は維持 (`synergos-ipc/src/transport.rs:39`、`synergos-ipc/native/frame.cpp`)。長い操作は 100 ms 以内に `JobStarted` を返す (CG-06)。
- 失敗時: クライアント切断で job は止まらず、`JobGet` で再取得できる。`JobCancel` は進行中の TXF2 を `CANCEL` する。
- 変更対象: `synergos-ipc/native/` (既存の frame codec の上に request id と chunk)、`synergos-core/native/` (job 管理)。置き換え対象: `synergos-ipc/src/command.rs:93` (`ProjectCheckout`) と `:227` (publish は 1 呼出 1024 パスまで)、`synergos-ipc/src/response.rs:101-110` (`CheckoutReportDto`)。ドメインは codec が network-transport (`synergos-ipc/`)、job 管理が mesh-core (`synergos-core/native/`)。
- 担保手段: ローカル検証ゲート (100 万項目の Status がフレーム分割で返る、`JobStarted` ≤100 ms) + ランタイム強制。

### 4.19 チャンク保管の分離判断と TRANSFER-M27 チャンク保管モジュール

- 区分: 互換契約の変更 (新モジュール。SPEC-SY-chunk-store、A)。対象発見: TRN-DD-01, TRN-WP-12, TRN-DD-07, TRN-DD-08, TRN-DD-15, TRN-DD-23。フェーズ T5。
- 目的: 反復保存の大容量ファイルとチェックポイントで版間の再利用を得る (TG-07。TRN-DD-01, TRN-WP-12, TRN-DD-07)。チャンクの複雑さを常時稼働の役割に閉じ込め、メンバーには入れない。

**分離判断の要約。** §3.3 (D-T03) の比較で、独立モジュール (案 B) がプロセス分離の 2 条件の下で 4 軸すべてで本体同居 (案 A) 以上になった。分離の境界は次の 3 点である。

1. **API 境界**: SPEC-SY-chunk-store だけ。呼び手は seed / archive / custody / edge の 4 役割に限る。メンバー (通常ノード) はリンクしない。
2. **セキュリティ境界**: SPEC-SY-chunk-access (security 正本、追補 X5 で無条件化、security §4.0.2 で受入済み)。プロセス内リンクは API と PEP (policy enforcement point) の境界であって、プロセスと OS 資格の境界ではない。そのため次を必須にする。
   - custody はモジュールを out-of-process mode (ローカル socket、モジュール専用の OS ユーザー) でだけ使い、custody admin の資格とチャンク索引を同じプロセスに置かない。seed / archive / edge は admin 資格を持たない役割なので、プロセス内リンクでよい。
   - `gc` と KEK の unwrap を伴う処理は、独自の OS ユーザーと Vault policy を持つ別プロセスとして Excubitor の定期ジョブで動かす (SPEC-SY-chunk-access の「`gc` runs under a separate identity」)。本番設定ではプロセス内の `gc(...)` 呼出を拒否する (`chunkstore.gc.in_process = false` 固定)。
   - チャンク id は HMAC、DEK は重複排除範囲ごと。
3. **配備境界**: seed / archive / edge では当面プロセス内ライブラリ。custody 用と GC 用は最初から別プロセスで、同じ API をローカル socket に載せる (契約は変えない)。別サービス化 (案 C) は計測で独立拡張が必要になった時の次の経路。

**契約文の不足 (Q19)。** SPEC-SY-chunk-access は全呼出が LCT (membership か custody-capability) を運び、配信した範囲ごとに `content.served` を出すことを求める。下の SPEC-SY-chunk-store は ctx を `{org, project_ref, policy_class}` とし、capability も `content.served` の義務も持たない。契約文はここで書き換えず、契約の登録簿の所有者 (neco) へ次の追記を求める: `get_range`・`chunk_map`・`has` の ctx に `cap` (検証済み LCT か principal handle) を足す。`get_range` の応答ごとに `content.served` を出す義務を足す。回答までの実装は security 側の文面 (全呼出に LCT) に従う。

> **SPEC-SY-chunk-store** (Sy seed, archive, custody, edge → chunk-store module): The module API (C++ interface; the same shape over a local socket if split out): `put(stream, ctx{org, project_ref, policy_class}) → {blake3, size, chunk_map_id?}`; `has(ctx, chunk_ids[]) → bitmap`; `get_range(ctx, blake3, off, len) → bytes + bao_slice`; `chunk_map(ctx, blake3) → [(chunk_id, len)]`; `export(ctx, blake3, dst)` (whole-object recovery); `gc(ctx, mark_set, retention_locks) → report` under a separate identity. Chunker: FastCDC/Gear at Xet constants (min 8 KiB, target 64 KiB, max 128 KiB, fixed 64-bit gear table and mask, reset at boundary), frozen by published test vectors. Objects < 8 MiB or `policy_class = whole` are stored whole. Packs ≤ 64 MiB and ≤ 8192 chunks; each chunk header `{ver, codec, csize, usize}`; codec by trial (D-T18). Offload to S3/GDrive is per pack, multipart, streaming. `chunk_id = HMAC(project_dedup_key, chunk_blake3)`; dedup domain = project by default, org by opt-in. Security invariants are SPEC-SY-chunk-access. Per-project switch `project.chunking = off | measure | on`; `measure` computes would-be savings without storing chunks. If the module is unavailable, transfer falls back to whole-object TXF2; chunks are never the only copy before a pack is durable.

- 利用者一覧:

| 役割 | 使う API | 目的 |
|---|---|---|
| seed | `put`, `get_range`, `chunk_map` | 版系列の保管、range plan の計算 (M28) |
| archive | `put`, `get_range`, `export` (`gc` は別プロセスの GC ジョブが呼ぶ) | 履歴の保管 (現行の whole-file objects を置換)、offload |
| custody (out-of-process mode のみ) | `put`, `has`, `get_range` | custody 保管の重複排除、seed↔custody の `chunk_have_v1` |
| edge | `get_range`, `has` | キャッシュの重複排除 |

- `off / measure / on` のゲート: 出荷時は全プロジェクト `off`。`measure` は保存せずに削減見込みを計測値 `transfer.bytes{path = chunk}` に出す。`on` にするには、(a) スタジオの実データでの depot 計測 (Q6) と、(b) そのプロジェクトで 14 日間の `measure` が削減率 ≥30 % を示すことの両方が要る。削減率が 30 % 未満なら §3.2 の案 B (`zstd --patch-from`) を代替にする。
- フォールバック: モジュールが使えなければ全体の TXF2。pack が耐久化するまで、チャンクを唯一の写しにしない。`export` で常に実体全体を書き出せる (2026-08-16 の復旧性を保つ)。
- 不変条件: チャンク化しても同一性はファイル全体の BLAKE3。チャンク境界の定数は test vector で固定し、実装の変種で切れ目が変わらない (fastcdc-rs には切れ目の異なる 3 変種がある)。GC は retention lock を尊重する。
- 失敗時: GC の mark 漏れはプロパティ試験で検出する。pack の破損は scrub で検出し、他の写しから再構築する。
- 変更対象: 新ディレクトリ `synergos-chunkstore/` (C++ ライブラリ、独自の CMake target、`spec/domains/chunk-store.domain.json` を新設)。DESIGN §15.1 の固定 256 KiB chunker と平坦 DAG (`synergos-net/src/content/chunker.rs:36-41`) は superseded と明記し、移植しない。
- 担保手段: ローカル検証ゲート (chunker vector、GC の性質試験 §6.2 試験 17、capability 無しの `get_range` 拒否と GC の別主体の試験 27、暗号化有無で重複排除率が同じ = security E5) + ランタイム強制 (`project.chunking`、`chunkstore.gc.in_process = false`)。

### 4.20 TRANSFER-M28 メンバー向けの range plan

- 区分: 互換契約の変更 (SPEC-SY-range-plan、W)。対象発見: TRN-DD-01, TRN-WP-12。フェーズ T5。
- 目的: chunker を持たないメンバーが、版 n から n+1 へ変わった範囲だけを wire で受け取る (TG-07。TRN-DD-01, TRN-WP-12)。

> **SPEC-SY-range-plan** (Sy seed/archive/edge (with chunk-store) → Sy members): `PLAN_REQ{project_ref, want_blake3, have_blake3[] ≤ 4}` → `PLAN{want_blake3, size, ops[(copy, have_blake3, src_off, dst_off, len) | (fetch, dst_off, len)]}`, coalesced into runs ≥ 1 MiB. The holder computes it from both chunk maps. The member copies the local ranges, fetches the rest with SPEC-SY-txf2 (from any source), and verifies the result against the whole-object BLAKE3/Bao; on mismatch it discards the plan and does a whole-object pull. Members need no chunker or chunk index. A plan reveals only chunk boundaries of objects the member is authorized for.

- 仕組み: 版 n を持つメンバーが n+1 を欲しい時、チャンク層を持つ保持者に plan を求め、変わらない範囲をローカルでコピーし、残りを取り、全体を検証する。結果はファイル全体の同一性のままで、wire はチャンク単位の削減を得る。
- 不変条件: 検証はいつも全体の BLAKE3。plan は認可された実体のチャンク境界だけを見せる。
- 失敗時: 不一致なら plan を捨てて全体 pull。
- 変更対象: `synergos-net/native/` (PLAN frame)、`synergos-core/native/` (plan の適用)、`synergos-chunkstore/` (plan の計算)。
- 担保手段: ランタイム強制 + ローカル検証ゲート (§6.2 試験 11)。

### 4.21 TRANSFER-M29 形式方針と派生区分の hook

- 区分: 互換契約の変更 (SPEC-SY-format-policy、A)。対象発見: TRN-DD-17。フェーズ: T1 は Curare の pre-submission hook だけ報告モード、Synergos の pre-publish hook は T2 から報告モード (`synergos-core/native/` は T2 で入る)、T5 でチャンクプロジェクトに強制。
- 目的: 重複排除を壊す形式 (deflate 圧縮の FBX、派生 GPU テクスチャ) を版にする前に見つけ、チャンクプロジェクトでの再利用率を守る (TRN-DD-17)。

> **SPEC-SY-format-policy** (Sy (table owner) → chunk-store, Sy pre-publish hook, Cu pre-submission hook): `chunk_policy.toml` maps extensions and sniffed formats to `policy_class ∈ {chunk, whole, derived}`: chunk for PSD/PSB/EXR/TGA/MA/MB/uncompressed FBX/safetensors/GGUF/parquet/≥ 1 GB shards; whole for PNG/JPEG/MP4/BCn/ASTC/KTX2/zip family; derived for GPU-compressed textures produced by a pipeline. A pre-publish (Sy) and pre-submission (Cu) hook rejects deflate-compressed FBX in chunk projects (message points at the exporter preset) and derived outputs (pointing at the derived class, cached by (source BLAKE3, encoder settings)). Curare versioning types carry `policy_class` (AGD-AI-01).

- 仕組み: 表の正本は Synergos の `chunk_policy.toml`。契約文の根拠注記は ai-game の検証済み発見 AGD-AI-01 (データセット集約が無い) を指す。Curare 側文書の同じ契約文も同じ ID にする。報告モードでは拒否の見込み件数を数えるだけ。派生の GPU テクスチャは版ではなく (原本 BLAKE3, エンコーダ設定) をキーにしたビルドキャッシュへ送る。
- 不変条件: 強制はチャンク有効プロジェクトだけ。全体転送プロジェクトでは拒否しない。
- 失敗時: 判定できない形式は `whole`。
- 変更対象: `spec/` (表と仕様)、`synergos-core/native/` (pre-publish hook。hook の sandbox は SECURITY-M12)。
- 担保手段: フック + ローカル検証ゲート (§6.2 試験 12)。

### 4.22 TRANSFER-M30 (Synergos 側) `sync_mode` の尊重

- 区分: 互換契約の変更 (wanted-set の絞り込みとして SPEC-SY-replication-stream の上で)。選択 (selection) は Curare 正本、VFS は ai-game 文書。対象発見: TRN-DD-18, TRN-E2-16。フェーズ T4。
- 目的: 疎な作業領域とデータセットで、選んだ範囲の外を取らない (TRN-DD-18, TRN-E2-16。AGD-DM-13, AGD-GW-06)。
- 仕組み: `sync_mode` Manual / Selective を wanted-set のフィルタで尊重する。作業領域の疎な集合の外にある Head の項目は索引には入れるが pull しない (現状は `synergos-core/src/daemon.rs:984-986` で全部 pull、`synergos-core/src/checkout.rs:80-109` で `sync_mode` は保存だけ)。後から開いた時は SPEC-SY-object-get か TXF2 で取る。
- 不変条件: 疎な集合の外の実体は配信の要求があるまで取らない。
- 失敗時: フィルタ設定が読めない場合は Manual 扱いとし、既に展開済みの実体だけを維持する。
- 変更対象: `synergos-core/native/` (wanted-set フィルタ)。
- 担保手段: ランタイム強制 + ローカル検証ゲート (§6.2 試験 24)。

### 4.23 TRANSFER-M31 転送予算と計測ゲート

- 区分: 運用 + 試験専用コード (SPEC-SY-transfer-budget、A)。対象発見: TRN-WP-17, TRN-DD-01 (計測), TRN-DD-19。フェーズ: T0 ベースライン、以後の全フェーズ。
- 目的: TG-01〜TG-14 を計測可能な受入条件にし、C++ への切替を Rust ベースライン以上の性能に結び付ける。チャンク層の有効化を計測で決める (TRN-WP-17, TRN-DD-01、AGD-OP-11)。

> **SPEC-SY-transfer-budget** (Sy (owner) → Revisor cases): `docs/performance.md` holds TG-01…TG-13 with populations and bench profiles. The bench harness (Linux runner, netem profiles `lan`, `wan30`, `wan100`, `wan100-loss`; network namespaces modelling a site with a seed, 20 members, a NAT-ed remote, a relay and an egress edge) runs a short profile as a Revisor pre-merge case and a nightly long profile (100 GB). The C++ port must meet or beat the recorded Rust baseline before cut-over. Telemetry `transfer.bytes{project, route, ext, path = whole | chunk | plan}` feeds the D-T02 enablement gate.

- 仕組み: T0 で Rust v1 実行時に対してハーネスを回してベースラインを記録する (試験専用コード、D-T19)。以後の各フェーズは短いプロファイルで自フェーズのゴールを満たしてからマージする。夜間の長いプロファイルが 100 GB と 500 クライアントの嵐を受け持つ。GitHub CI は使わない (security D-12)。
- 不変条件: C++ への切替は Rust ベースライン以上の値が条件。`cpp-migration.md:64-71` の Completion gate に性能の parity を加える (§4.31)。
- 失敗時: 夜間プロファイルの後退は翌日の Revisor ケースを赤にする。
- 変更対象: `docs/performance.md` (新設)、計測ハーネス (`synergos-core/tests/` 配下の ignored テストと、C++ 側の CTest label)。
- 担保手段: ローカル検証ゲート。

### 4.24 TRANSFER-M33 Rust の暫定措置

- 区分: 運用。対象発見: TRN-WP-05, TRN-WP-18, TRN-TP-05。フェーズ T0。
- 目的: C++ 切替までの間、P0 級のデータ損失 2 件と LAN の折返しの被害を、Rust のコードを変えずに運用で小さくする (オーナー決定 6)。
- 仕組み: Rust に入れるのは計測ハーネスと設定だけ。SECURITY-M34 (設定だけの暫定補償) と同じ扱い。運用上の緩和を運用文書に記録する。
  - TRN-WP-05: 遅い回線越しの大きなファイルの restore を避ける。pin の期間は定数 `PINNED_FETCH_TTL_MS` (`synergos-core/src/exchange/mod.rs:26`、5 分) で、設定では変えられない。
  - TRN-WP-18: 止まった checkout を再試行する前に daemon を再起動して gossip キャッシュ (メモリ上) を消す。
  - TRN-TP-05: overlay は Tailscale。bootstrap 一覧は tailnet IP を書き、同一拠点では LAN アドレスを先に書く。
  - 暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。
  - WP-05・WP-18・偽の `Ok(Relay)` のコード修正は neco の回答 (Q4) を待つ。
- 不変条件: Rust 実行時のソース (`synergos-*/src/`) を変えない。変えてよいのは試験専用の計測ハーネス (`synergos-core/tests/` の ignored テスト) と設定値だけ。
- 失敗時: 緩和で防げない事象 (遅い回線での大きな restore の失敗、止まった checkout) は運用文書の手順で再試行し、発生を記録して Q4 の判断材料にする。Q4 で修正が許された場合も、その修正だけを別の Revisor 提出にする。
- 変更対象: 運用文書だけ (`docs/mesh-operations.md` への追記、`docs/performance.md`)。Rust のソースは変えない。
- 担保手段: 記憶 + ランタイム (設定)。

### 4.25 参照する Curare 側の契約 (正本は Curare 側: spec/plan/redesign-transfer-efficiency.md)

Synergos が呼ばれる側または入力を受ける側になる契約である。定義を 1 回引用し、詳細は Curare 文書による。

> **SPEC-CU-changeset-v2** (Cu → Web, CLI, node client): `POST /api/v2/changesets {operationId, items[≤ 10 000]{resourceId | newResource{name, path, logicalAssetId?}, path, size, digests?, purpose, submissionKey?, requires?: versionId[]}}` → `{changesetId, expiresAt, items[{index, state: unchanged | present | upload, upload?{url, protocol: ietf-resumable | s3-multipart, partSize, capability}}]}`. Authorization is one set-based query (`requireVersionsBulk`); `unchanged` is decided server-side against the latest digest of the asset; `present` comes from SPEC-CU-object-registry; Curare calls SPEC-SY-custody-batch-admission once per changeset. Digests may be supplied at commit, so hashing overlaps the upload. `POST /api/v2/changesets/:id/commit {items[{index, digests, receipt}]}` is one transaction: assets locked in id order, multi-row INSERT of versions and dependencies (acyclic by construction, no advisory lock), admissions consumed, one `ChangesetCommitted` event. Tables `changesets(id, org_id, created_by, operation_id UNIQUE per user, state, expires_at)`, `changeset_items(changeset_id, idx, resource_id, blob_ref, state, digests)`. `/api/v1/upload` stays as a one-item facade.

> **SPEC-CU-collection-version** (Cu → clients; Sy manifests registered as collections): `asset_versions.kind ∈ {file, collection}`; a collection row references `manifest_hash`. `manifests(hash PK = BLAKE3 Merkle root, entry_count, total_bytes, created_by, created_at)`; entries are content-addressed ≈ 64 KiB sorted chunks in the SPEC-SY-manifest-v2 entry encoding, stored as objects, so two collections diff in O(changed subtrees). A Synergos snapshot registers as a Curare collection by tree hash; neither side interprets the semantics of the other. Collections remove the 500/500/1000 item caps for trees.

> **SPEC-CU-object-registry** (Cu → changeset, fetch session, placement): `objects(org_id, project_ref, blake3, sha256, size, chunk_map_id?, refcount, created_at)`, UNIQUE `(org_id, project_ref, blake3)`. The dedup domain equals the KEK domain of SPEC-SY-envelope-encryption: project by default, org by opt-in. Storage names are derived per that contract, never plaintext hashes. `present` answers are given only inside an authorized domain, rate-limited per SPEC-CU-abuse-policy and recorded in SPEC-CU-capability-ledger. `blob_ref` stays a per-admission id; the registry maps it to content.

> **SPEC-CU-fetch-session** (Cu → Web, CLI, node client, agents): `POST /api/v2/fetch-sessions {target: release | versions[] | collection | manifestHash, selection?{prefixes[], patterns[]}, have?: blake3[] | haveBloom}` → `{sessionId, manifestHash, page{entries[≤ 1000], cursor?}, needs[], locations{blake3 → [{origin, role, site_id, prio}]}, capability{token, exp ≤ 300 s, renewUrl}}`. `GET /api/v2/fetch-sessions/:id/pages?cursor=` returns the next ≤ 1000 entries with their own capability. Each capability is one set-scoped custody-capability LCT (`object_keys[]` or `prefix`) per security D-18, ledgered in SPEC-CU-capability-ledger and charged to the budget of the principal. Renew re-issues only the capability of that page. Closure resolution is one recursive CTE joined with set-based authorization (all-or-nothing, by comparing counts). Clients fetch with SPEC-SY-object-get, 8–16 concurrent streams, `.part` resume with verified offsets, and prefer same-site edge or seed, then replica, then origin.

> **SPEC-CU-storage-placement** (Cu (policy) → Sy custody (execution)): `storage_nodes(id, site_id, role: origin | replica | edge | egress, transfer_origin, capacity_bytes, enabled, kid)` extends `shared_nodes`; `object_placements(object_id, node_id, state: pending | durable | evicted, verified_at)`. Allocation by (org, uploader site, capacity). Release finalize requires `durable` on ≥ 2 nodes. Config `SYNERGOS_STORAGE_ADMIN_URL` (private) and `SYNERGOS_TRANSFER_ORIGINS` (allowlist); a capability URL is accepted only if its origin is allowlisted and matches the placement; start-up fails fast when either is missing. Magic links and agents resolve to edge or egress origins.

> **SPEC-CU-api-budget** (Cu (owner) → Revisor cases): CG-01…CG-06 with populations: round-trip counts per save and clone, p95 at 10^6 assets per tenant on a seeded fixture, EXPLAIN-plan snapshots for hot queries. Run as Revisor pre-merge cases.

Synergos との関係: changeset は M15 を呼ぶ。collection は manifest v2 の entry encoding を共有する。object registry の重複排除範囲は custody の content key と同じ範囲。fetch session のページ capability は custody / edge / seed の SPEC-SY-object-get で検証される。placement は M16 と M17 を駆動する。api-budget の CG-06 は Synergos の IPC (M26) が満たす。

### 4.26 参照するセキュリティ側の契約と、転送からの追補要求

security 正本の ID は再定義しない (`spec/plan/redesign-security.md`)。

| ID | 転送での使い方 |
|---|---|
| SPEC-SY-hlo2-session-binding | SPEC-SY-peer-wire・-txf2・-replication-stream が走る `synergos/2` セッション。組織ごとの flag day は 1 回 |
| SPEC-SY-membership-token | 会員証 (Cernere 署名の membership LCT。control は発行も再署名もしない)。新しい stream 種別 (TXF2, SYNC, PLAN, RLY) を本文の前で通す (追補 X4) |
| SPEC-SY-node-card | node card (Cernere 署名) に SPEC-SY-node-roles を `topo` として載せる (追補 X2)。計測した到達性 (`candidates`) は node 署名の `Presence` で運び、card には入れない |
| SPEC-SY-node-enrollment | enrollment の承認は Cernere (Cernere 署名の card を返す)。control は本文をそのまま中継するだけ。heartbeat は directory の鮮度と会員証の更新要求を運び、名簿を変えない |
| SPEC-SY-relay-auth | SPEC-SY-relay-v2 の参加認証 (追補 X1)。C++ でだけ実装する (オーナー決定 7) |
| SPEC-SY-revocation-bundle | `roster_status` (Cernere 署名の RosterStatus) と control の deny-only 部分の 2 部構成。directory と転送は `revocation_seq` と `roster_epoch` に反応し、配信中の処理を ≤60 秒で中止 |
| SPEC-SY-custody-capability-v2 | upload・object GET・fetch session のページを認可 (追補 X3) |
| SPEC-SY-custody-receipt-v2 | SPEC-SY-custody-transfer-v2 が 1 回生成。`locations[]` と `durability` は複製で更新 |
| SPEC-SY-custody-topology | D-T10 の mode 同居を制約 |
| SPEC-SY-custody-lifecycle | placement 変更後の退避と削除 |
| SPEC-SY-pull-only-receive | TXF2 は構造上 pull 専用。`WANT_MANY` と range plan も wanted-set の中だけ |
| SPEC-SY-envelope-encryption | 保管名とチャンク DEK の導出。重複排除範囲 = KEK 範囲 |
| SPEC-SY-chunk-access | チャンク保管境界のセキュリティ不変条件 (追補 X5) |
| SPEC-SY-decode-bounds | 本書の新しい frame 種別すべての上限 |
| SPEC-SY-retention-lock, SPEC-SY-retention-ack, SPEC-SY-displaced-preimage | チャンク GC は lock を守る。seed と archive は保持の義務を負う。受信経路は置換された旧バイトを rename で退避 (追加コピーなし、security E6) |
| SPEC-SY-ipc-capability | SPEC-SY-ipc-v2 の最初のフレームと scope |

**追補 X1〜X5 (security §4.0.2 で受入済み)。** 転送から security 正本へ求めた次の 5 件は、security 文書 (`spec/plan/redesign-security.md` の冒頭と §4.0.2) が契約文に反映済みである。経緯の記録として残す。

| # | security ID | 受入前の記述 | 転送からの追補 (受入済み) | 理由 |
|---|---|---|---|---|
| X1 | SPEC-SY-relay-auth | 「WSS only.」 | 「TCP 443 の WSS か UDP 443 の QUIC。同じ challenge 参加を両方に適用。framing は SPEC-SY-relay-v2」 | WSS だけだと QUIC-in-WS で TCP の head-of-line blocking が起き、TG-10 に要る UDP 経路を失う |
| X2 | SPEC-SY-node-card | payload `{peer_id, pubkey, endpoints[], protocol, version, iat, exp}` | 署名 payload の中に任意の `topo` (SPEC-SY-node-roles) を足す | 署名 card を 1 つに保ち、`node-card-v2` に分岐させない |
| X3 | SPEC-SY-custody-capability-v2 | 「PUT は commit まで jti ごとに単回、再開は同じ jti を使う」、`digest` caveat あり | PUT の capability は `object_keys = ["upload:<upload_id>"]` と `max_bytes` を指定。`digest` は任意 (無ければ受領証のダイジェストが正で、Curare が changeset commit で比較)。長い upload 中の更新は同じ `upload_id` に新しい jti を発行。「jti ごとに単回」を「`upload_id` ごとに commit 1 回」に置換 | 100 GB の upload は 300 秒の capability より長い (TG-06)。ハッシュは upload と重ねる必要がある。保管名は内容由来でハッシュ前には分からない |
| X4 | SPEC-SY-membership-token | TXFR, BSW1, GSP1, DHT, FileWant, history serve を列挙 | TXF2, SYNC, PLAN, RLY を追加。C++ 移植で削除される DHT, BSW1, FileWant を外す | 新しい stream 種別を本文の前で検査する |
| X5 | SPEC-SY-chunk-access | 「Synergos (conditional) … 転送文書が独立チャンク保管を選んだ場合だけ存在」 | 無条件にする (D-T03 で案 B を採用) | — |

X2 の card は 2026-10-23 夕の決定 8 で Cernere 署名 (SPEC-CR-node-roster) になり、計測した到達性 `candidates` は card から node 署名の `Presence` へ移った (`design/authority-amendment.md` §1.5、§1.6)。`topo` を card の署名 payload に 1 つだけ持つ点は変わらない。

security 文書の §4.0.2 の X5 行は受入前の記述として「条件付き」を残している。同文書の §4.24 (SECURITY-M27) と native-contracts の対応表に「条件付き」の文言が残る版があれば外し、X5 を両文書で無条件にそろえるよう security 文書の担当へ依頼する。チャンク保管の API 側の不足 (ctx の capability、`content.served`) は Q19 で扱う。

### 4.27 Cernere への依存 (SPEC-CR-*、オーナー決定 4。Synergos には実装しない)

オーナー決定 4 は依頼の経緯を記録することを求める (`spec/plan/redesign-analysis/HANDOFF.md` §7.1 の 4)。3 件はまとめて Cernere へ Actio タスクとして依頼済みである。Actio のタスク id は未確認 (HANDOFF §7.1 に番号の記録が無い)。依頼の決定日は 2026-10-23 で、起票日は未確認 (同じ理由)。本書の提出前に Actio でタスク id と起票日を引き、下表の「依頼の記録」列に書き入れる。

2026-10-23 夕のオーナー決定 8 で、Cernere への依頼に SPEC-CR-membership-issuer と SPEC-CR-node-roster の 2 件が加わった (正本は security 文書 §4.34。依頼は Actio `actio:fd0e4de8` が運ぶ。鍵の公開・SSF/CAEP・step-up は `actio:2a7af0fa`)。本書はこの 2 件を定義せず、id と要約だけを持つ。両者に共通する規則: Cernere は `purpose: membership` と `purpose: roster` の専用 Ed25519 kid を JWKS に公開し (利用者 JWT の鍵とは分ける)、Synergos の golden vector の適合ケースを固定 commit で回し、変更ごとに SSF の `project-membership-changed`・`node-revoked`・`roster-changed` を出す。署名オブジェクトにメールアドレスを入れない。

| ID | 状態 | 依頼の記録 (根拠の発見) | 転送での使い方 | 提供前の暫定 |
|---|---|---|---|---|
| SPEC-CR-capability-signing-keys | Actio タスクとして依頼済み | タスク id 未確認。根拠 SEC-AZ-17 (失効が伝播しない), SEC-CR-12 (受領証・capability の独自署名), SEC-BR-04 (全組織共通の静的 admin token) | custody・edge・seed が検証する capability の発行鍵の根。委任記録の用途から `membership` を外し、custody-delegation・receipt・revocation (deny-only) だけを残す。`membership` と `roster` の kid も JWKS に載る | Vault に固定した発行鍵 (Excubitor Vault)。custody 用の鍵だけで、会員・名簿の鍵は Vault 固定にしない (security D-02 の縮小) |
| SPEC-CR-ssf-caep-transmitter | Actio タスクとして依頼済み | タスク id 未確認。根拠 SEC-AZ-17, SEC-BR-12 (失効の伝播と kill switch が無い) | 配信中の転送を ≤60 秒で止める失効入力。`project-membership-changed`・`node-revoked`・`roster-changed` を加え、`roster-changed` で PEP と directory が再取得する | 60 秒の会員資格ポーリング (既知の欠落として記録) |
| SPEC-CR-step-up-assertion | Actio タスクとして依頼済み | タスク id 未確認。根拠 SEC-DR-17, SEC-BR-16 (破壊操作に歯止めが無い) | 転送は直接使わない。最後の耐久写しを退避する placement 変更で参照。claim code の発行、役割・project scope の card 再発行、publish / history の付与、OfflineLease でも要る (Cernere 側で使う) | 2 人承認 (Curare 側文書 §4.1 の最後の `durable` 退避と同じ。Curare 側文書 §4.16 と一致) |
| SPEC-CR-membership-issuer | Actio タスクとして依頼済み (オーナー決定 8) | `actio:fd0e4de8` (SPEC-CR-membership-issuer と SPEC-CR-node-roster。鍵の公開・SSF/CAEP・step-up は `actio:2a7af0fa`)。決定 8: プロジェクト会員の正本は Cernere | Cernere 署名の会員証 (membership LCT) と ProjectRoster。directory の `roster` と、TXF2・SYNC・PLAN・RLY の本文前の検査の根拠。OfflineLease (ctx `project-roster`) で拠点の切断中も許可を延ばす | なし (Synergos に暫定を置かない)。T3 と security P3 の会員・enrollment 部分・P4 は Cernere を待つ。それまで拠点は現行 Rust を SECURITY-M34 (本書 M33) の設定で Tailscale 越しに運用する。どの Synergos 部品も会員・名簿の鍵を持たない |
| SPEC-CR-node-roster | Actio タスクとして依頼済み (オーナー決定 8) | `actio:fd0e4de8` (SPEC-CR-membership-issuer と SPEC-CR-node-roster。鍵の公開・SSF/CAEP・step-up は `actio:2a7af0fa`)。決定 8: ノード名簿の正本は Cernere | Cernere 署名の node card (役割 `topo`・project scope・宣言 endpoint) と RosterStatus。seed・relay・edge の候補は card の役割からだけ選ぶ。失効は RosterStatus から 10 秒以内に見え、PEP の 30 秒 poll と合わせて G1 (≤60 秒) を満たす | なし (同上) |

### 4.28 契約の変更と実装の非効率の区別 (`spec/plan/redesign-analysis/HANDOFF.md` §6)

| 区分 | 機構 |
|---|---|
| **互換契約の変更** (cpp-migration Compatibility contracts / `native-contracts.md` / Curare API) | M01, M03, M05 (IPC 部分), M06, M07, M09, M10, M12, M14, M15, M17, M18, M19, M20, M21, M26, M27, M28, M29, M30, M32、M16 (placement API) |
| **ネイティブ実装要件** (C++ が最初から避ける。契約は変えない) | M02, M04, M05 (索引の内部), M08, M11, M13 |
| **Curare の TypeScript 実装** (C++ 移行の対象外) | M22, M23, M24, M25 |
| **運用 / 設定** | M16 (split origins), M31, M33 |

ネイティブ実装要件の一覧 (`cpp-migration.md` に「native implementation requirements」として加える): 窓 ≥ BDP、1 バイトにつき読込 1 回と書込 1 回のパイプライン I/O (reflink の無い FS は作業コピー分の書込 1 回を足す。利用者が編集するファイルへのハードリンク禁止)、verify で再ハッシュしない、メタデータ保管のバッチ化、`Incoming` ごとに 1 task。

### 4.29 新設データ (RULE_DATA_SCHEMA)

| データ名 | 種類 | 権威ソース | 保存先 | 保護要否 | 保護方法 |
|---|---|---|---|---|---|
| ローカル object + `.bao` | 内容 | Sy ノード | `.synergos/objects` | 要 | 保管時の封筒暗号 (security)、ファイル権限 0600 |
| `index.db`, `state.db` | 派生 / live | Sy daemon | `.synergos/` (コミットしない) | 完全性 | SQLite WAL、fsync、manifest + objects から再構築可能 |
| 受信の `.part` + bitmap、custody upload の `.part` + hasher 状態 | 一時 | 受信者 / custody | ノードのディスク | 要 | capability に束縛、`Upload-Limit` の max-age 後に GC |
| Head | 署名文書 | publisher | メモリ + ディスクキャッシュ | 完全性 | security の署名封筒、単調 `seq` |
| node card、ProjectRoster、RosterStatus、OfflineLease | 署名文書 | Cernere (SPEC-CR-node-roster、SPEC-CR-membership-issuer)。control とノードは配布・キャッシュするだけ | Cernere (正本)。control と各ノードのメモリ + ディスクキャッシュ (`projects.json` の隣) | 完全性 | Cernere の `membership` / `roster` kid の署名 (ctx `node-card`・`project-roster`・`roster-status`)、`roster_epoch` の単調性、card と roster は `exp` ≤24 時間・RosterStatus は 300 秒 |
| Presence | 署名文書 | 各ノード (card が名指す node 鍵) | メモリ + directory のキャッシュ | 完全性 | ctx `presence` の署名、`card_hash` の一致、`exp` ≤10 分 |
| directory | 束ねたキャッシュ (未署名) | control (選択・順序だけ。中身の権威は上の 2 行) | control のメモリ、ノードのディスクキャッシュ | 完全性 | 要素ごとの署名検証、roster の `nodes[]` に無い card の破棄、`exp` = roster と card の最小 (≤24 時間) |
| チャンク pack、チャンク表 | 内容 + 索引 | chunk-store モジュール | seed / archive / custody / edge のボリューム、S3 | 要 | SPEC-SY-chunk-access、HMAC id、範囲ごとの DEK |
| `docs/performance.md` の予算とベースライン | 設定 / 計測 | Sy | repo | 不要 | Revisor 経由の変更だけ |
| custody の受領証の保管 (`GET /v2/receipts/{key}` の背後。複製が更新する `locations[]`・`durability` を含む) | 署名文書 | custody | custody の volume | 完全性 | security の SPEC-SY-custody-receipt-v2 の署名。確定後は追記だけ |
| v1 キーの索引 (`sha256(opaqueObjectId)` → content key) | 索引 | custody | custody DB | 完全性 | `custody.v1_compat` が on の間だけ保持、巻き戻し期間 (§5.3) の後に削除可 |
| `chunk_policy.toml` | 設定 | Sy | repo | 不要 | Revisor 経由の変更だけ |
| `spec/vectors/` の golden vector | 試験データ | Sy | repo | 不要 | Revisor 経由の変更だけ |
| custody の scrub ジョブの状態 (`jobId`、進捗、不一致) | 運用 | custody | custody DB | 不要 | 不一致は security イベントとして出す |
| Excubitor catalog の拠点 seed・relay・edge の項目 | 設定 | 運用担当 | `excubitor.catalog.yaml` (repo) | 不要 (秘密は書かない) | 秘密は Excubitor Vault の binding から注入 |
| `.synergos/.gitignore` | 設定 | Sy daemon | 作業ツリー | 不要 | daemon が起動時に再生成 |

秘密 (発行鍵、HMAC 鍵、DEK の KEK) は Excubitor Vault が正本で、本書の機構は値を設定ファイルに書かない。

### 4.30 ドメイン帰属 (`spec/domains/*.domain.json`)

| ドメイン | 機構 |
|---|---|
| network-transport | M06, M07, M08, M09, M11, M12、M26 の IPC codec (`network-transport.domain.json` は `synergos-ipc/`・`synergos-net/`・`synergos-relay/` を持つ。`cpp-migration.md:24` も net・relay・IPC を network-transport に置く) |
| mesh-core | M02, M03, M04, M05, M13, M30、M26 の job 管理 (`synergos-core/native/`) |
| opaque-custody | M14, M15, M16 (複製ワーカーと、複製の指示を受ける custody-admin の endpoint), M17 |
| control-plane | M10 (directory の配布・キャッシュと、Cernere への enrollment・会員証更新の中継。会員と名簿は決めない)。M16 の複製の起点は Curare から custody-admin への呼出で、synergos-control 側に trigger は無い |
| desktop-gui | D-T22 経由でのみ参照 (`synergos-gui` の `desktop/` workspace への移動と、C++ desktop-gui 受入後の退役は SECURITY-M30) |
| admin-ui | 変更なし (directory と配置の表示は後続) |
| ars-plugin | 変更なし (IPC v2 の利用者として `client_kind: plugin`) |
| **chunk-store (新設案)** | M27, M28 (計算側), M29 (表) |

作業前の割当案は M26 を mesh-core だけに、M16 の複製 trigger を control-plane に置いていた。上の表のとおり改める (M26 は codec と job 管理で 2 つに分かれ、M16 には control 側の trigger が無い)。Curare 側文書が Synergos のドメインに触れる場合も本表に従う。

新ドメイン案 (REVIEW_DESIGN §0): `chunk-store` — 「常時稼働の役割が使う内容定義チャンク保管。チャンク化・pack・重複排除・GC・全体書き出しを持ち、同一性と版は持たない」。`spec/domains/chunk-store.domain.json` を T5 の着手時に追加し、メンバー (通常ノード) の経路からは参照されないことを Anatomia の依存規則で検査する。

### 4.31 `cpp-migration.md` "Compatibility contracts" 改訂案 (TRANSFER-M32)

- 区分: 互換契約の変更。対象発見: TRN-DD-22, TRN-WP-21, TRN-TP-16 (AGD-AI-20 も同じ凍結を指摘する)。フェーズ T0 (neco が Q1・Q2 に答えた後、セキュリティ側と ai-game 側の改訂と合わせて 1 回で編集)。
- 目的: C++ 移行の互換契約が現状の欠陥 (CRC だけの同一性、版なしの wire、全体再送の custody) を凍結しないようにし、移植後の 2 回目の flag day を無くす (TRN-DD-22)。
- 仕組み (改訂の行):

| 現在の行 | 改訂案 |
|---|---|
| `:39-40` 「Peer protocol: preserve protobuf schemas, QUIC ALPN, peer authentication, CRC/hash validation, transfer cancellation, and catalog exchange.」 | 「Peer protocol: ALPN `synergos/2` per SPEC-SY-peer-wire (MessagePack frames, versioned stream magics, capability bits), session per SPEC-SY-hlo2-session-binding, transfer per SPEC-SY-txf2, replication per SPEC-SY-replication-stream, content identity per SPEC-SY-content-identity. TXFR v1, DHT1, BSW1 and FileWant are frozen for golden-vector interop only.」 |
| `:41-43` 「Versioning: preserve `.synergos/manifest.json`, local high-water marks, …」 | 先頭に「manifest format 2 per SPEC-SY-manifest-v2; readers accept formats 1 and 2」を加える。「local high-water marks」は保つ (置き場は `.synergos/state.db`、単調で node-local の意味は同じ。§3.20 変更提案 8) |
| `:44-47` 「Custody: SPEC-SY-opaque-custody, SPEC-SY-custody-durability, SPEC-SY-custody-acquisition remain normative. Preserve canonical receipt bytes, HMAC, …」 | セキュリティ側の受領証・capability の改訂に並べて「byte path per SPEC-SY-custody-transfer-v2 (resumable upload, Range GET via SPEC-SY-object-get, receipt produced once, O(1) verify), batch admission per SPEC-SY-custody-batch-admission, replication per SPEC-SY-custody-replication」を加える。SPEC-SY-custody-durability は改訂後の文面 (§4.32、§3.20 変更提案 6) を normative とする |
| `:48` 「Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces.」 | 本書からは変えない。受入面の「native GUI」を C++ desktop-gui に改める変更提案は security 文書が持ち、本書はそれに従う |
| (新規) Topology | 「Topology: SPEC-SY-node-roles inside the Cernere-signed SPEC-SY-node-card (SPEC-CR-node-roster), SPEC-SY-directory (distributed by control, never signed by it), SPEC-SY-relay-v2 (C++ only). One binary; roles are per-process modes.」。注記: card・ProjectRoster・RosterStatus の署名者は Cernere だけで、C++ の synergos-control は会員・名簿の発行鍵を持たない (オーナー決定 8)。relay は C++ 版でだけ配備し、それまでの到達は Tailscale (オーナー決定 7) |
| (新規) Chunk store | 「Chunk store: SPEC-SY-chunk-store and SPEC-SY-range-plan; linked only by seed, archive, custody (out-of-process) and edge; SPEC-SY-format-policy.」 |
| `:35-37` IPC | 「result streaming and jobs per SPEC-SY-ipc-v2」を加える |
| (新規) Native implementation requirements | §4.28 の 5 項目と、利用者が編集するファイルへのハードリンク禁止 (§4.2) |
| `:64-71` Completion gate | 「parity evidence」に「SPEC-SY-transfer-budget の値が記録済み Rust ベースライン以上」を加える。旧挙動との parity ではなく新契約への適合を合格条件とする項目 (TXF2、受領証 1 回生成) を明記する。flag day 後の巻き戻し手順 (§5.3) の試験を合格条件に加える |

- 不変条件: 改訂は 1 回の編集で、セキュリティ側と ai-game 側の改訂と同じ提出にまとめる。`cpp-migration.md` と `native-contracts.md` に現れる契約 ID は本書・Curare 側文書・security 文書の ID と同じ綴りにする。
- 失敗時: neco が Q1 か Q2 を却下した場合は、該当行 (Chunk store、BLAKE3 同一性) を除いて提出し、残りの行は変えない。
- 変更対象: `spec/plan/cpp-migration.md`、`spec/feature/native-contracts.md` (§4.32)。
- 担保手段: ローカル検証ゲート (`cpp-migration.md` の Compatibility contracts と `native-contracts.md` の `[id:]` 節に現れる契約 ID が本書の契約 ID 一覧 (§4.32) と一致することを Revisor ケースで検査) + 記憶。

### 4.32 `native-contracts.md` へ追加する `[id:]` 節

| 新節 | 種別 | 主な検査 (CTest / golden vector) |
|---|---|---|
| `[id: SPEC-SY-content-identity]` | W | BLAKE3 / Bao / SHA-256 / CRC32 の vector、単一パス |
| `[id: SPEC-SY-manifest-v2]` | W | format 1 / 2 の往復、決定的直列化、`manifest_root` |
| `[id: SPEC-SY-local-index]` | A | racy 規則、デバイス変更で無効化 |
| `[id: SPEC-SY-peer-wire]` | W | 全 magic と版、`0x5304` |
| `[id: SPEC-SY-txf2]` | W | 全 frame の vector、Bao 範囲検証、再開 |
| `[id: SPEC-SY-replication-stream]` | W | Head 署名、HaveSet の IBLT / bloom |
| `[id: SPEC-SY-node-roles]` | A | 禁止された同居で起動失敗。card の `roles` を超える modes で起動失敗 |
| `[id: SPEC-SY-directory]` | A | 要素ごとの Cernere / node 署名の検証、roster 外の card の破棄、`exp`、etag |
| `[id: SPEC-SY-relay-v2]` | W | frame、STUN demux、背圧で破棄 |
| `[id: SPEC-SY-custody-transfer-v2]` | W | offset `409`、hasher 状態の再開、受領証 1 回 |
| `[id: SPEC-SY-object-get]` | W | `206`、`If-Range`、`.bao` スライス |
| `[id: SPEC-SY-custody-batch-admission]` | W | 1 万件、冪等 |
| `[id: SPEC-SY-custody-replication]` | A | `locations[]` 追記、削除しない |
| `[id: SPEC-SY-ipc-v2]` | W | `ResponseChunk`、job |
| `[id: SPEC-SY-chunk-store]` | A | chunker vector、pack 形式、GC |
| `[id: SPEC-SY-range-plan]` | W | plan の再構成が全体 BLAKE3 と一致 |
| `[id: SPEC-SY-format-policy]` | A | 表の解決、hook の判定 |
| `[id: SPEC-SY-transfer-budget]` | A | (計測ケースの定義。CTest label `bench`) |

既存節の改訂は 3 件である。`[id: SPEC-SY-native-metadata]` は「Unchanged size/CRC retains its existing version」を「Unchanged means equal BLAKE3; size/CRC applies only to format-1 entries without `blake3`」に改める。`[id: SPEC-SY-native-framing]` には「total result size is unbounded via SPEC-SY-ipc-v2 chunking」を加える。`[id: SPEC-SY-custody-durability]` (`spec/feature/opaque-custody.md:24-28`) は「receipt発行時にも実ファイルを再検証しsync_all成功を要求する」を「受領証は commit 時に 1 回だけ発行し、根拠は PUT 中に計算したダイジェストと fdatasync・ディレクトリ fsync の成功とする。実ファイルの全量再検証は非同期の scrub が行う」に改める (§3.20 変更提案 6)。

### 4.33 crate 別の変更マップ

「現行 Rust」の列は、オーナー決定 6 に従い計測ハーネスと設定以外を変更しないことを示す。

| crate / 面 | ドメイン | C++ 側で入る機構 | 置き換わる Rust の部分 | 現行 Rust への変更 |
|---|---|---|---|---|
| synergos-net (QUIC、gossip、DHT、bitswap、transfer、conduit、promotion、relay client) | network-transport | M06, M07, M08, M09, M11, M12 (client), M28 (frame) | `quic/mod.rs`、`transfer/mod.rs`、`gossip/`、`dht/` (移植しない)、`content/` (移植しない)、`conduit/` (移植しない)、`promotion.rs` | なし |
| synergos-core (daemon、exchange、manifest、history、checkout、ipc_server) | mesh-core | M01, M02, M03, M04, M05, M07 (受信状態), M09 (wanted-set), M10 (card キャッシュ), M13, M26 (job), M28 (適用), M29 (hook), M30 | `exchange/mod.rs`、`manifest.rs`、`project.rs`、`history/`、`checkout.rs`、`restore.rs`、`worktree_status.rs`、`catalog_sync.rs` | 試験専用の計測ハーネス (M31) と運用文書 (M33) だけ |
| synergos-control | control-plane | M10 (directory endpoint、heartbeat 応答、Cernere への enrollment・会員証更新の中継。会員・名簿の発行鍵は持たない) | `api/mod.rs` (node 向け API が無い) | なし |
| synergos-custody | opaque-custody | M01 (digest)、M14 (upload resource・range GET・receipt store・v1 キー索引)、M15、M16 (複製)、M17 (edge mode) | `handlers.rs`、`store.rs`、`signature.rs` (`object_key` の `sha256(opaqueObjectId)` は v1 キー索引として残る) | なし |
| synergos-relay | network-transport | M12 (C++ だけ) | `server.rs` (room 放送) | なし。Rust の relay は改修も配備もしない (オーナー決定 7、SECURITY-M34)。C++ relay が受入ゲートを通るまで到達は Tailscale |
| synergos-ipc | network-transport | M26 (codec。job 管理は synergos-core 側で mesh-core) | `command.rs`、`response.rs` (`transport.rs` の 1 MiB フレームは維持) | なし |
| synergos-chunkstore (新設) | chunk-store (新設案) | M27, M28 (計算), M29 (表の利用) | DESIGN §15.1 の chunker / DAG (superseded) | — |
| synergos-gui (egui) | desktop-gui | なし (C++ desktop-gui に置き換わる) | — | P0 で `desktop/` workspace へ移し、eframe の `accesskit` feature を外して quick-xml 0.30 の経路を消す。egui 本体の退役は C++ desktop-gui の受入後 (security D-13 / SECURITY-M30)。`cpp-migration.md:48` の受入面の変更提案は security 文書が持つ。転送機能の損失なし (§3.18) |
| synergos-tauri | desktop-gui | M26 の `Subscribe` を使う | ポーリング | なし |
| synergos-admin-ui | admin-ui | 後続 (directory・配置の表示) | — | なし |
| ars-plugin-synergos | ars-plugin | M26 (利用側) | — | なし |
| `excubitor.catalog.yaml`、`scripts/site/*` | (運用) | M13 (拠点 seed)、M14 (custody の上限)、M16 (admin / transfer の origin、CORS) | — | T0 で origin の記述を Curare の split origins に合わせ (`:24-29`)、`SYNERGOS_CUSTODY_CLIENT_ORIGINS` (`:33`) を拠点ごとに設定。T2 で `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` (`:31-32`) を 1 TiB の天井に上げる |
| `docs/performance.md`、`spec/vectors/` | (運用) | M01, M31 | — | T0 で新設 |

## 5. 実装ステップ・影響範囲・ロールバック方針

### 5.1 フェーズ表

フェーズ名は security の P0〜P6 と区別するため T0〜T5。変更はすべて Revisor の local PR で取り込む (オーナー決定 2)。migration は追加のみで DROP 禁止 (ゲートで検査)。

| フェーズ | 内容 | 対象 crate / module | 依存 (security フェーズ) | 検証方法 (試験種別) | ロールバック |
|---|---|---|---|---|---|
| **T0** 契約・ベースライン・即効策 | 本書と Curare 文書。§4 の全 W / A 契約の本文。content identity・manifest v2・peer-wire frame の golden vector。計測ハーネス (M31) で Rust v1 のベースライン記録 (試験専用)。catalog の split origins 記述と `SYNERGOS_CUSTODY_CLIENT_ORIGINS` の拠点別設定 (M16)。Rust の運用文書 (M33)。M32 の改訂案はここで起草し、neco が Q1 / Q2 に答えた後にセキュリティの改訂と一緒に提出 | `spec/`、`docs/performance.md`、`spec/vectors/`、`synergos-core/tests/` (bench)、`excubitor.catalog.yaml` | なし (security P0 と並行) | vector の自己試験、ベースライン記録、admin / transfer の origin を分けたブラウザーと CLI の upload / download と CORS の統合試験 (§6.2 試験 7) | 文書の revert、設定の revert |
| **T1** Curare のフロー | Curare 文書の M18〜M25、M29 の報告モード。Synergos は v1 facade での custody 受付 (サーバ側 1 件ずつ) を提供するだけで、コード変更なし | (Curare) | T0、security P1 (LCT codec)・P2 (台帳・セッション検証) | Curare 側の CG ケース | Curare のフラグ |
| **T2** C++ ネイティブ中核 | M01, M02, M03, M04, M05, M06, M07, M08, M14, M15, M26 を C++ 移植に入れる (cpp-migration 手順 3〜4)。custody 部分は security P3 と、peer 部分は security P4 と、**組織ごとに 1 回の `synergos/2` flag day** で切り替える。custody v2 の有効化と同時に catalog の `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` を 1 TiB に上げる (M14)。Synergos の pre-publish hook (M29) は報告モードで入る | `synergos-net/native/`、`synergos-core/native/`、`synergos-custody/native/`、`synergos-ipc/native/` | T0 の契約、security P3 (custody の資格)・P4 (HLO2、会員資格。P3 の会員・enrollment 部分と P4 は Cernere の SPEC-CR-membership-issuer / SPEC-CR-node-roster を待つ、オーナー決定 8) | TG-01〜TG-06、TG-12〜TG-14 の計測。Rust ベースラインとの比較。100 GB custody upload / 再開。Bao 破損グループ試験。flag day 後の巻き戻し試験 (§6.2 試験 29) | flag day 前: その組織はゲートを通るまで Rust 実行時に留まる (security P4 のフォールバック)。flag day 後: §5.3 の巻き戻し手順 (巻き戻し期間の間だけ)。`custody.v1_compat` で Curare 向けの v1 PUT と v1 キー索引を保つ |
| **T3** トポロジ | M09, M10, M11, M12, M13。拠点 seed を Excubitor catalog で配備 | `synergos-net/native/`、`synergos-control/native/`、`synergos-relay/native/`、`synergos-core/native/` | T2、security の SPEC-SY-membership-token と -revocation-bundle が稼働。Cernere の SPEC-CR-membership-issuer と SPEC-CR-node-roster が稼働 (オーナー決定 8。Synergos 側の暫定は置かず、それまで拠点は現行 Rust を M33 / SECURITY-M34 の設定で Tailscale 越しに運用)。C++ relay だけ (オーナー決定 7) | TG-08〜TG-11。星形伝播 (ハブ 1 + スポーク 3)。network namespace の NAT 行列。relay 停止から ≤10 秒の移行。directory 期限切れ。roster 外・非 Cernere 署名の card の破棄 | 役割をプロセスごとに設定で無効化 |
| **T4** 配置と edge | M16 の custody 複製、M17 の拠点 / egress edge、M30 の `sync_mode` | `synergos-custody/native/`、`synergos-core/native/` | T3、SPEC-CR-capability-signing-keys (暫定は Vault 固定鍵)、egress edge は neco の Q3 | egress edge からの 100 GB clone の kill / resume。ベンダーの magic link 経路。rf 2 未満で確定が止まる。失効で edge の範囲応答が ≤60 秒で止まる | Curare が origin の location に戻す。edge を無効化 |
| **T5** チャンク層 | M27 (`measure` から)、M28、M29 の強制 (チャンクプロジェクトだけ) | `synergos-chunkstore/`、seed / archive / custody / edge への結合 | T4、neco の Q1、security の SPEC-SY-chunk-access と封筒暗号 (security P5)、スタジオの計測コーパス (Q6) | chunker vector。retention lock 下の GC mark / sweep の性質試験。コーパスでの TG-07。暗号化の有無で重複排除率が同じ (security E5) | `project.chunking = off`。`export` で全体を再構築。全体の TXF2 は常にある |

### 5.2 互換性と移行

- peer プロトコルの切替は組織ごとに **1 回**の flag day (`synergos/1` → `synergos/2`)。security と共有する。組織内で版の混在した mesh は作らない。security が `synergos/1` を拒否するため (HLO1 は再生可能)、転送側にも v1 の互換窓を置かない。flag day の前に戻す時は「その組織を Rust 実行時に留める」で、プロトコルのフォールバックはしない。flag day の後に戻す時は §5.3。
- manifest format 1 は永久に読める。format 2 の書込は flag day 以降だけ (Rust の読み手は format 2 を拒否する、`synergos-core/src/manifest.rs:138-141`)。書き手は format 2 の項目でも `crc` を埋め続ける。
- Curare の v1 API は v2 の facade としてクライアントの移行まで残る。削除は別の判断。
- custody の v1 単発 PUT は Curare の storage port v2 が唯一の呼び手になるまで `custody.v1_compat` の下に残る。その間 C++ custody は v1 キーの索引 (`sha256(opaqueObjectId)` → content key) を保つ (§4.14)。
- 既存の履歴ノードの objects: 現行の保管は `<history root>/<blake3(project_id)>/objects/<hh>/<blake3>` に `.meta.json` の sidecar と `index.json` を持つ (`docs/versioning-design.md:130-131`, `:149`、`synergos-core/src/history/store.rs:3`)。C++ の object store は `.synergos/objects/<b3[0..2]>/<b3>` と `.bao` で、root も sidecar も違い、既存の object には Bao outboard が無い。キーが同じ BLAKE3 であることだけが共通なので、次の手順で移す。
  1. C++ daemon は初回起動時に、プロジェクトごとの既存履歴 root を読取専用の第 2 の object 置き場としてマウントする (同じ BLAKE3 で引ける)。同じ volume ならハードリンクで `.synergos/objects` に取り込んでよい (どちらも Synergos だけが書き、利用者が編集しないファイル。§4.2)。
  2. `index.json` と `.meta.json` の refs を `state.db` の `history` 表へ取り込む。元の JSON と objects は消さない。
  3. Bao outboard は最初の範囲配信の時に遅延生成する。全体配信だけなら作らない。
  4. チャンク層を on にしたプロジェクトだけ、archive が新しい版から順にチャンク保管へ移る。旧版の移し替えは背景ジョブで行い、`export` で元に戻せる。
  移行試験 (§6.2 試験 30) は旧配置と新配置から読んだ全 object を byte 単位で比べる。
- 既存の `.synergos/manifest.json` / `state.json` から `state.db` と `index.db` を初回起動時に作る。観測高水位は max(`state.json`, manifest の版) で作る。元の JSON は消さない。
- 新しいローカル成果物 (`.synergos/objects/`、`index.db*`、`state.db*`、`incoming/`) は daemon が管理する `.synergos/.gitignore` で ignore する (§4.4)。受信後の `git status` に出ないことをゲートで確かめる (§6.2 試験 28)。

### 5.3 flag day 後の巻き戻し

flag day の前は「その組織を Rust 実行時に留める」で足りる。flag day の後は C++ だけが書いたデータが残り、Rust は次を読めない。

| C++ が書くもの | Rust で起きること |
|---|---|
| manifest format 2 | 読込で拒否する (`synergos-core/src/manifest.rs:138-141`) |
| `state.db` の観測高水位 | 残った `state.json` は flag day 時点で止まる。そこから発番すると使用済みの版番号を再利用し、同版別内容の衝突になる (`docs/versioning-design.md:187-191` の規則が破れる) |
| `.synergos/objects/` の配置 | Rust の履歴は `<history root>/<blake3(project_id)>/objects/` だけを見る |
| custody の内容由来キー `HMAC(project_key, blake3)` | Rust custody は `sha256(opaqueObjectId)` で引く (`synergos-custody/src/signature.rs:5-7`) |

巻き戻しを可能にするため、巻き戻し期間 (既定: flag day から 30 日、設定 `rollback.window_days = 30`) の間は次を守る。

1. manifest: 書き手は format 2 の項目でも `crc` を埋め続ける。`synergos manifest downgrade` は format 2 を format 1 に書き直す (`blake3`・`chunk_map_id`・`digests[]`・`manifest_root` を落とす。`crc` が欠けた項目は object から計算する)。
2. 観測高水位: `state.db` は snapshot のたびに `state.json` へ高水位を書き出す。downgrade ツールも `state.db` から `state.json` を作り直す。
3. 履歴: downgrade ツールは期間中に書かれた objects を Rust の履歴 root (`objects/<hh>/<blake3>`、`.meta.json`、`index.json`) へハードリンクかコピーで書き出す (どちらも Synergos だけが書くファイル)。
4. custody: `custody.v1_compat` が on の間、C++ custody は v1 キーの索引を保つ。巻き戻し時は `synergos custody export-v1` が v1 キーの位置に実体を書き出し、v1 の受領証 (HMAC) を作り直して Rust custody が読めるようにする。v2 だけで受け付けて v1 キーを持たない実体は一覧に出し、Curare が v1 で再受付する。
5. 期間の長さと終了の判断は neco (Q22)。期間が終わると 1〜4 の書き出しを止め、以後の巻き戻しはできない。

受入試験 (§6.2 試験 29): flag day → 1 日の書込 → Rust へ巻き戻し。全 path の版番号が単調で、巻き戻し後の publish が既存の版番号と衝突しない。期間中に書かれた全 object と custody 実体が Rust から読める。

## 6. 検証計画

### 6.1 性能の受入条件 (計測対象・母集団・目標値)

すべて Revisor のマージ前ケース (短いプロファイル) か夜間ケース (長いプロファイル)。GitHub CI は使わない。

| ゴール | 計測対象 | 母集団 | プロファイル | 目標値 | ケース種別 |
|---|---|---|---|---|---|
| TG-01 | 単一実体の到達 throughput | 4 GiB の擬似乱数 1 本、送信元 1、5 回の中央値 | `wan100` | ≥800 Mbps | 短 |
| TG-02 | 同上 | 4 GiB、送信元 4 / BBR 有効、5 回 | `wan100-loss` | ≥300 Mbps (T0 後に再設定) | 夜間 |
| TG-03 | LAN throughput と経路 | 4 GiB 1 本、同一拠点の組 | `lan` | ≥5 Gbps、overlay 経由 0 回 | 短 |
| TG-04a | ディスク増幅 (reflink 可能) | 4 GiB 1 本の受信と publish、`/proc/self/io` か ETW。Btrfs・XFS・APFS・ReFS | `lan` | 受信書込 ≤1.1×、余分な全量読込 0、publish 読込 ≤1.05× | 短 |
| TG-04b | ディスク増幅 (reflink なし) | 同上。ReFS でない NTFS と ext4 | `lan` | 受信書込 ≤2.05×、余分な全量読込 0、publish の読込 ≤1.05× + 書込 ≤1.05× | 短 |
| TG-05 | 再開時の再送率 | 4 GiB を 50 % で kill。経路は TG-05 の 4 つ (P2P の TXF2 / custody upload / custody・edge・seed の範囲 GET / Curare client の download)。MinIO の S3 multipart は Curare 側文書 §6 | `wan30` | ≤55 % | 短 |
| TG-06 | 100 GB の完走 | 100 GB の sparse fixture 1 本: upload → 受領証 → 版作成 → clone | `wan30` | 完走。wall-clock 上限による失敗 0 | 夜間 |
| TG-07 | 500 MB PSD の再送率 | スタジオコーパスの PSD 系列 (Q6)。レイヤ ≤25 % 変更の連続保存 20 回 | `wan100` | チャンクプロジェクトで ≤35 % (暫定) | 夜間 (T5 から) |
| TG-08 | publisher の送出量 | 100 MB × 50、seed 1・メンバー 20 の拠点 | namespace 拠点モデル | 拠点あたり ≤1.2×、保持者 3 で受信 ≤1.05× | 短 |
| TG-09 | 伝播時間 | 星形 (ハブ 1 + スポーク 3) と拠点モデル、publish 100 回 | `wan30` | p95 ≤60 秒、フレーム 1 個損失で最悪 ≤90 秒 | 短 |
| TG-10 | 中継 | RelayOnly の組 10、relay 停止 1 回 | `wan30` + NAT 行列 | データ経路 100 %、throughput ≥80 %、移行 ≤10 秒 | 短 |
| TG-11 | 再接続の嵐 | 模擬クライアント 500 が同時再接続 | `lan` | 10 秒以内に全受入 | 夜間 |
| TG-12 | 準備完了 | 10 GB / 20 万ファイルのプロジェクト | `lan` | ≤5 秒、`status` の読込 0 | 短 |
| TG-13 | メタデータ書込量 | 10^5 ファイルの checkout | `lan` | ≤最終 manifest サイズの 2× | 短 |
| TG-14 | offload のメモリ | 100 GB の sparse fixture 1 本を S3 互換の保管へ offload | `lan` | daemon の最大 RSS ≤256 MiB | 夜間 |
| CG-06 | IPC | 10^6 項目の `Status`、checkout 開始 | ローカル | フレーム ≤1 MiB、`JobStarted` ≤100 ms | 短 |

### 6.2 機能の受入条件 (Synergos が正本のケース)

性能の受入条件は §6.1 の表である。本表の番号は 1 から振り、§4 の各節はこの番号で試験を指す。

| # | ケース | 合格条件 | 発見 / 機構 |
|---|---|---|---|
| 1 | 再開: TG-05 の 4 経路 (P2P の TXF2、custody upload、custody・edge・seed の範囲 GET、Curare client の download) で 50 % kill | 再送 ≤55 %。MinIO の S3 multipart は Curare 側文書の試験 | TRN-WP-04, TRN-DD-03, TRN-DD-09 / M07, M14 |
| 2 | 1 つの Bao グループを壊した送信元 | 16 KiB 以内で中止し、別の送信元から完了 | TRN-WP-08, TRN-DD-04 / M07 |
| 3 | 遅い回線での大きな restore | 転送中に pin が失効しない。完了後に stale として消されない | TRN-WP-05 / M07 |
| 4 | 失敗後に同じ (project, file, version) を再試行 | バイトが届く | TRN-WP-18 / M07, M09 |
| 5 | 保持者 3・受信者 1 | 受信の流入 ≤1.05×、重複した全量 push 0 | TRN-WP-19, TRN-TP-02 / M07 |
| 6 | 星形でスポークが publish | 他の全スポークへ p95 ≤60 秒。daemon 再起動後も同じ | TRN-TP-01, TRN-TP-12 / M09, M10 |
| 7 | admin と transfer の origin を分けた構成 | ブラウザーと CLI の upload と download が成功し、ブラウザーの要求が CORS を通る (`SYNERGOS_CUSTODY_CLIENT_ORIGINS` に Curare の Web origin がある) | TRN-TP-07 / M16 |
| 8 | メンバーの失効 | その TXF2・relay セッション・custody / edge の範囲応答が ≤60 秒で閉じる (security 試験 10 と照合) | security G1 / M07, M12, M17 |
| 9 | 1 万実体の clone の認可 | Curare 呼出 ≤10 回、custody admin 呼出 0 回 (security 試験 22a と照合) | TRN-E2-05 / M14, M17 |
| 10 | 適合 vector | Rust v1 encoder と C++ で content identity・manifest v2・peer-wire frame・TXF2・Head・chunker 切れ目が一致 | TRN-DD-22, TRN-WP-21 / M01, M03, M06, M32 |
| 11 | range plan | 再構成した実体が全体 BLAKE3 と一致。わざと誤った plan で全体 pull に落ちる | TRN-WP-12 / M28 |
| 12 | 形式 hook | チャンクプロジェクトの deflate FBX は preset を示すメッセージで拒否。報告モードでは数えるだけ | TRN-DD-17 / M29 |
| 13 | ディスク計数 | TG-04a / TG-04b の値を `/proc/self/io` か ETW で満たす。reflink の無い FS でもハードリンクが作られない (作業ツリーのファイルの link count が 1) | TRN-WP-07 / M02 |
| 14 | 禁止された同居 | custody-admin と relay を同じプロセスで起動すると起動失敗 | D-T10 / M10 |
| 15 | directory の期限切れと改ざん | `exp` 後は新規セッションを張らず、既存の認証済みセッションだけ保つ。ProjectRoster の `nodes[]` に無い card、Cernere 署名でない card・roster、card と `card_hash` が合わない Presence を directory に混ぜると、ノードはそれを捨てて接続しない (security AT-SY-SEC-41 と照合) | TRN-TP-14 / M10 |
| 16 | format 2 と Rust | flag day 前の組織で format 2 の manifest を書く操作がゲートで拒否される | TRN-DD-05 / M03 |
| 17 | チャンク GC | retention lock 下の mark / sweep の性質試験で、lock された版のチャンクが消えない | TRN-DD-07 / M27 |
| 18 | publish 後の in-place 編集 | publish の後に作業ファイルを in-place で書き換える (DCC ツールの上書き保存を模擬)。object の BLAKE3 が変わらず、旧版の restore が元のバイトを返す | `docs/versioning-design.md:156-157` / M02 |
| 19 | 送信中の編集 | 4 GiB の送信中に送信側の作業ファイルを書き換える。受信側は publish された BLAKE3 の内容を得るか、何も得ない (未 publish のバイトが届かない) | TRN-WP-20 / M02 |
| 20 | 手元にある内容の offer | 手元の object と同じ BLAKE3 の offer (rename を含む) を wire 0 バイトで満たし、作業ツリーへ reflink かコピーで置く | TRN-DD-02, AGD-DM-06 / M02 |
| 21 | 大きな offload のメモリ | 100 GB の offload で daemon の最大 RSS ≤256 MiB (TG-14) | TRN-DD-08 / M13 |
| 22 | O(1) verify | `verify` と `GET /v2/receipts/{key}` の間のデータ読込が 0 バイト (`/proc/self/io` の `read_bytes`) | TRN-DD-10, TRN-E2-02 / M14 |
| 23 | 一括受付 | 10 000 件の受付が 1 回の呼出で返る。同じ `operationId` の再送で状態が変わらない | TRN-E2-01, TRN-DD-11 / M15 |
| 24 | `sync_mode` Selective | pattern の外の実体の pull が 0 件。Manual では明示要求の外を取らない | TRN-DD-18, TRN-E2-16 / M30 |
| 25 | custody の複製 | 複製の commit が受領証の `locations[]` に追記し、`durability` を更新する。複製の失敗でも削除が起きない | TRN-TP-06 / M16 |
| 26 | NAT 行列 (full-cone、symmetric、CGNAT) | 状態表示の経路が実際の経路と一致する。中継していないのに Relay と表示されない | TRN-TP-03, TRN-TP-04 / M11, M12 |
| 27 | チャンク保管の認可と GC の主体 | 集合を覆う capability の無い `get_range` が拒否される。GC が呼び手と別の OS ユーザー・Vault policy で動き、プロセス内の `gc` 呼出は本番設定で拒否される | SPEC-SY-chunk-access / M27 |
| 28 | ローカル成果物の ignore | 受信の後の `git status` に `.synergos/objects/`・`index.db*`・`state.db*`・`incoming/` が出ない | §3.20 変更提案 9 / M04 |
| 29 | flag day 後の巻き戻し | flag day → 1 日の書込 → Rust へ巻き戻し。版番号が単調で衝突せず、全 object と custody 実体が Rust から読める | §5.3 / M03, M04, M14 |
| 30 | 旧履歴の移行 | 旧配置 (`<history root>/<blake3(project_id)>/objects/`) と新配置から読んだ全 object が byte 単位で一致する | §5.2 / M02, M04 |
| 31 | HaveSet の符号化 | IBLT だけの実装と IBLT + bloom の実装が相互に照合できる。bloom は両端の caps にある時だけ使われる | M09 |

Curare が正本のケース (CG-01〜CG-05、changeset、fetch session、MinIO multipart) は Curare 側: spec/plan/redesign-transfer-efficiency.md §6。

### 6.3 計測環境

Linux ランナー上の netem (`lan`、`wan30`、`wan100`、`wan100-loss`) と network namespace で、seed 1・メンバー 20・NAT 越しの在宅 1・relay 1・egress edge 1 の拠点を模擬する。100 GB の夜間プロファイルの実行場所は Q6 で決める。Windows 上の実行は Linux / macOS の実行時 parity の根拠にならない (`cpp-migration.md:69`「Windows execution cannot establish Linux/macOS runtime parity.」と同じ)。

## 7. 未決事項と判断者

Q 番号は Curare 側文書と共通に振る。Q1〜Q10 は両文書で同じ問い。Q11〜Q17 は Curare 側文書だけで扱う。Q7・Q9・Q18〜Q22 は本書だけで扱う。

| Q | 問い | 判断者 | 本書の既定 |
|---|---|---|---|
| Q1 | 2026-08-16 決定の部分変更を認めるか: seed / archive / custody / edge に限った独立チャンク保管モジュール (D-T02, D-T03) と、メンバーの wire が変更範囲だけを運ぶ range plan (D-T04)。同一性と展開はファイル全体のまま | neco | 変更提案として記述。T5 は回答後に着手 |
| Q2 | BLAKE3 を repo 横断の正準同一性にするか (D-T01)。Curare `versioning-types.md:59` と `SPEC-SY-native-metadata` の size / CRC 規則に対する変更 | neco | 変更提案。Curare は SHA-256 の列を残す |
| Q3 | Curare INV-1 の例外として HQ 外のインターネット向け egress edge を置くか (D-T16) | neco | 回答までは拠点 edge だけ。ベンダーは現行経路 |
| Q4 | オーナー決定 6 を転送とトポロジの全プロトコル機能へ広げるか (D-T19)。別に、P0 級のデータ損失 2 件 (TRN-WP-05, TRN-WP-18) と偽の `Ok(Relay)` 状態に、C++ 切替前の最小限の Rust 修正を許すか | neco | Rust に新機能なし。3 件は運用文書だけ |
| Q5 | 重複排除範囲: 既定はプロジェクト、組織単位は組織ごとの opt-in (security Q6 と共通) | neco | プロジェクトのみ |
| Q6 | スタジオの実 depot 計測コーパス (PSD / FBX / uasset / EXR / チェックポイント系列) と、夜間 100 GB 計測の実行場所 | 運用担当 | 未確認 (コーパスの提供元が決まっていない)。T5 の `on` は入手まで不可 |
| Q7 | 拠点 seed と relay のホスティング: オフィスごとに seed + relay 1 台、地域ごとにインターネット relay 1 台を Excubitor で運用するか | 運用担当 | 提案のまま。relay は C++ 版でだけ配備する (オーナー決定 7)。それまでは Tailscale |
| Q8 | 追補 X1〜X5 (§4.26) をセキュリティの契約に取り込むか | セキュリティ担当 | **解決済み**: security §4.0.2 で受入済み。残る「条件付き」の文言の整理を security 文書の担当へ依頼 (§4.26) |
| Q9 | T0 ベースライン後の TG-02 (損失あり WAN) の目標値と、BBR を既定にする切替 | neco | 暫定 300 Mbps、既定 Cubic |
| Q10 | collection 登録で使う `project_ref` ↔ Curare space の対応 (共通識別子) | neco (ai-game 文書) | 参照のみ (security Q8) |
| Q18 | 保留ハンドシェイク上限の既定値: 本書 M08 の 256 とセキュリティ NR-1 の 64 のどちらに揃えるか (TG-11 の 500 クライアント / 10 秒と攻撃耐性の釣り合い) | セキュリティ担当 | T2 の計測で TG-11 を満たす最小値を採り、セキュリティ文書に合わせて 1 つの値にする |
| Q19 | SPEC-SY-chunk-store の ctx に `cap` (検証済み LCT か principal handle) を `get_range`・`chunk_map`・`has` について足し、`get_range` ごとの `content.served` の義務を足すか (SPEC-SY-chunk-access との整合、§4.19) | セキュリティ担当 (契約の登録簿の所有者として neco が確定) | 回答まで実装は SPEC-SY-chunk-access の文面 (全呼出に LCT) に従う |
| Q20 | SPEC-SY-replication-stream の HaveSet 符号化: IBLT 必須・bloom 任意とし、SPEC-SY-peer-wire の `caps` に bloom 対応 bit (案 `haveset_bloom_v1`) を足す文言 (§4.9) | neco (契約の登録簿の所有者) | 本書の M09 の規則 (IBLT 必須) で実装する |
| Q21 | manifest v2 での削除 (tombstone) と rename の表現。本書の manifest v2 と TXF2 は削除を運ばない。正本は ai-game 文書の AIGAME-M07 と SPEC-SY-tombstone (AGD-DM-06, AGD-GW-15, AGD-AI-20) | neco (ai-game 文書の担当が起案) | rename の内容転送 0 バイトは本書 M02 で満たす。削除の表現は ai-game 文書に従い、manifest v2 の entry encoding へ足す時は SPEC-SY-manifest-v2 を同時に改める |
| Q22 | flag day 後の巻き戻し期間の長さと、終了の判断 (§5.3) | neco | 30 日。終了の宣言まで書き出しを続ける |

**相手文書と他テーマ文書への整合依頼。** 次を各文書の担当へ依頼する。

- Curare 側文書: 冒頭の相対パスを `../../../Synergos/spec/plan/redesign-transfer-efficiency.md` に直す。§7 の注記 (Q7 と Q9 は Synergos 側だけ) に Q18〜Q22 を加える。§2.1 の区分を本書 §2.1 の 5 種の写し方に揃える (TRN-DD-17 は「未実装機能」、TRN-TP-07 は「設計ギャップ (不整合)」)。契約の種別 W / A / X の凡例を本書 §4.0 と同じにする。SPEC-SY-format-policy の契約文の根拠注記を AGD-AI-01 にする。Synergos のドメインに触れる場合は本書 §4.30 の割当に従う。
- security 文書: §4.24 と native-contracts の対応表に SPEC-SY-chunk-access の「条件付き」の文言が残っていれば外す (§4.26)。Q19 の契約文の追記を検討する。

## 8. 参考資料

設計の根拠にした外部資料だけを挙げる (調査ノート `research/transfer-efficiency.md`、`research/vcs-game-ai.md` から)。

1. Hugging Face Xet chunking: https://github.com/huggingface/hub-docs/blob/main/docs/xet/chunking.md
2. Hugging Face Xet xorb 形式: https://github.com/huggingface/hub-docs/blob/main/docs/xet/xorb.md
3. Hugging Face Xet deduplication: https://github.com/huggingface/hub-docs/blob/main/docs/xet/deduplication.md
4. Hugging Face Xet hashing: https://github.com/huggingface/hub-docs/blob/main/docs/xet/hashing.md
5. Hugging Face Xet upload protocol: https://github.com/huggingface/hub-docs/blob/main/docs/xet/upload-protocol.md
6. Hugging Face "Rearchitecting uploads and downloads": https://huggingface.co/blog/rearchitecting-uploads-and-downloads
7. Hugging Face parquet CDC: https://huggingface.co/blog/parquet-cdc
8. BLAKE3: https://github.com/BLAKE3-team/BLAKE3
9. Bao 仕様: https://github.com/oconnor663/bao/blob/master/docs/spec.md
10. bao-tree (チャンクグループ): https://docs.rs/bao-tree
11. fastcdc-rs (変種で切れ目が異なる警告): https://github.com/nlfiedler/fastcdc-rs/blob/master/README.md
12. ZipLLM (LLM チェックポイントで 54.1 % 削減): https://arxiv.org/html/2505.06252v3
13. borg の内部構造 (チャンク保管と pack): https://borgbackup.readthedocs.io/en/1.0-maint/internals.html
14. zstd `--patch-from`: https://man.netbsd.org/1/zstd.1
15. xdelta のメモリ予算: https://github.com/jmacd/xdelta/blob/wiki/TuningMemoryBudget.md
16. bsdiff のメモリ要件: https://pages.cs.wisc.edu/~starr/bwapi-read-only/branches/bwapi4/bwapi/bsdiff/Binary%20diff.txt
17. rsync アルゴリズム: https://rsync.samba.org/tech_report/node2.html
18. quinn transport 既定値: https://github.com/quinn-rs/quinn/blob/main/quinn-proto/src/config/transport.rs
19. quinn BBR 実装: https://github.com/quinn-rs/quinn/blob/main/quinn-proto/src/congestion/bbr/mod.rs
20. quinn の BBR に関する issue: https://github.com/quinn-rs/quinn/issues/1749
21. quinn-udp (GSO / GRO): https://github.com/quinn-rs/quinn/blob/main/quinn-udp/src/unix.rs
22. QUIC が 500 Mbps を超える時の性能: https://arxiv.org/pdf/2310.09423
23. draft-ietf-ccwg-bbr-04: https://datatracker.ietf.org/doc/html/draft-ietf-ccwg-bbr-04
24. QUIC ACK frequency: https://datatracker.ietf.org/doc/draft-ietf-quic-ack-frequency/
25. IETF resumable upload: https://github.com/httpwg/http-extensions/blob/main/draft-ietf-httpbis-resumable-upload.md
26. tus 1.0: https://github.com/tus/tus-resumable-upload-protocol/blob/main/protocol.md
27. S3 の上限 (単発 PUT 5 GB、part 10 000): https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html
28. MinIO の上限: https://docs.min.io/community/minio-object-store/operations/concepts/thresholds.html
29. Cloudflare のキャッシュと本文サイズ: https://developers.cloudflare.com/cache/concepts/default-cache-behavior
30. Cloudflare Tunnel の要件 (QUIC の扱い): https://developers.cloudflare.com/tunnel/platform/system-requirements/
31. NAT 越えの仕組み (DERP): https://blog.apnic.net/2022/04/26/how-nat-traversal-works-nat-notes-for-nerds/
32. BitTorrent BEP 3: https://github.com/bittorrent/bittorrent.org/blob/master/beps/bep_0003.rst
33. libtorrent の piece picker: https://blog.libtorrent.org/2011/11/writing-a-fast-piece-picker/
34. Uber Kraken (P2P の Docker レジストリ): https://github.com/uber/kraken/blob/master/README.md
35. Dragonfly (P2P 配信): https://d7y.io/docs/v2.3.0/
36. TLS early data (0-RTT) の注意: https://blog.trailofbits.com/2019/03/25/what-application-developers-need-to-know-about-tls-early-data-0rtt/
37. Unreal Engine の virtual assets: https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-virtual-assets-in-unreal-engine
38. Steam の partial depot 更新: https://partner.steamgames.com/doc/sdk/updating/partial_depot
39. ProjFS のデータ取得 callback (VFS の土台): https://learn.microsoft.com/en-us/windows/win32/api/projectedfslib/nc-projectedfslib-prj_get_file_data_cb
40. WebDataset 形式 (データセット shard): https://huggingface.co/docs/hub/en/datasets-webdataset.md
