# Synergos 再設計 — AI開発・ゲーム開発の課題解決

- 作成日: 2026-10-23
- 対象: Synergos main @ d404f20
- 関連テーマ: 本書は 3 テーマのうち ai-game-dev (AI 開発・ゲーム開発のワークフロー) の Synergos 側正本。版管理の対象モデル (Blob / Tree / Changeset / Ref) と、その複製・作業ツリー反映・履歴保持を持つ。同 repo の他テーマは `spec/plan/redesign-security.md` (脆弱性と侵害の歯止め。署名封筒・LCT・パス方針・保持ロック・desktop workspace の分割を持つ) と `spec/plan/redesign-transfer-efficiency.md` (転送速度と効率。内容 ID・wire・manifest v2・CDC / チャンク保管の判断を持つ)。相手 repo の同名文書は Curare リポジトリの `spec/plan/redesign-ai-game-dev.md` (ref の権威・データセット・権利・来歴・消去台帳を持つ)。repo をまたぐ相対リンクはホスティング側で切れるので書かない。契約 ID・機構 ID (AIGAME-Mxx)・目標 ID (G-A*)・受入試験の番号 1〜21・未決事項 Q-A* は両文書で同一 (提出前の照合は §5.3)。
- 根拠資料: 本 repo の `docs/versioning-design.md`、`docs/operations-readiness.md`、`docs/hooks.md`、`docs/site-operations.md`、`docs/two-node-operations.md`、`docs/platforms.md`、`spec/feature/opaque-custody.md`、`spec/feature/native-contracts.md`、`spec/plan/cpp-migration.md`。Curare の `spec/plan/versioning-types.md`、`spec/feature/ux-definition.md`。解析成果は `spec/plan/redesign-analysis/` の `HANDOFF.md` (§6 契約変更と実装の非効率の区別、§7.1 オーナー決定)、読解マップ `maps/sy-*.md`、調査ノート `research/vcs-game-ai.md`・`research/ai-dev-data.md`・`research/transfer-efficiency.md`、検証済み発見 `findings/ai-game-verified.json` (kept[] 77 件のうち Synergos 関係 60 件。本書と同じ Revisor 変更で取り込む。security 文書 §5 P0 と同じ扱い)。外部資料は §8。
- 判断の前提: 2026-10-23 のオーナー決定 6 点 (`spec/plan/redesign-analysis/HANDOFF.md` §7.1)。特に決定 6 により、本書の Synergos 実行時変更はすべて **C++ 移行の互換契約 / ネイティブ実装要件** として書き、現行 Rust 実装へは入れない。

## 0. 結論 (先に)

- **足りないもの**: 内容と結び付いた版の ID と、changeset・親・tree・ref の集合単位。版は `path → u64 + size + CRC32` で、索引の権威も git と Curare に割れている (§2.6)。
- **作り直すもの**: 1 つの内容アドレス型グラフ (SPEC-SY-object-model)。ref の権威は Curare、Synergos は署名付き複製・commit・作業ツリーへの原子的反映を担う。その上にロック cache・部分ワークスペース・墓標・複製数の勘定・汚染の封じ込め・LFS 互換を載せる。
- **着地点**: すべて C++ 移行の互換契約。`cpp-migration.md` 手順 3 の前に互換契約を改訂し (3 文書共通の改訂表、§4.27)、Rust 側は SECURITY-M30 の workspace 分割以外を変えない。
- **順序**: A0 契約・vector・ゲート → A1/A2 Curare の土台 → A3 C++ のオブジェクトモデル (security P3/P4・transfer T2 と同じ flag day) → A4 ML 向けの読み出し → A5 エコシステム連携と format 1 書込の退役。
- **最悪シナリオ**: auto-pull が未 publish の `.uasset` を上書きし、asset と `.meta` が別々に届いて Unity の GUID がチーム全体で作り直される。C++ が format 1 を出荷した後では、この修正に移行が 2 回要る。

## 1. 背景・ゴール・非ゴール

### 1.1 背景

現状設計の要点 (既存 spec の引用):

- `docs/versioning-design.md:10`: 「**差分管理はしない**。履歴の「索引」は git 側 (`.synergos/manifest.json` をコミット)。履歴の「実体」は **フラグを立てた履歴ノード (history node) が版ごとに丸ごと保持**する」。
- `docs/versioning-design.md:11`: 「転送は Phase 1 のまま全体転送 (blake3 検証)。差分の検知は `version` + size + CRC で足りる。CDC チャンク重複排除案は neco 決定 (2026-08-16) で取り下げ」。
- `docs/versioning-design.md:28`: 「Synergos は「何が何版か」の履歴を持たず、それは git に一本化する。」
- `docs/versioning-design.md:104`: 「履歴ノードは複数あってよい (全部が保持する = 冗長化)。相互の同期はしない (各自が見た版を持つ)。」
- `docs/versioning-design.md:13`: 「自動マージはしない (バイナリはマージ不能)。退避・選択 UI とロック機構は Phase 3」。
- `spec/feature/opaque-custody.md:5`: 「Synergosはopaque objectの転送と保管の実行を担当し、制作履歴や承認を作らない。」(2026-09-08 の Cu / Sy / Cr 境界)
- Curare `spec/plan/versioning-types.md:3-4`: 「この文書は従来の Git 連携前提、および Synergos が制作履歴の意味を管理する案を置き換える。Git は製品利用の前提にしない。」
- Curare `spec/feature/ux-definition.md:94`: 「既存の編集は勝手に上書きしない。」
- `spec/plan/cpp-migration.md:41-43`: 「Versioning: preserve `.synergos/manifest.json`, local high-water marks, restore/checkout semantics, tags and protected history.」

ユーザー要求は、AI 開発・ゲーム開発の課題解決、セキュリティ、転送速度と効率の 3 条件を同時に満たすことである。本書は AI 開発・ゲーム開発の課題解決を扱う。セキュリティの不変条件 (署名・会員資格・パス方針・保持ロック) は `spec/plan/redesign-security.md`、転送の wire と CDC は `spec/plan/redesign-transfer-efficiency.md` の契約をそのまま使い、本書では再定義しない。

オーナー決定 (2026-10-23) のうち本書を拘束するもの:

| # | 決定 | 本書での扱い |
|---|---|---|
| 1 | 設計書は 3 テーマ × 2 repo の 6 本 | 本書は ai-game-dev の Synergos 側 |
| 2 | 提出経路は Revisor のみ | 本書と `cpp-migration.md` 改訂は Revisor の local PR で出す。検証もすべて Revisor のマージ前ケース |
| 3 | CDC / チャンク化を再検討。チャンク保管の独立モジュール化もありうる | 比較と採否は転送効率文書 (D-T02 / D-T03 で独立モジュール `synergos-chunkstore` を採用)。本書はどのパスが入るかの利用側の見解だけを持つ (D-A20) |
| 4 | Cernere 側の短寿命 capability 署名鍵、SSF/CAEP 失効、step-up は SPEC-CR-* として依頼済み | 依存として参照し、各暫定策を書く (§4.24)。Synergos / Curare には実装しない |
| 5 | quick-xml 0.30 は依存経路自体が要るかから判断 | 製品として eframe の accesskit 経路は要らない。作業は SECURITY-M30 (D-13 の E): `synergos-gui` を `desktop/` workspace へ移し accesskit を外す。egui は C++ desktop-gui の受入まで parity の基準として残す (D-A19) |
| 6 | HLO2・custody 強化・認証強化は Rust に入れず C++ 移行契約として再仕様化 | 本書の機構も同じ扱い。各機構に「契約変更 / 実装要件」の区分を付け (§4.30)、`cpp-migration.md` 改訂案を §4.27 に置く |

### 1.2 ゴール (測定可能な到達条件)

番号は Curare 文書と共通 (G-A*)。Synergos が担うものだけを番号順に載せる。「(transfer)」は転送効率テーマが持つ数値で、本書は引用する。

| # | ゴール | 到達条件 | 対象発見 |
|---|---|---|---|
| G-A2 | 未 publish 編集の上書き 0 | auto-pull が dirty な作業ファイルを置換する件数 **0** (Full / Manual / Selective の回帰コーパス)。受信バイトは `.conflict-<short>` の兄弟ファイルへ | AGD-GW-02 |
| G-A3 | changeset の原子性 | 5,000 ファイルの OFPA changeset の反映中に受信側を kill しても、再起動後は旧 tree か新 tree のどちらかで、混在 **0**。asset と `.meta` が分かれない | AGD-GW-22, AGD-DM-02 |
| G-A4 | rename と削除 | 500 MB の PSD の rename / move で受信側への内容転送 **0 バイト**。削除の伝播後の復活 **0** | AGD-DM-06, AGD-GW-15 |
| G-A5 | 部分ワークスペース (クライアント側) | 2 TB / 10^6 エントリのプロジェクトで、パスの 2 % を覆う view の取得バイト **≤ 5 %**。取得する tree node 数は変更のあった部分木の数に比例し、エントリ数に比例しない | AGD-DM-13, AGD-GW-06 |
| G-A6 | オフラインからの追いつき | 30 回分の publish の間オフラインだったノードが、接続後 **60 秒 + 転送時間** 以内に head に達する。生の offer を必要としない。状態表示に「head から N changeset / M ファイル遅れ」を出す | AGD-GW-10 |
| G-A13 | 改ざん封じ込め (受信側) | 証明済みまたは署名 ref から到達可能な tree エントリに一致しないバイトを、作業ツリーへの反映・履歴保管・loader への引渡しに使う件数 **0**。負けた候補は隔離し、先着を勝たせない | AGD-AI-21 |
| G-A15 | 複製の耐久性 (Synergos 側: GC) | 保護 ref から到達可能なオブジェクトを、有効な RetentionAck または受領証の保管場所が `min_copies` (既定 **2**) 未満のまま GC が消す件数 **0** (障害注入試験)。サイト export が確定済み blob の欠落を黙って成功にする件数 **0** (export の報告は Curare 文書の C5 と共有。本書は export の照合を M20 で持つ) | AGD-AI-18, AGD-OP-14, AGD-OP-07 |
| G-A16 | 大きいオブジェクト (利用側) | 方針の rule が `chunk=true` で 8 MiB 以上のファイルは、転送効率のベンチ (64 GiB × 10 checkpoint) で保存バイトの削減が **≥ 30 %** の場合だけチャンク層に入る (transfer)。500 MB PSD の再転送比率は転送効率の TG-07: チャンク有効プロジェクトで **≤ 35 %**、全体転送プロジェクトでは 100 % だが TG-08 により **拠点あたり 1 回** (transfer、暫定)。テンプレート例の「< 10 %」は未計測 | AGD-AI-07 |
| G-A18 | 可搬なパス | 予約パス・衝突のコーパス (`.Synergos/…`、`SYNERG~1`、`CON.txt`、NFC/NFD の組、大小文字だけ違う組、末尾のドット/空白、UTF-16 で 240 超) を windows-latest・macos-latest・ubuntu の 3 runner で **100 %** 拒否または隔離 | AGD-OP-01, AGD-OP-02 |
| G-A19 | 契約 vector | object-model の vector (≥ 40 件、10^5 エントリの prolly tree を含む) が C++・TypeScript・Rust の旧形式リーダーで **100 %** バイト一致 | AGD-DM-17, AGD-AI-20, AGD-DM-01 |
| G-A21 | ワークステーション配備 | Windows で daemon をマシンサービスとして入れ、状態を `%ProgramData%` 配下に置ける。2 つ目のインスタンスを **100 %** 拒否。明示した設定パスが無いとき **100 %** 起動失敗 | AGD-OP-03, AGD-OP-04, AGD-OP-13 |

### 1.3 非ゴール

| 範囲 | 本書で扱わない理由 | 正本 / 判断者 |
|---|---|---|
| ref の権威・ref log・changeset の受理・ロックの権威・view・データセット・権利と用途・来歴・消去台帳・証明の発行・S3 互換 gateway | Curare が所有する (2026-09-08 境界を維持) | Curare リポジトリの `spec/plan/redesign-ai-game-dev.md` |
| peer wire・TXF2・複製 stream・内容 ID・manifest v2・CDC とチャンク保管・IPC v2 の framing | 転送効率テーマの契約 | `spec/plan/redesign-transfer-efficiency.md` |
| 署名封筒・LCT・HLO2・会員資格・パス方針・置換旧バイト・保持ロック・sandbox・取込判定の展開ゲート・Cargo workspace の分割 | セキュリティテーマの契約 | `spec/plan/redesign-security.md` |
| Cernere 側の主体・step-up・失効通知 | オーナー決定 4 | Cernere オーナー (Actio タスク起票済み) |
| DCC / エンジンのアダプタ本体の置き場所 | repo を新設するか未決 | neco (Q-A4) |
| 現行 Rust 実行時への機能追加 | オーナー決定 6 | neco |

## 2. 現状分析 — 足りない要素と再設計すべき点

### 2.0 読み方

- 検証済み発見 (`findings/ai-game-verified.json` の kept[]) のうち `repo` が `synergos` または `both` の 60 件を全件載せる。`curare` だけの 17 件は Curare 文書が持つ (§2.7 で相互参照)。dropped[] の 1 件は使わない。
- 区分: 脆弱性 / 欠落機構 / 設計ギャップ / 非効率 / 未実装機能。「設計ギャップ (矛盾)」は 2 つの spec が食い違うもの。
- 重大度: 発見の severity (高 / 中 / 低) と検証者の優先度 (P0〜P2)。表記は Curare 文書と揃える (§5.3)。
- 「再設計の方向」の M 番号は AIGAME-M*、契約 ID は §4 の節に対応する。`both` の行は Synergos 側の根拠と影響だけを書き、Curare 側の根拠は Curare 文書にある。
- 引用した `file:line` は main @ d404f20 で確かめた。発見の引用が 1 行ずれていたもの (AGD-GW-06, AGD-GW-15) は本書で正した。

### 2.1 データモデル (DM)

| ID | 区分 | 現状 (file:line 根拠) | 影響 (AI/ゲーム制作での実害) | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| AGD-DM-01 | 設計ギャップ | 版の同一性が 3 系統。`ManifestEntry{version,size,crc,updated_at,publisher}` (`synergos-core/src/manifest.rs:35-46`)、custody は不透明 ID の SHA-256 (`synergos-custody/src/signature.rs:5-7`)、Curare は resource UUID → version_num | 「ビルド / データセット X に何が入っていたか」に答えられない。Curare のリリースを Synergos で checkout・保護・転送できない | 高 / P0 | M01: SPEC-SY-object-model に 1 本化 |
| AGD-DM-02 | 設計ギャップ | changeset が無い。`publish --all` は 128 件ずつ独立に送り途中成功を許す (`synergos-core/src/cli_worktree.rs:13,34-45`)。IPC は `PublishUpdate{project_id, file_paths}` (`synergos-ipc/src/command.rs:85-88`) | prefab + テクスチャ + `.meta`、OFPA レベル、データセットのディレクトリが途中で切れた状態で peer に届く | 高 / P0 | M05: SPEC-SY-commit-ipc。単一の ref CAS を commit 点にする |
| AGD-DM-03 | 設計ギャップ (矛盾) | 索引は git (`docs/versioning-design.md:10,184`、`synergos-core/src/manifest.rs:12-14`)。Curare は git を前提にしない (Curare `versioning-types.md:3-4`)。`cpp-migration.md:41` は manifest の保存を契約にする | git の無いアーティスト・データチームは履歴索引を持たない。git のあるチームは manifest の衝突が常に起きる | 高 / P0 | M04 + M02: Curare ref log を権威に、git は任意のポインタ (変更提案、Q-A1) |
| AGD-DM-04 | 設計ギャップ | 同一判定は size + CRC32 (`synergos-core/src/manifest.rs:203`)。`FileOffer.content_hash` は常にゼロ (`synergos-core/src/exchange/mod.rs:571-572`)。custody の鍵は不透明 ID のハッシュ | 重複排除・キャッシュ判定・lineage の `datasetVersion`・LFS 互換が成り立たない。size と CRC を合わせた細工ファイルが「変更なし」になる | 高 / P0 | M01 + 転送の SPEC-SY-content-identity (BLAKE3-256) |
| AGD-DM-05 | 設計ギャップ | 書込に親が無い。`version = max(手元+1, floor+1)` (`synergos-core/src/manifest.rs:204-205`)。競合は同版番号で CRC 違いだけ (`synergos-net/src/gossip/message.rs:229-237`)、`ConflictManager` はパスだけをキーにする (`synergos-core/src/conflict/mod.rs:84-87`) | PSD / FBX / `.uasset` / チェックポイントの同時編集が無言で失われ、発覚が遅い | 高 / P0 | M05 (base 付き commit と SPEC-CU-path-conflict) + M09 (ロック) |
| AGD-DM-06 | 設計ギャップ | `FileId` はパスそのもの (`synergos-core/src/ipc_server.rs:1381`、`synergos-net/src/types.rs:74`)。欠けたファイルは削除として publish しない (`synergos-core/src/cli_worktree.rs:55-59`)。checkout は余分なファイルを報告だけする (`synergos-core/src/checkout.rs:111`) | move は全再転送と版番号のリセット。削除したアセット (法的に消すべきものを含む) が復活する | 高 / P0 | M07: パスは tree の属性、同一性は `entry_id` |
| AGD-DM-07 | 設計ギャップ (矛盾) | タグは node ローカルで無署名の (path → version) 集合 (`synergos-core/src/history/tags.rs:3-6,22-31`、`docs/versioning-design.md:208`) | マイルストーン・プラットフォーム提出・学習データの名前付き状態が不変・複製・検証可能でない | 中 / P1 | M04: ref (tag / release / alias / dataset_snapshot) に統一 |
| AGD-DM-08 | 設計ギャップ (矛盾) | 権威が宣言されていない。「履歴を持たず git に一本化」(`docs/versioning-design.md:28`)、局所高水位は `.synergos/state.json` (`:191`)、custody 側は「Cuの業務保持参照をSy独自ポリシーで無視しない」(`spec/feature/opaque-custody.md:42`) | Synergos の restore や GC が Curare の版と静かに食い違い、リリースが参照するバイトを追い出す | 高 / P0 | M01 の権威表 + M04 |
| AGD-DM-09 | 設計ギャップ | blob 平面が 2 つ。履歴 CAS は `<root>/<blake3(project_id)>` (`synergos-core/src/history/store.rs:80-87`)、custody は `sha256(opaque id)` (`synergos-custody/src/signature.rs:5-7`) | P2P で配ったファイルを版にするには Curare へ再アップロードが要り、WAN 転送と保管が倍になる | 中 / P1 | M01: 両平面を `b3:` ID とプロジェクト単位の重複排除領域に統一 |
| AGD-DM-10 | 設計ギャップ (矛盾) | 履歴の対象は既定で全プロジェクト (`synergos-net/src/config.rs:453-454`)、受信物も全部保管 (`synergos-core/src/exchange/mod.rs:833-841`)。manifest にクラスが無い (`manifest.rs:35-46`) | 再生成可能な出力 (cook、DDC、プレビュー、学習 shard) が保管を食い、ソースと同じ GC 運命を負う | 中 / P1 | M08: SPEC-CU-project-policy の rule id を tree エントリに記録して消費 |
| AGD-DM-12 | 未実装機能 | プロジェクトを隔離し保管庫もプロジェクト単位 (`synergos-core/src/history/store.rs:80-87`) | 共有エンジンコンテンツ・外注パック・共有コーパスがプロジェクトごとに重複保管・転送される | 中 / P2 | M13: tree の `link` エントリを読み取り専用で実体化 |
| AGD-DM-13 | 設計ギャップ | `Selective` は常に空のパターン (`synergos-core/src/project.rs:32-33,55`)。`.synergosignore` は意図して機械ローカル (`synergos-core/src/worktree_status.rs:21-22`)。checkout は manifest 全体を回す (`synergos-core/src/checkout.rs:80`) | 数 TB のプロジェクト・データセットを全量取得する。パス単位の読み取り秘匿 (未発表キャラクター) を強制できない | 中 / P1 | M10: SPEC-SY-workspace-view |
| AGD-DM-14 | 設計ギャップ (矛盾) | プロジェクト ID は自由文字列 (`synergos-core/src/project.rs:683-689`) で gossip topic・DHT キー・保管庫名を兼ねる。`FileId(pub String)` (`synergos-net/src/types.rs:74`)。版番号はノードごとに独立発番 (`docs/versioning-design.md:191`) | 別スタジオの同名プロジェクトが gossip と履歴を混ぜる (漏えい・破損) | 中 / P1 | M03: SPEC-CU-project-descriptor の `project_ref` を消費 |
| AGD-DM-17 | 設計ギャップ | `cpp-migration.md:41-43` と `native-contracts.md:17,22-24` が format 1・size/CRC の「変更なし」・局所高水位を規範にする。最初のネイティブ増分は manifest/state JSON を含む (`cpp-migration.md:82-83`) | 欠陥のある模型を C++ で作り直し、修正に移行がもう 1 回要る。その間 format 1 のデータが増え続ける | 高 / P0 | M02: 手順 3 の前に互換契約を改訂 |
| AGD-DM-18 | 設計ギャップ (矛盾) | 「プロジェクトの中身」の表現が 7 つ。`RootCatalog` (`synergos-net/src/catalog/types.rs:7`)、`ChainBlock` (`synergos-net/src/chain/file_chain.rs:9`)、`ChunkDag` (`synergos-net/src/content/chunker.rs:22`)、`HistoryIndex` (`synergos-core/src/history/index.rs:56`) ほか。休眠中の catalog が publish ごとに絶対パス付きで gossip される (`synergos-core/src/exchange/mod.rs:286`) | 版の層の変更コストが上がり、存在しない保証 (fork 検出、差分同期) を利用者に誤認させる | 低 / P2 | M01: RootCatalog → RefUpdate、FileChain / ChunkDag は廃止、HistoryIndex は changeset から導出する cache |

### 2.2 ゲーム開発ワークフロー (GW)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| AGD-GW-01 | 未実装機能 | ロックが無い。IPC にロック系のコマンドが無い (`synergos-ipc/src/command.rs`、d404f20 で該当 0 件)。設計は「Slack で一言」で Phase 3 送り (`docs/versioning-design.md:337-339`)。C++ の `ManifestEntry` も同じ (`synergos-core/native/include/synergos/core/manifest_versions.hpp:7-13`) | P4 の `+l` 相当が無く、`.psd/.mb/.uasset/.unity` の上書き事故を防げない。一定規模以上のチームで Perforce を置き換えられない | 高 / P0 | M09: Curare 権威のロック + SPEC-SY-lock-announce |
| AGD-GW-02 | 設計ギャップ | auto-pull の条件に dirty 判定が無い (`synergos-core/src/daemon.rs:984-986`)。`SyncMode` は保存されるが daemon と exchange は読まない (`synergos-core/src/project.rs:27-34,55`)。受信の比較は記録済みの版だけ (`synergos-core/src/exchange/mod.rs:765-777`)。`resolve_conflict` は状態を変えるだけ (`synergos-core/src/conflict/mod.rs:242`) | 未 publish の作業が無言で消える。製品規則「既存の編集は勝手に上書きしない」(Curare `ux-definition.md:94`) に反する | 高 / P0 | M06: SPEC-SY-worktree-apply (dirty guard と同期モードの意味) |
| AGD-GW-03 | 未実装機能 | 系譜はパスごとに 1 本 (`docs/versioning-design.md:42`)。巻き戻し後の版番号衝突を max で回避 (`:188-190`)。タグは 1 履歴ノード内のファイル (`synergos-core/src/history/tags.rs:56`) | release からの hotfix・merge-down が無い。並行マイルストーンを回せない | 中 / P1 | M04: ref の種別と stream 記録 (権威は Curare) |
| AGD-GW-04 | 設計ギャップ | 2 つの版管理系が ID を共有しない。custody は `sha256(blobRef)` (`synergos-custody/src/signature.rs:5-7`)、「git-LFS の P2P 版」(`docs/versioning-design.md:12`)、「制作履歴や承認を作らない」(`spec/feature/opaque-custody.md:5`) | 「hero.mb v7 は何で誰が承認したか」に 1 つの答えが無い | 高 / P0 | M01 |
| AGD-GW-05 | 未実装機能 | publish は per-file の bump と archive の直後に全 peer へ放送 (`synergos-core/src/ipc_server.rs:1417`、`synergos-core/src/exchange/mod.rs:1325-1327`) | WIP の受け渡しが恒久履歴を汚すか、Slack / Drive に逃げる | 中 / P2 | M10: `shelf` ref (未受理 changeset を指名 peer と共有) |
| AGD-GW-06 | 未実装機能 | checkout と auto-pull は manifest 全体 (`synergos-core/src/checkout.rs:80-81`、`synergos-core/src/daemon.rs:984-986`)。`.synergosignore` は publish 側の除外だけ (`synergos-core/src/worktree_status.rs:18-23`、`docs/operations-readiness.md:30`) | コンソール規模の depot をアーティストのノート PC で使えない | 高 / P1 | M10 |
| AGD-GW-07 | 未実装機能 | ars plugin はイベントの橋渡しだけ (`ars-plugin-synergos/src/bridge.rs:1-4,14-21`)。Tauri は open/publish のみ (`synergos-tauri/frontend/src/lib/tauri.ts:85-129`)。LFS の endpoint 無し | エディタ / DCC 統合が無く、既存の Unity / Unreal の LFS プラグインも使えない | 高 / P1 | M23: SPEC-SY-local-broker、SPEC-SY-lfs-transfer-agent |
| AGD-GW-10 | 設計ギャップ | 接続は維持・再接続する (`synergos-core/src/peer_link.rs:24,108-128`) が、pull の契機は生の FileOffer だけ (`synergos-core/src/daemon.rs:984-997`)。CatalogSync は実ファイルに使われない (`docs/operations-readiness.md:26`)。resume 無し (`synergos-net/src/transfer/mod.rs:11-12`)。pin は壁時計 5 分 (`synergos-core/src/exchange/mod.rs:26`) | 戻ってきたノードは「接続中」のまま古い入力で作業する | 高 / P0 | M06 + 転送の SPEC-SY-replication-stream / SPEC-SY-txf2 |
| AGD-GW-11 | 未実装機能 | 派生物の置き場が無い。hook がソースツリーを書き換えて publish する (`docs/hooks.md:31,33,136-146`) | cook / DDC / プレビューが反復時間を支配するのに再利用できない | 中 / P2 | Curare の SPEC-CU-derived-cache (Synergos は M08 のクラスで派生物を履歴のソースとして保管しない) |
| AGD-GW-14 | 設計ギャップ | git に入れる manifest は整形 JSON の全体書換え (`synergos-core/src/manifest.rs:55,166`)。merge driver も `.gitattributes` も無い。競合するテキストアセットは拒否 (`synergos-core/src/exchange/mod.rs:765-777`)。`TextDiff` は休眠 (`synergos-net/src/chain/file_chain.rs:135`) | 別ファイルを publish した 2 人が git で必ず衝突する | 中 / P0 | M02 (別ファイルの 1 行ポインタ + merge driver) + M05 (方針で選ぶ merge driver) |
| AGD-GW-15 | 設計ギャップ | rename = 新規エントリ (`synergos-core/src/manifest.rs:213`)。削除は伝播しない (`docs/operations-readiness.md:29`)。manifest のキーはバイト一致で大小文字の rename が 2 エントリになる | Unity / Unreal の再編成のたびに全再転送。「ゲームから消した」アセットが戻る | 中 / P1 | M07 |
| AGD-GW-16 | 未実装機能 | Perforce / SVN / Git LFS からの取込み経路が無い (LFS は `docs/versioning-design.md:12,103,337` の比喩だけ) | 採用時に履歴をゼロから始める | 中 / P2 | M23 (Synergos は SPEC-SY-legacy-import と LFS agent。取込器は Curare の SPEC-CU-import-origin) |
| AGD-GW-17 | 設計ギャップ | 全 manifest 書換え (`synergos-core/src/manifest.rs:162-166`)、起動時の全 CRC 走査 (`synergos-core/src/restore.rs:44`)、IPC 1 MiB (`synergos-ipc/src/transport.rs:39`)、`--all` は 128 件 (`synergos-core/src/cli_worktree.rs:11-13`)、履歴ノードは相互同期しない (`docs/versioning-design.md:103-104`) | 数百万ファイル規模のスタジオ (公開例: 24M ファイル) と大半のデータセットが対象外 | 高 / P1 | M02 の NR-AG-1〜3 (C++ 実装要件) + Curare M11 |
| AGD-GW-18 | 未実装機能 | 保持は全パス共通の 1 つの設定 (`synergos-net/src/config.rs:288-299`、`docs/versioning-design.md:201`) | ソースの履歴を失わずに保管量を抑える手段が無い | 低 / P2 | M08: SPEC-CU-project-policy の消費 |
| AGD-GW-19 | 非効率 | gossip は (メッセージ, peer) ごとに 1 stream (`synergos-core/src/daemon.rs:878`)、FileOffer はファイルごと (`synergos-core/src/exchange/mod.rs:1343`) | OFPA レベルや小ファイルのデータセットは制御オーバーヘッドが支配する | 中 / P1 | M05 (changeset 単位) + 転送の `WANT_MANY` (SPEC-SY-txf2) |
| AGD-GW-20 | 設計ギャップ | `cpp-migration.md:41-43` の保存契約と Completion gate の parity (`:66-67`)。C++ codec は format 1 以外を拒否 (`synergos-core/native/manifest_codec.cpp:35`) | ロック・dirty 判定・rename・削除・内容 ID の欠落が parity 要件になる | 高 / P0 | M02 |
| AGD-GW-22 | 設計ギャップ | publish と受信はファイル単位で途中停止しうる (`synergos-core/src/ipc_server.rs:1352,1376,1403`、`synergos-core/src/cli_worktree.rs:34-47`)。受信側はファイルごとに auto-pull (`synergos-core/src/exchange/mod.rs:1343`、`daemon.rs:984-997`) | asset と `.meta` が別々に届き、Unity が GUID を再生成してチーム全体に広がる | 高 / P0 | M05 + M06 |

### 2.3 AI データ・モデル (AI)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| AGD-AI-06 | 欠落機構 | manifest に墓標が無い (`synergos-core/src/manifest.rs:35-46`、`docs/operations-readiness.md:29`)。履歴 GC は年齢と件数だけ (`synergos-core/src/history/gc.rs:22-26`)。custody の reclaim は即時削除で hold が無い (`synergos-custody/src/store.rs:210-215`)。退避先の暗号化経路が無い (推定。`synergos-core/src/history/rotation/mod.rs` に暗号化処理が無く、`:194-198` はローカルの object path をそのまま `backend.put` に渡す) | 同意撤回・オプトアウト・特別カテゴリの削除義務を果たせず、学習データを預かれない | 高 / P0 | M19: SPEC-SY-tombstone (Curare の消去台帳を消費) |
| AGD-AI-07 | 非効率 | custody は単発 PUT で 1800 秒 (`synergos-custody/src/handlers.rs:236`)、GET は全体 stream (`:254-255`)、transfer は単一 stream で resume 無し (`synergos-net/src/transfer/mod.rs:11-12,25`)、CDC は取り下げ (`docs/versioning-design.md:11`)、固定長 chunker は呼び出し元無し (`synergos-net/src/content/chunker.rs:37-41`) | 数十 GB のチェックポイントを保管できない。反復保存の再利用率 0 % | 高 / P1 | 本体は転送効率文書 (SPEC-SY-chunk-store、SPEC-SY-custody-transfer-v2)。本書は M22 で利用側の条件を宣言 |
| AGD-AI-08 | 未実装機能 | custody の GET は 1 オブジェクト全体 (`synergos-custody/src/handlers.rs:243-269`)。Range も一括取得面も無い | GPU クラスタが学習データを読めない | 高 / P1 | M22: `edge` 役が Bao 検証付きの範囲を配る (gateway は Curare) |
| AGD-AI-09 | 非効率 | 重複排除はファイル単位かつプロジェクト内だけ (`docs/versioning-design.md:151`)、custody は不透明 ID を鍵にする (`synergos-custody/src/signature.rs:5-7`) | 重なるスナップショットが毎回全コピー | 中 / P1 | M01 + M13。範囲はプロジェクト単位 (D-A16)。領域の定義は SPEC-CU-object-registry + SPEC-SY-envelope-encryption |
| AGD-AI-13 | 設計ギャップ | タグは node ローカル (`synergos-core/src/history/tags.rs:3-4`)、履歴ノードは相互同期しない (`docs/versioning-design.md:104`)、GC は年齢 (`synergos-core/src/history/gc.rs:55-58`) | 学習スナップショットとリリースが再現も検証もできない | 中 / P1 | M04: 保護 ref の閉包を全ノード同一の keep-set にする |
| AGD-AI-15 | 非効率 | manifest は publish ごとに全体を整形 JSON で書換え (`synergos-core/src/manifest.rs:162-171`)、同一性は CRC32 (`:35-46`)、offer に内容ハッシュ無し (`synergos-core/src/exchange/mod.rs:569-572`)、1 publish 1024 パス (`synergos-ipc/src/command.rs:227`)、履歴索引は 1 つの JSON (`synergos-core/src/history/index.rs:56-61`) | 10^5〜10^7 ファイルのデータセット (と同じ形の OFPA) を運べない | 高 / P1 | M02 の NR-AG-1〜3 + M01 (prolly tree) |
| AGD-AI-16 | 欠落機構 | 履歴ノードは接続中の任意の peer に任意の版を返す (`synergos-core/src/exchange/mod.rs:959-990`、`docs/versioning-design.md:101`) | ライセンス・同意の範囲が限られたデータセットを Curare の方針を迂回して配れる | 中 / P1 | M17 の Synergos 側 (§4.19): 履歴 / edge は `(project_ref, tree, purpose)` を名指す LCT がある場合だけ配る。素の認可は SECURITY-M03 |
| AGD-AI-17 | 未実装機能 | 独自の TXFR framing だけ (`synergos-net/src/transfer/mod.rs:25`)。LFS / HTTP の adapter 無し | ML とゲームの既存ツールに専用クライアントが要る | 中 / P2 | M23 (LFS agent、broker)。OpenLineage・MLflow・S3 は Curare |
| AGD-AI-18 | 設計ギャップ | 履歴ノードは相互同期しない (`docs/versioning-design.md:104-106`)。GC は複製数を見ない (`synergos-core/src/history/gc.rs:89-96`)。custody の durable は単一ボリューム (`spec/feature/opaque-custody.md:26,31`、`synergos-custody/src/handlers.rs:124` の `locations` は常に 1 件) | 参照されたスナップショットが既知の複製数で残る保証が無い | 中 / P1 | M20: SPEC-SY-replica-accounting |
| AGD-AI-19 | 設計ギャップ | AI のペルソナと受入シナリオが無い。判断軸は `docs/versioning-design.md:15` のゲーム寄りの 4 択だけ | AI の発見に受入条件と担当が無い | 低 / P1 | Curare の M29 (ペルソナとフロー)。Synergos は §6 のシナリオを Augur に登録 |
| AGD-AI-20 | 設計ギャップ | `native-contracts.md:17,24`、`cpp-migration.md:41-42,45-46` が CRC32・無署名・墓標無しの manifest と単一 SHA-256 の HMAC 受領証を保存契約にする。C++ の `ManifestEntry` も同形 (`manifest_versions.hpp:7-13`) | 内容 ID・署名付き系譜・墓標を足すには、移植中の契約を開き直す必要がある | 中 / P0 | M02 (期限: `cpp-migration.md` 手順 3 の前。`:44-47` の Custody 行は §4.27 の共通改訂表) |
| AGD-AI-21 | 設計ギャップ | 版は無署名の wire ヘッダから作る (`synergos-core/src/exchange/mod.rs:747`)。履歴は先着が勝ち、本物を後から拒否する (`synergos-core/src/history/store.rs:154-160`)。offer に内容ハッシュ無し (`exchange/mod.rs:569-572`)。publisher は送信元そのまま (`:840`) | 悪意ある peer が学習データを無言で汚染できる。ゲームアセットなら目に見える誤りだが、学習データではモデルの挙動が静かに変わる | 高 / P0 | M21 (受信側): 署名 ref から到達可能な tree エントリに一致するバイトだけを受ける |

### 2.4 運用・プラットフォーム (OP)

| ID | 区分 | 現状 (file:line 根拠) | 影響 | 重大度 | 再設計の方向 |
|---|---|---|---|---|---|
| AGD-OP-01 | 脆弱性 | `.synergos` の保護は大小文字を区別する比較 (`synergos-core/src/manifest.rs:308-313`、`synergos-ipc/src/command.rs:266-272`)。`.synergos/hooks.toml` は実行定義 (`synergos-core/src/hooks/project_file.rs:1,14-15`) | NTFS / APFS 既定で peer が `.Synergos/...` から manifest・state を上書きし hook を仕込める。スタジオの主力 OS で守りが効かない | 高 / P0 | M07 + SECURITY-M13 (SPEC-SY-path-policy) |
| AGD-OP-02 | 設計ギャップ | パスの正規化は `\` → `/` と先頭除去だけ (`synergos-core/src/manifest.rs:282-288`)、拒否は空・絶対・制御文字・`:` (`:295-317`)。NFC/NFD・大小文字・予約名・長さの扱い無し | 濁点付きのかな名、大小文字だけ違う名前、Windows 予約名で同じアセットが OS 間で分岐し、Unity の GUID と Unreal のパッケージパスが壊れる | 高 / P1 | M07 + 記述子の `path_policy` |
| AGD-OP-03 | 設計ギャップ | サービス化しない (`docs/two-node-operations.md:255-258`、`docs/platforms.md:111-115`)。IPC の呼び出し元は同一 SID 比較 (`synergos-core/src/ipc_server.rs:342,369`)。identity 鍵は roaming `%APPDATA%`、無ければ `.` や `/tmp` (`synergos-net/src/identity.rs:196-217`)、平文保存 (`:125-137`) | ビルド・cook・レンダー・GPU 学習ノードが常駐 seed / archive になれない。鍵の複製や盗難で他ノードになりすませる | 高 / P0 | M25 (マシンモード、OS 鍵保管) + SECURITY-M04 |
| AGD-OP-04 | 設計ギャップ | 明示した `--config` が無いと既定値で起動 (`synergos-core/src/daemon.rs:581-595`)。Tauri は daemon が読まない場所に書く (`synergos-tauri/src-tauri/src/config.rs:164-198`)。`ConfigUpdate` は何もしないで Ok (`synergos-core/src/ipc_server.rs:2042-2047`)、GUI は「反映」と表示 (`synergos-gui/src/ui/settings.rs:109`) | 履歴ノードの設定ミスが復元時まで見えないデータ損失になる | 中 / P1 | M25: 設定の単一解決器と fail-fast |
| AGD-OP-05 | 設計ギャップ | Tauri の bundle は daemon を含まず起動もしない (`synergos-tauri/src-tauri/tauri.conf.json:4,26-33`)。QUIC Hello に版情報が無い (`synergos-net/src/quic/hello.rs:35-38`)。format 1 以外を拒否 (`synergos-core/src/manifest.rs:138-141`、`synergos-core/src/history/index.rs:118`) | ツール版をマイルストーンで固定できず、更新がスタジオ全体の停止リスクになる。manifest を git に入れているので最初の format 更新で旧クライアントが取り残される | 中 / P1 | M25 + SPEC-SY-format-skew |
| AGD-OP-06 | 設計ギャップ | 保管ノードの ID と localhost の公開 URL を catalog の env 既定値として固定。Synergos は `excubitor.catalog.yaml:29-30` (`SYNERGOS_CUSTODY_PUBLIC_URL`、`SYNERGOS_CUSTODY_NODE_ID`)、Curare は同じ UUID を Curare の `excubitor.catalog.yaml:33` (`SYNERGOS_STORAGE_NODE_ID`) に持つ | 既定のまま配備すると全拠点が同じ保管ノードを名乗る。値は運用者が上書きできる catalog の既定値で、構造上の欠落は「複数拠点の配置を表すデータモデルが無い」こと | 高 / P0 | M03 (識別子) + 転送の SPEC-CU-storage-placement + M27 |
| AGD-OP-07 | 設計ギャップ | export は `dataDir()` だけを束ね (`scripts/site/export-data.mjs:10`)、`SYNERGOS_CUSTODY_ROOT` の移設を無視 (`scripts/site/run-control.mjs:16`)。停止の前提を強制しない (`docs/site-operations.md:19,24`)。既定のデータは checkout 内 (`.gitignore:26`) | 成功を報告しながら資産バイトを落とす。ワークステーションと履歴ノードは export 自体が無い | 高 / P0 | M20 (完全性の照合) + SECURITY-M28 (bundle v2) |
| AGD-OP-08 | 設計ギャップ | 永続形式に版付きの移行経路が無い。`RegistrySnapshot` (`synergos-control/src/store/mod.rs:66-72`)、state (`synergos-core/src/version_state.rs:52`)、manifest (`synergos-core/src/manifest.rs:138`) | 長期運用のデータに新しい形式を足せない | 中 / P1 | SPEC-SY-format-skew + SPEC-SY-legacy-import |
| AGD-OP-09 | 設計ギャップ | control の組織は slug とメールの別台帳 (`synergos-control/src/store/mod.rs:9-17`)。全組織が 1 つの保管ノードと鍵空間を共有 | ホスト型の提供が共有ディスクになり、外部スタジオが契約上受け入れない | 高 / P0 | M27: Curare の SPEC-CU-tenant-v1 を control と custody が消費 |
| AGD-OP-10 | 設計ギャップ | Synergos 側に Cernere の ID が全く無い。参加は bearer の招待 (`docs/two-node-operations.md:20`) | 外注先の ID を連携・範囲限定・中央失効できない | 中 / P1 | Curare M16 + SECURITY-M17。Synergos は会員資格を SPEC-SY-membership-token で受ける |
| AGD-OP-11 | 未実装機能 | metrics / トレースが無い。ログは tracing の stdout だけ (`synergos-core/src/main.rs:27-47`)。heartbeat は ID と版だけ (`synergos-core/src/control_heartbeat.rs:103-107`)。GUI の帯域表示は計測されていない (`synergos-core/src/ipc_server.rs:1448`) | 詰まった履歴ノード・満杯のディスク・遅い拠点回線が crunch まで見つからない | 中 / P1 | M28 の Synergos 側: changeset ID と W3C trace context を daemon まで通す |
| AGD-OP-12 | 設計ギャップ | catalog は `autostart: false` / `restart_policy: "no"` (`excubitor.catalog.yaml:17-18`)。control の health は版を返さない (`synergos-control/src/api/mod.rs:100-105`) | 自己回復せず、走っている版が分からない | 中 / P1 | M28 + SECURITY-M32 |
| AGD-OP-13 | 設計ギャップ | 既存ソケットを無条件に削除して bind (`synergos-core/src/ipc_server.rs:85-88`)。`projects.json` は非原子的に書く (`synergos-core/src/project.rs:327-331`)。読込失敗を `if let Ok` で握りつぶす (`synergos-core/src/daemon.rs:163`)。root は絶対パスで保存 (`synergos-core/src/project.rs:313-319`) | 2 つ目の daemon がソケットを奪う。状態を失い、招待し直して数十〜数百 GB を再取得する | 中 / P1 | M25: 単一インスタンス・原子的保存・前世代の保持 |
| AGD-OP-14 | 設計ギャップ | 履歴ノードは catalog に無い (`excubitor.catalog.yaml:5-8`)。heartbeat に役割が無い (`synergos-control/src/api/heartbeat.rs:17-25`)。保持は既定で無制限 (`synergos-net/src/config.rs:288-299`)。回転タイマーは manifest の版だけを keep に渡す (`synergos-core/src/daemon.rs:677-705`)。タグは複製されない | 版の倉庫の存在・範囲・複製数を観測も保証もできない | 中 / P1 | M20 (archive 役の宣言) + M04 (keep-set) + M08 (クラス別保持) |
| AGD-OP-15 | 未実装機能 | GUI が 3 つ (egui、Tauri、Dioxus 管理画面は workspace 外: `Cargo.toml:15-19`) | 運用者向けの一貫した画面が無い | 中 / P2 | M26 の Synergos 半分: egui は SECURITY-M30 の `desktop/` workspace で accesskit を外し、C++ desktop-gui の受入で撤去。運用画面は Curare 側に 1 つ |
| AGD-OP-16 | 設計ギャップ | 配備が LUDIARS 内部基盤に固定 (`docs/site-operations.md:30` の共通 Cloudflare entry) | 無償の on-prem 版と有償のホスト版の区別が作れない | 中 / P2 | M27: 配備プロファイル (hosted / on-prem appliance / hybrid) |

### 2.5 ワークフロー別充足表

✗ = 無い、△ = 一部。「Synergos の機構」は本書の §4、「Curare の機構」は Curare 文書の §4 を指す。

| ワークフロー | 現状 | 阻害している発見 | Synergos の機構 | Curare の機構 |
|---|---|---|---|---|
| アーティストが PSD を checkout、オフラインで編集、check-in | ✗ (ロック無し、上書きの危険) | GW-01, GW-02, DM-05 | M06, M09 (LockAnnounce、読み取り専用ビット) | M05, M08, M09 |
| Unity の scene + `.meta`、prefab 一式の publish | ✗ (途中で切れる) | GW-22, DM-02 | M05 (commit-ipc), M06 (companion を最後に反映) | M05, M08 |
| Unreal の OFPA レベル (10^4〜10^5 の小ファイル) | ✗ (ファイルごとの gossip と行) | GW-19, GW-17 | M05 + 転送の `WANT_MANY` | M11 |
| release からの hotfix ブランチと merge-down | ✗ | GW-03, DM-07 | M04 (ref-sync)、M09 (`retained`) | M04, M12 |
| プログラマが `.prefab` やテキストアセットをマージ | ✗ (git の manifest 衝突) | GW-14 | M02 (別ファイルのポインタ)、M05 (merge driver を sandbox で実行) | M05 |
| UI アーティストが 2 TB のうち 2 % で作業 | ✗ | GW-06, DM-13 | M10 (workspace-view) | M10 |
| ノート PC が 1 週間オフライン後に再接続 | ✗ | GW-10 | M06 + 転送の replication stream | — |
| リードが 40 アセットの変更をサムネイル付きでレビュー | △ (アセットごと、メタデータの差分) | DM-16, GW-08, GW-12 | (なし。tree の差分を提供するだけ) | M12, M14 |
| 外注先が参考資料を受け取りモデルを納品 | △ (ダウンロードだけ) | GW-13, GW-21 | M22 (edge から配る) | M16 |
| Perforce / Git LFS からの移行 | ✗ | GW-16, GW-07 | M23 (LFS agent)、SPEC-SY-legacy-import | M23 |
| IT が 80 台の Windows に配備 | ✗ | OP-03, OP-04, OP-05, OP-13 | M25 | — |
| ML エンジニアが 10^6 ファイルのデータセットを取込み固定 | ✗ (ファイル数に比例する行と呼出し) | AI-01, DM-15, AI-13 | M01 (prolly tree)、M04 | M04, M11 |
| 固定スナップショットから GPU クラスタで学習 | ✗ (300 秒のファイル単位 URL、平坦な名前、Range 無し) | AI-08, AI-07 | M22 (edge の範囲配信) | M22 |
| 10 × 64 GiB のチェックポイントの反復 | ✗ (単発 PUT、1800 秒上限) | AI-07, AI-09 | M22 (利用側の条件) + 転送のチャンク層 | M22 |
| ライセンス・同意付きデータ、オプトアウト、消去 | ✗ | AI-05, AI-06, AI-16, AI-14 | M19 (墓標の消費)、M17 の配信制限 | M17, M19 |
| 取込判定が済んでいないデータの複製 | ✗ (受信物を全部保管) | AI-11 (Curare 側)、DM-10 | M18 (`clean` だけ archive・配信) | M18 |
| 来歴付きの AI 生成アセット | △ (平坦な 1 行) | AI-04, AI-19 | (なし。changeset の `provenance_run` を運ぶだけ) | M15 |
| Codex やパイプラインのエージェントが版を書く | ✗ (人間扱いの長寿命 token) | AI-12 | M05 (`author_principal` を changeset に記録) | M16 |
| 汚染された学習スナップショットの検出 | ✗ | AI-21 | M21 (受信側の照合と隔離) | M21 |
| ホスト型テナントの開始・on-prem スタジオ | ✗ (env 編集と再起動) | OP-09, OP-16, OP-06 | M27 の Synergos 側 | M27 |

### 2.6 根本原因の整理

1. **同一性の不在**: 「このファイルのこの状態」を名指す ID が 3 系統あり、どれも内容と結び付かない (DM-01, DM-04, GW-04, DM-09)。版は `path → u64 version + size + CRC32` で (`synergos-core/src/manifest.rs:35-46`)、転送時に計算した BLAKE3 は捨てられる (`synergos-core/src/exchange/mod.rs:569-572`)。重複排除・lineage・改ざん検知・LFS 互換は全部この上に立つ。
2. **集合単位の不在**: 版はパス単位の連番で、changeset・親・tree・ref が無い (DM-02, DM-05, DM-06, GW-03, GW-22)。原子性・競合検出・rename・削除・ブランチが表せない。複数ファイルの変更は途中で切れたまま届き (GW-22)、同時編集は無言で失われ (DM-05)、auto-pull は未 publish の編集を上書きし (GW-02)、rename は全再転送・削除は復活する (DM-06)。
3. **権威の矛盾**: 索引の正が git (`docs/versioning-design.md:10`) と Curare (Curare `spec/plan/versioning-types.md:3-4`) に分かれ、局所高水位・node ローカルのタグがさらに分散させる (DM-03, DM-07, DM-08, AI-13)。
4. **受信側の規則の不在**: 受け取ったものは条件なく作業ツリーと履歴へ入る。dirty 判定・同期モード・照合・取込判定・クラス別保持が無い (GW-02, AI-21, DM-10, OP-14)。
5. **移行の順序**: `cpp-migration.md:41-43` は manifest format 1・局所高水位・node ローカルのタグを互換契約として保存すると書き、手順 3 (`cpp-migration.md:56`) で C++ に移植すると、上の欠陥が parity 要件として固定され、修正のたびに 2 回目の移行が要る (DM-17, GW-20, AI-20)。

### 2.7 他テーマとの相互参照

- セキュリティ: AGD-OP-01 の大小文字区別の保護は SPEC-SY-path-policy (SECURITY-M13) が契約を持つ。AGD-AI-16 の素の認可は SPEC-SY-membership-token (SECURITY-M03)。AGD-AI-21 の転送完全性の穴は SEC 側の受信規則 (SPEC-SY-pull-only-receive) と同じ根を持つ。本書は「汚染された候補を版にしない」照合だけを足す。取込判定の作業ツリー展開ゲートは SECURITY-M24 で、本書の M18 は履歴・edge の複製と配信だけを足す。AGD-OP-07 の bundle 形式は SECURITY-M28。AGD-OP-15 の workspace 分割は SECURITY-M30。
- 転送効率: AGD-AI-07 (大きいオブジェクト)、AGD-GW-19 (小ファイル)、AGD-GW-10 の resume は転送効率文書の SPEC-SY-txf2・SPEC-SY-chunk-store・SPEC-SY-custody-transfer-v2 が本体。本書は利用側の条件と、HEAD に Curare 署名の RefUpdate を載せる追補 (X-AG3)、`sync_mode` の意味 (TRANSFER-M30 の上位の契約、X-AG11) を持つ。
- Curare: `curare` だけの発見 (AGD-AI-01, -03, -04, -05, -10, -11, -12, -14、AGD-DM-11, -15, -16, -19、AGD-GW-08, -09, -12, -13, -21) は Curare 文書が持つ。Synergos はそこで定義される契約 (§4.23) を消費する。

## 3. 設計判断と選択肢の比較

4 軸は HARNESS §3.5 に従い、各軸 1〜5 で **5 が常に良い側**。L = AI 学習量 (5 = 新規に学ぶことが最も少ない)、W = 作業コスト (5 = 最も安い)、R = 解決度、A = 主目的との一致度 (AI 開発・ゲーム開発・セキュリティ・転送効率を 1 つの解で満たす)。採用案は太字。負けた案は代替案として残す。**[変更提案 (neco 再判断)]** は既存の neco 決定を変えるもので、§7 の再判断事項になる (Curare 文書も同じ表記に揃える、§5.3)。

### 3.1 D-A01 版のオブジェクトモデルと履歴の権威 (DM-01, DM-03, DM-04, DM-08, GW-04) [変更提案 (neco 再判断) 2026-08-16、2026-09-08 の精緻化]

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. 1 つの CAS グラフ (Blob/Tree/Changeset/Ref、BLAKE3)。Curare の ref log が権威、Synergos は署名付きの複製、git は任意のポインタ** | 3 | 2 | 5 | 5 |
| B. git の manifest を索引のまま、Curare を git 対応にする | 4 | 3 | 3 | 1 (「git を前提にしない」を覆す) |
| C. 両系統を残して ID の対応表を足す | 4 | 3 | 2 (正が 2 つ残る) | 2 |
| D. Synergos ローカルの journal を権威にし Curare が写す | 3 | 3 | 3 | 1 (権威が 2 つ) |

**A を採用**。

- 2026-08-16 から維持: 同一性はファイル全体。通常ノードはファイル全体を受け取る。履歴ノードのフラグ (クラス対応にし RetentionAck で裏付ける)。索引の権威は 1 つで、第二の履歴系を持たない。
- 2026-08-16 から変更: 索引の権威を git から Curare の ref log へ移す。`docs/versioning-design.md:10` の「履歴の「索引」は git 側」と `:28` の「git に一本化する」を変える。
- 2026-09-08 から維持: 版・クラス・許可・受理は Curare が所有し、受理に副署する。転送の成功は版を作らない。
- 2026-09-08 から変更: Synergos は checkout と差分のために tree の符号化を解釈する。許可の解釈はしない (`spec/feature/opaque-custody.md:5` の「制作履歴や承認を作らない」は維持)。

### 3.2 D-A02 Curare の無い Synergos

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| A. git / format 1 モードを恒久に残す | 5 | 4 | 2 | 2 |
| B. Synergos daemon に最小の ref 権威を埋め込む | 3 | 2 | 3 | 1 (2026-08-16 の案 C を作り直す) |
| **C. オブジェクトモデル (SPEC-SY-object-model) は Curare の ref 権威を必須にする (ホスト型か、M27 の on-prem appliance)。format 1 は旧形式の読取・変換経路として残し、A5 まで Rust daemon だけが書く** | 3 | 3 | 5 | 5 |

**C を採用**。Curare の無い LAN だけのスタジオは新機能を失うので、neco の判断事項 (Q-A2)。

### 3.3 D-A04 競合モデル (DM-05, GW-01, GW-02, GW-14)

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. 楽観的な ref CAS + パス単位の 3-way 検査 + マージ不能な型には方針でロック** | 3 | 3 | 5 | 5 |
| B. すべてのバイナリに排他ロックを必須 (全体に P4 の `+l`) | 4 | 4 | 3 (オフライン作業とデータセットを止める) | 3 |
| C. 後勝ち + 競合コピー | 5 | 5 | 1 | 1 |

**A を採用**。Synergos は commit-ipc で `base_cs` を必ず送り、409 `path_conflict` を受けて `Conflict` を作業ツリーに残す (§4.5)。

### 3.4 D-A05 ロックの権威 (GW-01) — Synergos は cache 側

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. Curare が権威のブランチ対応 `asset_locks` (Unity Smart Lock の「retained」)、LFS locking の wire、soft モード。Synergos は署名付き LockAnnounce を cache として gossip し、読み取り専用ビットを立てる** | 4 | 3 | 5 | 5 |
| B. Synergos 内の P2P ロック合意 | 2 | 2 | 3 (オフラインで split brain) | 3 |
| C. LFS のパスロックだけ (単一ブランチ) | 5 | 4 | 3 | 3 |
| D. soft ロックだけ (Diversion 型) | 5 | 5 | 2 | 3 |

**A を採用**。D は `lock = "soft"` として残す。`docs/versioning-design.md:337-339` の「ロック情報は gossip で配り、ロック中ファイルの publish を他ノードで拒否する」は、gossip を cache に、拒否の判定を Curare に移す形で引き継ぐ。

### 3.5 D-A07 部分ワークスペース (DM-13, GW-06)

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. サーバが知る View (Curare) を Synergos の workspace が参照し、読取 ACL と交差させる** | 4 | 3 | 5 | 5 |
| B. 同期される `.synergosview` ファイル | 4 | 4 | 3 | 3 |
| C. クライアント側のフィルタだけ | 5 | 5 | 2 | 2 |
| D. 遅延 VFS マウント (ProjFS / FUSE) | 2 | 1 | 4 | 3 |

**A を採用**。

- **VFS (D) の判断は本書が持つ** (転送効率文書の非ゴール表と D-T17 は VFS を本書に委ね、自らは土台の SPEC-SY-object-get を出すだけと書く)。現時点の判断は「保留」。再検討の契機は、(1) SPEC-SY-object-get と TXF2 の範囲取得が T4 で稼働し、(2) Selective モードの G-A5 計測 (§6.2 S2) が出ていること。判断者は neco。
- **同期モードの意味の正本は本書の SPEC-SY-worktree-apply** で、Full / Manual / Selective がそれぞれ何を作業ツリーに反映するかを決める。転送効率の TRANSFER-M30 (`sync_mode` の尊重) は、その下で「何を取得するか」を絞る wanted-set の実装である (§4.6、X-AG11)。

### 3.6 D-A16 重複排除と横断再利用の範囲 (AI-09, DM-12) — Synergos は消費側

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. プロジェクト単位の重複排除領域 (security D-10/D-11)。プロジェクト横断は tree の `link` エントリを参照先の capability で読む。組織範囲は opt-in** | 3 | 3 | 5 | 5 |
| B. テナント単位の CAS | 4 | 3 | 5 | 3 (security D-11 と衝突。プロジェクト間の存在オラクル) |
| C. 全体の収束型重複排除 | 3 | 3 | 4 | 1 (テナント間のオラクル) |

**A を採用**。領域の定義は SPEC-CU-object-registry (`sha256 ↔ b3` の対応と登録) + SPEC-SY-envelope-encryption (保管名の導出) で、転送効率文書の登録表に従う。以前の検討で使った ID `SPEC-SY-dedup-domain` は転送効率文書で廃止され、この 2 つに置き換わった。Curare 文書の参照表も同じ置換を行う (§5.3)。

### 3.7 D-A17 エンジン / DCC 連携の順序 (GW-07, GW-16, AI-17)

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| **A. LFS の wire 互換を先に (batch + locks + Synergos の custom-transfer agent)、次にローカル broker 上のネイティブ adapter** | 4 | 3 | 5 | 5 |
| B. ネイティブ adapter だけ | 3 | 2 | 3 (採用が遅い) | 3 |
| C. LFS だけ | 5 | 4 | 3 (エディタ内のプレビューや stream が無い) | 3 |

**A を採用**。Synergos 側の担当は SPEC-SY-lfs-transfer-agent と SPEC-SY-local-broker。

### 3.8 D-A18 C++ 移行との順序 (DM-17, GW-20, AI-20、オーナー決定 6)

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| A. format 1 を今移植し、後で移行する | 4 | 2 (codec 2 つ、移行 2 回) | 2 | 2 |
| B. 新模型を Rust で実装してから移植する | 4 | 1 | 4 | 1 (決定 6 に反する) |
| **C. 手順 3 の前に互換契約を改訂。format 1 は旧形式の読取・変換、object model を C++ の規範にする。vector が通るまでネイティブの versioning / history の移植を保留し、契約に関係しない C++ 作業 (framing、transport、custody の基本部品) は続ける** | 3 | 3 | 5 | 5 |

**C を採用**。`cpp-migration.md` の改訂文は §4.27 の 3 文書共通の改訂表。

### 3.9 D-A19 デスクトップの外殻と quick-xml の経路 (OP-15、オーナー決定 5)

判断の正本は security 文書の D-13 (同文書 §3.13) と SECURITY-M30 (同文書 §4.27) で、本書はそれに従う。security は次の 5 案を比べ、E を採った。

| 選択肢 (security D-13 と同じ) | L | W | R | A |
|---|---|---|---|---|
| A. `audit.toml` で除外 | 5 | 5 | 1 | 1 (決定 5 で却下) |
| B. 赤のまま許容 | 5 | 5 | 1 | 1 |
| C. eframe を `default-features = false` にして `accesskit` を外す | 4 | 4 | 4 | 3 |
| D. `synergos-gui` を今 workspace から外す | 4 | 5 | 5 | 2 (C++ desktop-gui が無い時点で `cpp-migration.md:48` の受入面と parity の基準を消す) |
| **E. Cargo を `server/` と `desktop/` の workspace に分け、`synergos-gui` を `desktop/` へ移して C を適用する。egui バイナリは C++ desktop-gui が受入ゲートを通るまで build できる状態に保ち、その時点で撤去する** | 4 | 4 | 5 | 5 |

**E を採用 (SECURITY-M30 のとおり)**。オーナー決定 5 の問い「依存経路は要るか」への ai-game の答えは次のとおり。

- **eframe の accesskit 経路は製品として要らない**。`synergos-gui` は accesskit を外して `desktop/` workspace へ移り、C++ desktop-gui が受入ゲートを通った時点で撤去する。security Q3 で egui GUI の利用者がいないと分かれば、それより前に撤去してよい。
- 経路の逆依存 (`quick-xml 0.30.0` ← `zbus_xml` ← … ← `eframe` ← `synergos-gui`) の確認と audit ゲートは SECURITY-M30 が持つ (`Cargo.lock:9374-9379`)。
- 「Tauri が暫定のデスクトップ面」は **推定** である (security D-13 と同じ扱い。`docs/operations-readiness.md:36` は egui と Tauri を同じ状態で並べる)。security Q3 で確かめる。
- ai-game の追加要件: accesskit が与えていたアクセシビリティは、置き換える外殻 (C++ desktop-gui) の明示要件にする (スクリーンリーダー対応)。外殻は SPEC-SY-local-broker (SPEC-SY-ipc-v2 上) だけを使う。

egui にしかない画面 (競合・設定・転送要求・取消) の扱いは別の製品判断 (Q-A14) で、上の E を前提に比べる。

| 選択肢 (Q-A14) | L | W | R | A |
|---|---|---|---|---|
| **A. egui の画面は `desktop/` に残して使い続け、同じ画面を C++ desktop-gui の要件に書く** | 5 | 5 | 4 (A0〜A3 の利用者は egui を使える) | 5 (重複実装が無い) |
| B. 同じ画面を今 Tauri へ移す | 3 | 2 | 4 | 2 (C++ 外殻と合わせて 3 回作る) |
| C. 画面を作らず CLI だけにする | 5 | 5 | 2 | 2 |

既定は A。security Q3 で egui の利用者がいない、かつ Tauri が暫定面だと確かめられた場合だけ B を再検討する。

### 3.10 D-A20 大きいオブジェクトと CDC (AI-07、オーナー決定 3) — 利用側の見解

判断の正本は転送効率文書の D-T02 / D-T03 で、「Synergos 本体に入れる案」と「チャンク保管を独立モジュールにする案」を 4 軸で比べ、独立モジュール `synergos-chunkstore` (SPEC-SY-chunk-store) を採った。[変更提案 (neco 再判断) 2026-08-16] もそちらが持つ (transfer Q1)。本書の立場:

- ゲームのメディアはファイル全体のまま (圧縮テクスチャ、FBX、`.uasset`、PSD は方針の rule で `chunk=false`)。
- チャンク層に入るのは、SPEC-CU-project-policy の rule が `chunk=true` で、かつファイルが 8 MiB 以上 (転送効率の SPEC-SY-format-policy の閾値) のパスだけ。クラス名で選ばない (方針のクラスは `working|submission|reproducible|derived|dataset` の 5 つで、`model` や `checkpoint_series` は Curare の `datasets.kind` の値)。
- 同一性はファイル全体の BLAKE3 のまま。メンバーはファイル全体を受ける。テンソル単位の重複排除は対象外。

| 選択肢 (ai-game の見方) | L | W | R | A |
|---|---|---|---|---|
| A. ファイル全体だけ (2026-08-16) | 5 | 5 | 1 (再利用 0 %、5 GB 超の経路が無い) | 2 |
| B. daemon と custody の中で CDC | 3 | 2 | 4 | 3 |
| **C. 独立したチャンク保管モジュールを、方針の rule で選んだパスだけに使う** | 3 | 3 | 4 | 5 |

### 3.11 D-A22 正規の符号化 (セキュリティの単一署名封筒との整合)

| 選択肢 | L | W | R | A |
|---|---|---|---|---|
| A. 全オブジェクトを CBOR、署名も CBOR 上 | 3 | 3 | 4 | 2 (セキュリティの単一 JCS 封筒を壊す) |
| B. tree を含め全オブジェクトを JCS | 4 | 4 | 3 (10^6 エントリの tree が膨らむ) | 3 |
| **C. tree (ハッシュするだけで署名しない) は決定的 CBOR (RFC 8949 §4.2)。署名するもの (changeset、RefUpdate、LockAnnounce、purge list、プロジェクト記述子、証明 payload) は SPEC-SY-signed-envelope の JCS (RFC 8785) で、tree ID をフィールドとして持つ** | 3 | 3 | 5 | 5 |

**C を採用**。changeset ID は署名を除いた JCS 本体の BLAKE3。

### 3.12 既存決定への変更提案の一覧 (Q-A1 / Q-A12)

| 既存の記述 | 変更提案 | 判断事項 |
|---|---|---|
| `docs/versioning-design.md:10`「履歴の「索引」は git 側 (`.synergos/manifest.json` をコミット)」、`:28`「git に一本化する」、`:346`「`manifest.json` は git にコミットする」 | 索引の権威は Curare の ref log。git にコミットするのは任意の 1 行ポインタ `.synergos/head.json` (SPEC-SY-workspace-pointer)。`manifest.json` は転送効率の manifest v2 で、X-AG1 が受理されればノードローカルの実体化索引になる | Q-A1, Q-A12 |
| `docs/versioning-design.md:191`「観測最大版は node-local の `.synergos/state.json` に永続化」、`:205-209` のノードローカルのタグ | 版番号は表示専用で wire に載せない。タグは Curare の不変の ref。局所高水位とノードローカルのタグは旧形式の取込み (SPEC-SY-legacy-import) でだけ読む | Q-A1 (security Q1 と合わせる) |
| `cpp-migration.md:39-40`・`:41-43`・`:44-47`・`:48`、`native-contracts.md:15-24` の SPEC-SY-native-metadata | §4.27 の 3 文書共通の改訂表 | Q-A1, Q-A12, security Q1 / Q3, transfer Q1 / Q2 |
| `spec/feature/opaque-custody.md:5` の「制作履歴や承認を作らない」の解釈 | 文言は維持。Synergos は tree を解釈して checkout と差分を行うが、許可・クラス・受理は判断しない (精緻化) | Q-A1 |
| `docs/versioning-design.md:104`「相互の同期はしない」 | 保護 ref の閉包を全ノード同一の keep-set にし、`min_copies` を満たすまで GC しない (SPEC-SY-replica-accounting) | Q-A1 (SEC の SPEC-SY-retention-ack と同じ方向) |
| 転送効率の SPEC-SY-manifest-v2「(Sy daemon → git, …) … Readers accept formats 1 and 2 forever」 | manifest v2 は git にコミットせず、権威でなく、SPEC-SY-object-model から再構築できるノードローカルの実体化索引にする (X-AG1)。ポインタは別ファイルなので、X-AG1 の採否にかかわらず format 番号は衝突しない | Q-A12 |

## 4. 再設計の内容

### 4.0 読み方と機構の所有表

- 機構の見出しは AIGAME-Mxx (両 repo 共通の機構 ID)。本書は Synergos 正本の契約 13 件 (SPEC-SY-object-model、-ref-sync、-workspace-pointer、-commit-ipc、-worktree-apply、-workspace-view、-lock-announce、-replica-accounting、-tombstone、-local-broker、-lfs-transfer-agent、-legacy-import、-format-skew) の全文を持つ。SPEC-SY-workspace-pointer は X-AG1 が受理されるまで **[変更提案]** で、正本の文としては扱わない (§4.2)。
- **区分** は `spec/plan/redesign-analysis/HANDOFF.md` §6 に従う。[契約変更] = C++ 移行の互換契約 (`cpp-migration.md` "Compatibility contracts" と `native-contracts.md` の `[id:]`) を変える。[実装要件] = 契約は変えず、C++ 実装が最初から避ける非効率・欠陥 (NR-AG-*)。[運用] = 配置・設定・文書・ゲート。
- **すべての Synergos 機構は C++ 移行の互換契約として指定し、Rust 現行実装には入れない** (オーナー決定 6)。実装の置き場所は `<module>/native/` (`cpp-migration.md:26-27`)。Rust 側の変更は SECURITY-M30 の workspace 分割だけで、その作業は security 文書が持つ。
- 契約の種別: W = wire / 保存形式の契約変更、A = 追加の新契約、X = 外部依存 (Cernere)。本書が正本の契約は、定義を 1 回だけ原文 (英語) で引用する。他の正本の契約は §4.23 で 1 回だけ引用するか、相手テーマ文書を参照する。
- **担保手段** は AIFormat `HARNESS.md` の「ハーネスの原則」(`HARNESS.md:13-29`) の 3 区分 (記憶 / フック強制 / ローカル検証ゲート) に、実行時の強制 (ランタイム) を加えた 4 種。ローカル検証ゲートは Revisor のマージ前ケース。フックは object-model の codec と vector の変更時に vector 試験を起動するハーネスの事前検査 (§4.1) で使う。それ以外の機構は、検査がネイティブ実行時か Revisor の多ノード試験でしか成り立たない (ファイル単位の編集時点で判定できる規則が無い) ので、フックを置かない。
- フェーズは A0〜A5 (§5)。security の P0〜P6、transfer の T0〜T5 と区別する。

機構の所有表 (両文書の §4.0 に同じ表を置く。Curare 文書はこの表を写す、§5.3):

| 機構 | 契約の正本 | Synergos 半分 (本書) | Curare 半分 (Curare 文書) |
|---|---|---|---|
| M01 オブジェクトモデル | SPEC-SY-object-model (Sy) | §4.1 C++ codec・vector | §4.5 TS 実装・権威表 |
| M02 C++ 互換契約・ポインタ | SPEC-SY-workspace-pointer、-legacy-import (Sy) | §4.2 | — |
| M03 記述子と識別子 | SPEC-CU-project-descriptor (Cu) | §4.3 消費 | §4.6 |
| M04 ref 権威と複製 | SPEC-CU-ref-authority (Cu)、SPEC-SY-ref-sync (Sy) | §4.4 | §4.7 |
| M05 changeset commit | SPEC-CU-path-conflict (Cu)、SPEC-SY-commit-ipc (Sy) | §4.5 | §4.8 |
| M06 作業ツリー反映 | SPEC-SY-worktree-apply (Sy) | §4.6 | — |
| M07 パスは tree の属性 | SPEC-SY-object-model の規則 (Sy) + SPEC-SY-path-policy (security) | §4.7 | §4.9 |
| M08 方針とクラス | SPEC-CU-project-policy (Cu) | §4.8 消費 | §4.10 |
| M09 ロック | SPEC-CU-lock-service (Cu)、SPEC-SY-lock-announce (Sy) | §4.9 | §4.11 |
| M10 view と shelf | SPEC-CU-view-spec (Cu)、SPEC-SY-workspace-view (Sy) | §4.10 | §4.12 |
| M11 規模の索引 | SPEC-CU-dataset-snapshot (Cu) | — | §4.13 |
| M12 レビューと release | SPEC-CU-review-range (Cu) | — | §4.14 |
| M13 link | SPEC-SY-object-model の `link` (Sy) | §4.11 実体化 | §4.15 |
| M14 派生キャッシュ | SPEC-CU-derived-cache (Cu) | (M08 のクラスで保管先を選ぶだけ) | §4.16 |
| M15 来歴 | SPEC-CU-provenance-run (Cu) | — | §4.17 |
| M16 主体 | SPEC-CU-principal-model (Cu) | — | §4.18 |
| M17 権利と用途 | SPEC-CU-rights-purpose (Cu) | §4.19 配信制限 | §4.19 |
| M18 取込判定ゲート | SPEC-CU-ingest-verdict (security) | §4.20 複製と配信の門 | §4.20 |
| M19 消去 | SPEC-CU-erasure-ledger (Cu)、SPEC-SY-tombstone (Sy) | §4.12 | §4.21 |
| M20 複製の勘定 | SPEC-SY-replica-accounting (Sy) | §4.13 | §4.22 |
| M21 証明と汚染 | SPEC-CU-snapshot-attestation (Cu) + 受信規則 (Sy) | §4.14 | §4.23 |
| M22 学習クラスタの読み出し | SPEC-CU-snapshot-gateway (Cu) | §4.15 edge | §4.24 |
| M23 ツール連携 | SPEC-CU-lfs-compat、-import-origin (Cu)、SPEC-SY-lfs-transfer-agent、-local-broker (Sy) | §4.16 | §4.25 |
| M24 検索 | (なし) | — | §4.26 |
| M25 ワークステーション daemon | SPEC-SY-format-skew、-local-broker (Sy) | §4.17 | — |
| M26 外殻と運用画面 | (なし。作業は SECURITY-M30) | §4.18 M26 の Synergos 半分 (egui / desktop) | §4.27 運用コンソール |
| M27 テナントと配置 | SPEC-CU-tenant-v1 (Cu) | §4.21 | §4.28 |
| M28 運用性 | (なし) | §4.22 | §4.29 |
| M29 製品要件 | (なし) | (§6 のシナリオを Augur に登録) | §4.31 |

### 4.1 AIGAME-M01 統一オブジェクトモデル、ID、vector

- 区分: [契約変更] (SPEC-SY-object-model、W)。対象発見: AGD-DM-01, -04, -09, -18, AGD-GW-04, AGD-AI-09, AGD-AI-15。フェーズ A0 (契約と vector)、A3 (C++)。
- 目的: Synergos と Curare のすべての状態を 1 つの ID で名指す。7 つの重複表現 (AGD-DM-18) と 2 つの blob 平面 (AGD-DM-09) を畳む。
- 契約 SPEC-SY-object-model (正本: 本書):

  > The version-control object graph. Every Synergos and Curare state is named by one id.
  > • **Blob:** id `b3:<blake3-256 hex>` (multihash 0x1e on the wire), with header `{size, digests{blake3, sha256}, chunk_map_id?}`. The digests come from one pass per SPEC-SY-content-identity. There is no separate `bao_root`.
  > • **Tree:** deterministic CBOR (RFC 8949 §4.2.1). Entries are sorted by NFC name bytes: `{name, kind: file|exec|tree|link, id, size, entry_id (UUIDv7, stable lineage), class, policy_rule_id, attrs?{lock_mode, origin?}}`. Nodes are prolly-split with content-defined boundaries (target 64 KiB, max 4,096 entries), so equal subtrees share ids and diffs cost O(changed). Tree id = BLAKE3 of the canonical bytes. A `link` entry is `{scope, ref|changeset, subpath, mode: pinned|track}`.
  > • **Changeset:** JCS body `{tree, parents[0..n], scope, author_principal, producer_node?, intent: save|submission|import|merge|erase|shelve, message, operation_id, provenance_run?, policy_blob, created_at}`. Id = BLAKE3(JCS body). Detached signatures under SPEC-SY-signed-envelope ctx `changeset`: `producer_sig` (node key) and `acceptor_sig` (Curare, on acceptance).
  > • **Rules:** deletion = an absent entry; rename = same blob + same `entry_id` under a new path. Typed API prefixes are `blob:`, `tree:` and `cs:`. Version numbers are display-only and never sent on the wire. CRC32 survives only as a pre-check. No absolute local path appears in any gossiped structure.
  > • **Vectors:** ≥40 golden encodings live in Synergos `spec/vectors/object-model/` and run in the C++, TS and Rust-reader CI.

- 仕組み:
  - データモデル: 上の 3 種のオブジェクトと `link`。Blob の digest は転送の SPEC-SY-content-identity の 1 回の走査で作る。tree は prolly 分割で、同じ部分木は同じ ID になる。
  - 旧表現の扱い: `RootCatalog` (`synergos-net/src/catalog/types.rs:7`) → RefUpdate、`ChainBlock` (`synergos-net/src/chain/file_chain.rs:9`) と `ChunkDag` (`synergos-net/src/content/chunker.rs:22`) → 移植しない (チャンク地図は SPEC-SY-chunk-store が持つ)、`HistoryIndex` (`synergos-core/src/history/index.rs:56`) → changeset から導出する cache、`Tag` → ref。
  - blob 平面の統一: 履歴の CAS、custody のオブジェクト、Curare の `blob_id` はすべて `b3:` ID を使い、プロジェクト単位の重複排除領域に置く。保管名は SPEC-SY-envelope-encryption で導出し、平文のハッシュを保管名にしない。
  - 権威表 (両 repo の data-schema 節に載せる): ref と方針 → Curare。バイト → 自己証明する CAS の複製 (どの複製も権威ではない)。配置の受領証 → Synergos custody。作業ツリーの状態 → ノードローカルで使い捨て。
  - vector: `spec/vectors/object-model/` に、非 BMP の名前、NFC/NFD の組、空 tree、4,096 エントリ境界、10^5 エントリの prolly tree、`link` 各種、親 0/1/2 の changeset、署名の分離を含める。契約文の「CI」は Revisor のマージ前ケースを指す (オーナー決定 2)。
- 不変条件: wire と gossip に版番号と絶対パスを載せない。同じ内容は同じ ID。tree の符号化は 1 通り (決定的 CBOR)。
- 失敗時: 非正規の符号化 (順序違い、重複名、NFC でない名前) は `object.non_canonical` で拒否する。未知の `kind` は `object.unknown_kind`。
- 変更対象: 新設 `synergos-core/native/object_model/` (codec と prolly 分割)、`spec/vectors/object-model/`。Rust 側は旧形式リーダーの vector 試験だけを Revisor ケースに登録する (実行時コードは変えない)。
- 担保手段: ローカル検証ゲート (C++・TS・Rust リーダーの vector 一致、G-A19)、フック (codec か `spec/vectors/object-model/` を変える編集のあと、ハーネスの事前検査が vector 試験を走らせる。Anatomia の supply→verify に検査規則として足す)、ランタイム (codec が唯一の入口)。
- ドメイン: mesh-core に `object-model` を足す。

### 4.2 AIGAME-M02 C++ 互換契約の再仕様化と順序、git ポインタ

- 区分: [契約変更] (SPEC-SY-workspace-pointer は W、SPEC-SY-legacy-import は A) + [実装要件] (NR-AG-1〜4)。対象発見: AGD-DM-17, AGD-GW-20, AGD-AI-20, AGD-AI-15, AGD-DM-03, AGD-GW-14, AGD-GW-17。フェーズ A0。
- 目的: 手順 3 で format 1 を規範として固定するのを止める。git に入れるファイルを、衝突しないポインタに変える。
- 契約 SPEC-SY-workspace-pointer (正本: 本書。**X-AG1 が転送効率文書に受理されるまで [変更提案]**):

  > The optional git-committed workspace pointer is a separate file, `.synergos/head.json`, holding one line `{pointer_format:1, scope, ref, cs, tree}`. Committing it is optional, and it is trivially mergeable (a `synergos git-merge-pointer` driver plus a shipped `.gitattributes`). It never reuses the `.synergos/manifest.json` format numbers: `manifest.json` format 1 is read and converted only (SPEC-SY-legacy-import), and format 2 is transfer's SPEC-SY-manifest-v2, the node-local materialized index of a tree (X-AG1). This replaces cpp-migration "Versioning: preserve `.synergos/manifest.json`, local high-water marks, … tags".

- 契約 SPEC-SY-legacy-import (正本: 本書):

  > `synergos migrate --to object-model --map <legacy_id>=<project_uuid> [--dry-run]` reads manifest v1, `state.json` and the history index. It uploads blobs through batch stage and emits one `import` changeset per project plus one per history version chain (linear parents). Tags become *proposed* aliases that need operator approval. The legacy-id → UUID mapping needs operator approval.

- 仕組み:
  - ポインタ: `.synergos/head.json` に `{pointer_format:1, scope, ref, cs, tree}` の 1 行 JSON。`synergos git-merge-pointer` は 2 つのポインタのうち ref log 上で後の `cs` を採り、同じ ref で分岐していれば `cs` の両方を残して `Conflict` として報告する。`.gitattributes` に `/.synergos/head.json merge=synergos-pointer` を同梱する。
  - format 番号を分けた理由: 転送効率の SPEC-SY-manifest-v2 は `manifest.json` format 2 を「format 1 + per-entry `blake3`」と定める。ポインタが同じファイルの format 2 を名乗ると、2 つの文書が同じ番号に別の意味を与える。ポインタを別ファイルにすれば、X-AG1 が受理されない場合 (manifest v2 を git にコミットし続ける場合) でも番号は衝突しない。
  - 実体化索引: エントリ単位の一覧はノードローカルの SPEC-SY-manifest-v2 (転送) に置き、git には入れない (X-AG1 の提案)。
  - 旧形式取込み: 手順と対応表の承認は運用者が行う。dry-run の差分報告を先に出す。
  - 順序 (D-A18): `cpp-migration.md` 手順 3 を「framing・transport・custody の基本部品」と「project / history サービス」に分け、後者は object-model の vector が C++ で通るまで保留する。Completion gate の parity を「format 1 取込みの parity」と「新契約の振る舞い試験」に分ける (§4.27)。
- ネイティブ実装要件 (契約は変えない):

  | ID | 要件 | 対象発見 |
  |---|---|---|
  | NR-AG-1 | 全 manifest の書換えをしない。JSON の履歴索引を使わず、転送効率の SPEC-SY-local-index の保管 (ストアの選択は転送効率文書が持つ) に履歴索引の表を置く | AGD-AI-15, AGD-GW-17 |
  | NR-AG-2 | 起動時の全 CRC 走査をしない (`synergos-core/src/restore.rs:44`)。stat cache (SPEC-SY-local-index) を使う | AGD-GW-17 |
  | NR-AG-3 | IPC の応答は stream にし、結果全体の 1 MiB 上限を持たない。frame 自体の上限は維持 (SPEC-SY-ipc-v2) | AGD-AI-15, AGD-GW-17 |
  | NR-AG-4 | publish 時にバイトを 1 回だけ読む (内容 ID・CRC・転送を 1 走査で。転送効率と共有) | AGD-AI-15 |

- 不変条件: `manifest.json` format 1 は永久に読める。新形式の書込は SPEC-SY-format-skew の条件を満たすまで行わない。C++ は format 1 を書かない。ポインタと `manifest.json` は format 番号を共有しない。
- 失敗時: ポインタの `cs` が ref log に無い場合は `pointer.unknown_cs` を返し、作業ツリーを変えない。取込みで対応表が未承認なら `migrate.mapping_unapproved` で止まる。
- 変更対象: `spec/plan/cpp-migration.md`、`spec/feature/native-contracts.md` (§4.27 / §4.28)、`docs/versioning-design.md` (変更提案の注記)、新設 `synergos-core/native/workspace_pointer.cpp`、旧形式取込みの C++ ツール。既存の `synergos-core/native/manifest_codec.cpp` は取込み専用の読取に限る。
- 担保手段: 記憶 (改訂した互換契約)、ローカル検証ゲート (2 つのポインタのマージ試験 S3、format 1 取込みの parity 試験 S4)。

### 4.3 AIGAME-M03 プロジェクト記述子と識別子 (Synergos 側: 消費)

- 区分: [契約変更] (Curare 正本の SPEC-CU-project-descriptor を消費。Synergos 側は `project_ref` の導出を変える)。対象発見: AGD-DM-14, AGD-OP-06 (識別子の部分)。フェーズ A3。
- 目的: 自由文字列のプロジェクト ID (`synergos-core/src/project.rs:683-689`) を gossip topic・DHT キー・保管庫名に使うのをやめる。security が委ねた `project_ref` と Curare space の対応 (security Q8、transfer Q10、X-AG5) を確定する。
- 仕組み: Curare 紐付けのプロジェクトは、Curare が署名した記述子からだけ開く。`project_ref = blake3("curare" ‖ org_uuid ‖ scope_uuid)` で、gossip topic と DHT キーはここから導く。P2P だけの旧プロジェクトは security の `blake3(org ‖ host_pubkey ‖ local_id)` のまま。自由文字列は表示専用。既存 ID の対応付けは運用者が 1 回だけ行う (SPEC-SY-legacy-import の `--map`)。ノード ID はセットアップ時に作り、catalog の env 既定値に置かない (Synergos `excubitor.catalog.yaml:30` の `SYNERGOS_CUSTODY_NODE_ID` と、Curare `excubitor.catalog.yaml:33` の `SYNERGOS_STORAGE_NODE_ID` の固定 UUID を廃止。配置のデータモデルは転送の SPEC-CU-storage-placement と M27)。
- 記述子の `path_policy` (unicode・大小文字・長さ・予約名) は §4.7 のパス正規化に、`ref_log_verify_keys` と `verify_keys` は §4.4・§4.9・§4.12・§4.14 の署名検証に、`policy_blob` は §4.8 に渡す。
- 検証鍵の出所 (X-AG13、Curare Q-CU-2 と同じ問い): ctx `ref-log` は記述子の `ref_log_verify_keys[{kid, pub}]`。ctx `lock-announce`・`attestation`・`purge-list` は記述子の追加欄 `verify_keys{<ctx>: [{kid, pub, not_after?}]}` (Curare 文書 §4.6 の提案と同じ形)。purge list は org 全体で 1 つなので、その鍵は org の 1 鍵 (Vault Transit `curare-purge-list`) で、権威は SPEC-CU-tenant-v1 の `verify_keys.purge-list`。Curare は同じ公開鍵を org の全プロジェクトの記述子に写す。記述子を持たないノード (custody・control 専用) は SPEC-CU-tenant-v1 の取得 (M27) で鍵を得る。
- 鍵の切替: 各 ctx は現行 kid と直前の kid を同時に受け、直前の kid は `not_after` まで有効。未知の kid を見たら記述子を 1 回だけ取り直し、それでも無ければ拒否する。
- 不変条件: 記述子の署名 (ctx `project-descriptor`) が検証できないプロジェクトは開かない。
- 失敗時: `descriptor.bad_signature` / `descriptor.unknown_org` で open を拒否。未知の kid は `sig.unknown_kid{ctx, kid}`。
- 変更対象: `synergos-core/src/project.rs` 相当の C++ (`synergos-core/native/`)、gossip topic の導出 (`synergos-net` の C++ 移植先)。
- 担保手段: ランタイム、ローカル検証ゲート (同名の 2 プロジェクトが別 topic になる試験 S14、鍵切替の試験 S17)。

### 4.4 AIGAME-M04 ref の複製 (ref-sync) と keep-set

- 区分: [契約変更] (SPEC-SY-ref-sync、W)。対象発見: AGD-DM-03, -07, -08, AGD-GW-03, AGD-AI-13, AGD-OP-14 (keep-set)。フェーズ A3。
- 目的: node ローカルのタグと manifest 由来の keep (`synergos-core/src/daemon.rs:701-705`) を、全ノードで同一の保護集合に置き換える。
- 契約 SPEC-SY-ref-sync (正本: 本書):

  > Synergos holds a signed **replica** of Curare refs.
  > • It pulls `GET /api/v1/scopes/{id}/ref-log?after=seq&limit=1000` (keyset paging; the response carries the next `after`). Each entry is a JCS `RefUpdate{scope, ref, old_cs, new_cs, seq, principal, reason}` signed by Curare under ctx `ref-log` (X-AG7). Keys come from SPEC-CU-project-descriptor.
  > • Gossip of `RefUpdate` is a hint only. Its transport is transfer's SPEC-SY-replication-stream HEAD frame (X-AG3).
  > • Replica TTL is `ref_sync.max_staleness_s` (default 300). A stale replica serves reads but blocks GC and blocks new publishes to protected refs.
  > • The history keep-set = the closure of protected refs, identical on every node. It replaces node-local tags and `--keep-manifest`.
  > • This replaces the RootCatalog gossip.

- 仕組み:
  - データモデル: ノードローカルの ref 複製 `ref_replica(scope, name, kind, target_cs, protected, seq, sig, fetched_at)` と、取込み済み `seq` の高水位。
  - 取得: 起動時と `ref_sync.poll_interval_s` (既定 60) ごとに `?after=seq&limit=1000` で取得し、次ページが無くなるまで続ける。endpoint とページングの定義は Curare 文書 §4.7 と同じ。HEAD フレームで新しい `seq` を見たら即時に取得する (gossip は合図だけ)。X-AG3 が受理されない場合は poll だけで追いつく (§7 Q-A15)。
  - keep-set: `protected = true` の ref から到達可能な changeset → tree → blob の閉包。prolly tree なので差分だけ再計算する。
  - 取込判定の配布 (M18): 同じページが `verdicts[{blob, scope, state, seq}]` を運ぶ (X-AG12、§4.20)。
  - 設定キー (C++ で新設): `ref_sync.max_staleness_s` (既定 300)、`ref_sync.poll_interval_s` (既定 60)。
- 不変条件: 署名検証に失敗した RefUpdate は適用しない。`seq` は後退しない。古い複製 (`max_staleness_s` 超過) は GC と保護 ref への publish を止める。
- 失敗時: `ref_sync.stale{scope, age_s}` を出し、状態表示に「ref 複製が古い」を出す。検証失敗は `ref_sync.bad_signature` で、その scope の ref 取得を止めて警報。
- 変更対象: 新設 `synergos-core/native/ref_replica/`、履歴 GC の C++ 移植 (`synergos-core/src/history/gc.rs`・`tags.rs` 相当。タグは移植しない)。
- 担保手段: ランタイム、ローカル検証ゲート (改ざんした RefUpdate の拒否、古い複製で GC が止まる試験 S5)。
- ドメイン: mesh-core に `ref-replica` を足す。

### 4.5 AIGAME-M05 changeset の commit (Synergos 側: commit-ipc)

- 区分: [契約変更] (SPEC-SY-commit-ipc、W)。対象発見: AGD-DM-02, -05, AGD-GW-22, -14, -19。フェーズ A3。
- 目的: 128 件ずつの独立送信 (`synergos-core/src/cli_worktree.rs:13,34-45`) とファイルごとの bump・archive・放送 (`synergos-core/src/ipc_server.rs:1352-1417`) を、1 回の ref CAS で確定する changeset に置き換える。
- 契約 SPEC-SY-commit-ipc (正本: 本書):

  > A command set on transfer's SPEC-SY-ipc-v2, authorized by security's SPEC-SY-ipc-capability scope `project:<ref>:publish`. `Commit{scope, base_cs, ref, entries (streamed frames: {path, blob|delete|link, mode, class}), message, intent, operation_id}` has no 1024-path cap. The daemon stages blobs into the local object store, uploads them through transfer's SPEC-SY-custody-transfer-v2 and attaches the receipts per item when the policy rule sets `min_copies`, builds and signs the tree and changeset, calls Curare (SPEC-CU-changeset-v2 + SPEC-CU-ref-authority), and on success emits a `RefUpdate` hint. Responses `CommitResult{cs, ref_seq} | PathConflict{…} | CommitRejected{code, details}` are streamed.

- 仕組み:
  - 流れ: (1) エントリを stream で受ける。(2) blob を局所オブジェクトストアに stage (1 回読み、NR-AG-4)。(3) stage した blob を SPEC-SY-custody-transfer-v2 で custody へ上げ、方針の rule が `min_copies` を持つ項目には受領証 (SPEC-SY-custody-receipt-v2) を付ける。(4) tree と changeset を作り `producer_sig` を付ける。(5) Curare の changeset v2 で batch admission → データ → commit (X-AG2 の `{parents[], base_cs, ref, expected_cs, intent, operation_id, author_principal, policy_blob}`)。(6) Curare が受理し副署 (`acceptor_sig`) して ref CAS。(7) 成功したら `RefUpdate` の合図を replication stream に流す。
  - 競合: Curare が 409 `path_conflict` を返したら `PathConflict{path, entry_id, base_entry, head_entry, incoming_entry}` を stream で返し、`Conflict` を作業ツリーに残す (両方の blob を保持)。解決は親 2 つの `merge` changeset。merge driver (例: `unityyaml`) は方針で選ばれ、明示的な解決のときだけ SPEC-SY-sandbox-profile の中で動く。バイナリは自動マージしない。
  - Curare の型付き拒否 (Curare 文書 §4.7〜§4.20 の応答) は `CommitRejected{code, details}` に写す。どの場合も stage した blob は消さない (再送で再利用)。

    | Curare の応答 | `code` | daemon の扱い |
    |---|---|---|
    | 409 `durability_pending` | `commit.durability_pending` | 受領証の不足を `details.missing[]` で返す。custody への上げを再試行し、同じ `operation_id` で再送する |
    | 423 `locked{holder, destination_ref}` | `commit.locked` | 局所の `lock_intent` を残し (§4.9)、保持者と宛先 ref を表示する |
    | 422 `companion_missing` | `commit.companion_missing` | 欠けた組 (例: `.unity` と `.meta`) を `details.pairs[]` で返す |
    | 422 `path_policy_violation` | `commit.path_policy_violation` | 違反パスと規則を返す。§4.7 の局所検査が先に止めるはずなので、到達したら局所規則の版ずれとして警報 |
    | 422 `class_policy_violation` | `commit.class_policy_violation` | 違反エントリと rule id を返す |
    | 422 `tree_mismatch` | `commit.tree_mismatch` | 局所の tree 符号化の不一致として警報し、vector の版を表示する |
    | 403 `agent_policy_violation` | `commit.agent_policy_violation` | 主体と理由を返す。再送しない |
    | 409 `screening_pending` | `commit.screening_pending` | 判定待ちのエントリを返す。判定後に同じ `operation_id` で再送できる |
    | 503 `ref_signing_unavailable` | `commit.authority_unavailable` | 指数待ちで再送する。ref は動いていない |

  - 冪等: (principal, `operation_id`) を冪等キーにする。再送は同じ `CommitResult` を返す。
  - 小ファイル (AGD-GW-19): 放送は changeset ごとに 1 回 (HEAD)。取得は転送の `WANT_MANY`。
  - CLI: `synergos publish <id> --all` は 1 つの `Commit` になる。途中成功は存在しない。
- 不変条件: commit 点は Curare の ref CAS の 1 回だけ。`base_cs` の無い commit は受け付けない (旧形式取込みの `import` を除く)。受理前の changeset は他ノードの作業ツリーに反映しない (shelf と draft は §4.10)。
- 失敗時: Curare 不達は `commit.authority_unreachable`、署名失敗は `commit.sign_failed`、上の表の型付き拒否 (`commit.durability_pending`、`commit.locked` ほか)。どれでも stage した blob は残す。
- 変更対象: `synergos-ipc` の C++ 移植 (`synergos-ipc/native/`、コマンドの追加)、`synergos-core/src/ipc_server.rs` と `cli_worktree.rs` 相当の C++、新設 `synergos-core/native/commit/`。
- 担保手段: ランタイム、ローカル検証ゲート (§6.2 S1「同じ `base_cs` からの 2 つの commit」と Curare 文書の試験 1、型付き拒否の写像 S16)。

### 4.6 AIGAME-M06 作業ツリーへの反映: 原子的、dirty に安全、同期モード、追いつき

- 区分: [契約変更] (SPEC-SY-worktree-apply、W) + [実装要件] (状態表示、wanted set の永続化)。対象発見: AGD-GW-02, AGD-GW-10, AGD-GW-22。フェーズ A3。
- 目的: dirty 判定の無い auto-pull (`synergos-core/src/daemon.rs:984-986`)、読まれない `SyncMode` (`synergos-core/src/project.rs:27-34,55`)、ファイル単位の反映、生の offer だけに頼る追いつきをやめる。
- 契約 SPEC-SY-worktree-apply (正本: 本書):

  > How a target tree reaches the worktree:
  > (1) Prefetch every needed blob into `.synergos/incoming/`.
  > (2) Dirty check against transfer's SPEC-SY-local-index (stat cache size+mtime+file id → blake3; hash only on miss).
  > (3) Apply in one rename pass. Companions declared in policy go last (`*.meta` follows its asset; OFPA externals follow their `.umap`).
  > (4) Never replace a dirty file. Incoming bytes go to `<name>.conflict-<short>.<ext>`, and a `ConflictDetected` event is emitted.
  > (5) Deletes move to `.synergos/trash`; dirty files are never deleted.
  > (6) Displaced bytes follow security's SPEC-SY-displaced-preimage.
  > `SyncMode` is enforced: Full = apply; Manual = an "available updates" ledger + `project pull`; Selective = the SPEC-SY-workspace-view filter. Status reports `pending_changeset` until the pass completes.

- 仕組み:
  - 反映の journal: `.synergos/apply.journal` に `{target_cs, phase: prefetch|apply|done, renamed[]}` を書き、再起動時は journal から「旧 tree に戻す」か「新 tree まで進める」を決める (途中状態を残さない)。
  - companion 規則: 方針の `companions[]` (SPEC-CU-project-policy) で決める。既定は `*.meta` → 本体の後、OFPA の外部 actor → `.umap` の後。
  - 同期モードの意味 (本書が正本): `Full` は反映、`Manual` はノードローカルの「利用可能な更新」台帳 `{scope, ref, cs, files, bytes}` に記録して `project pull` で反映、`Selective` は §4.10 の view で絞る。
  - TRANSFER-M30 との関係: 転送効率の TRANSFER-M30 (Synergos 側、`sync_mode` の尊重) は、この意味の下で「何を取得するか」を wanted-set で絞る実装である。本書は「何を作業ツリーに反映するか」を決め、TRANSFER-M30 は取得量を減らす。段階の順序は、TRANSFER-M30 が T4、本書の反映が A3 (transfer T2 と同じ flag day) で、反映が先に来る。そこで X-AG11 で TRANSFER-M30 を T2 / A3 へ前倒しする追補を出す。受理されない場合、A3〜T4 の間は Manual / Selective でも取得は全量のまま (帯域の非効率だけで、作業ツリーの正しさは本書の反映規則が守る) で、T4 で wanted-set による絞り込みが加わる。
  - 追いつき (AGD-GW-10): 接続時と 60 秒ごとの HEAD / HAVESET (転送の SPEC-SY-replication-stream) で head を知り、Curare 紐付けのプロジェクトでは HEAD に Curare 署名の RefUpdate を載せる (X-AG3)。X-AG3 が受理されない場合は SPEC-SY-ref-sync の poll (60 秒) だけで追いつく (G-A6 の「60 秒 + 転送時間」は poll 間隔と同じなので満たせる。§7 Q-A15)。プロジェクトごとの wanted set は永続化し、offer を取り逃がしても失われない。pin は壁時計 (`synergos-core/src/exchange/mod.rs:26` の 5 分) ではなく転送 ID に結ぶ (TXF2 の `req_id`)。
  - 状態表示: IPC の `Status` に `behind{changesets, files, bytes}` と `pending_changeset` を足す (SPEC-SY-ipc-v2 の `Status` を拡張)。
  - 競合の解決: `KeepLocal` は手元のバイトを親 2 つの `merge` changeset として commit、`AcceptRemote` は `.conflict-` コピーを rename で置く。現行の状態だけ変える解決 (`synergos-core/src/conflict/mod.rs:242`) は移植しない。
  - 取込判定の展開ゲート (スクリプト可能形式の隔離) は SECURITY-M24 が持ち、本書は変えない。
- 不変条件: dirty なファイルは置換も削除もしない。changeset は全部反映するか全く反映しないか。受理されていない changeset は反映しない (shelf を明示的に取る場合を除く)。Manual モードでは `project pull` 無しに作業ツリーを変えない。
- 失敗時: 容量不足は prefetch 段で止め、作業ツリーを変えない (`apply.insufficient_space`)。反映中の kill は journal から回復する。`ConflictDetected{path, ours, theirs, cs}` を出す。
- 変更対象: `synergos-core/src/daemon.rs`・`exchange/mod.rs`・`checkout.rs`・`conflict/mod.rs` 相当の C++、新設 `synergos-core/native/worktree/`。
- 担保手段: ランタイム、ローカル検証ゲート (§6 試験 2・3・7)。
- ドメイン: mesh-core に `worktree` を足す。

### 4.7 AIGAME-M07 パスは tree の属性: rename、削除、可搬性 (Synergos 側)

- 区分: [契約変更] (SPEC-SY-object-model の規則 + security の SPEC-SY-path-policy)。対象発見: AGD-DM-06, AGD-GW-15, AGD-OP-01, AGD-OP-02。フェーズ A3。
- 目的: パスを同一性に使うのをやめ (`FileId(pub String)`、`synergos-net/src/types.rs:74`)、削除を伝播し、OS 間で同じアセットが分岐しないようにする。
- 仕組み:
  - 同一性は `(scope, entry_id)`。rename は同じ blob と同じ `entry_id` を新しいパスに置くだけで、内容転送は 0 (G-A4)。受信側は旧パスのファイルを rename で移す。
  - 削除は「エントリが無い」で表し、§4.6 の trash 経由で反映する。checkout は余分なファイルを報告だけにしない (`synergos-core/src/checkout.rs:111` の挙動を変える。dirty なものは残す)。
  - 正規化は security の SPEC-SY-path-policy の 1 関数 (NFC、常に case-fold、末尾のドットと空白の除去、8.3 短縮名・ADS・デバイス名の拒否、どの深さでも deny リスト) に、記述子の `path_policy` (大小文字・`max_segment_utf16: 255`・`max_path_utf16: 240`・`forbid_windows_reserved`) を重ねる。1 つの tree の中で `casefold(NFC(path))` は一意 (サーバとクライアントの両方で検査)。
  - 既存の `.synergos` の大小文字区別の比較 (`synergos-core/src/manifest.rs:308-313`、`synergos-ipc/src/command.rs:266-272`) は C++ に移植しない。
- 不変条件: 一意性違反の tree は commit も受信もしない。予約パスはどの深さでも書かない。
- 失敗時: `path.collision{a, b}`、`path.reserved{path}`、`path.too_long{path, utf16_len}` で拒否または隔離。
- 変更対象: `synergos-core/native/relative_path.cpp` (既存、security M13 と共有)、§4.6 の worktree。
- 担保手段: ランタイム、ローカル検証ゲート (3 OS の runner で §6 試験 8)。

### 4.8 AIGAME-M08 プロジェクト方針とクラス (Synergos 側: 消費)

- 区分: [契約変更] (SPEC-CU-project-policy を消費し、tree エントリに `class` と `policy_rule_id` を持つ)。対象発見: AGD-DM-10, AGD-GW-18, AGD-OP-14 (クラス別保持)、AGD-GW-11 (派生物を履歴のソースにしない)。フェーズ A3。
- 目的: 既定で全プロジェクト・全受信物を保管する挙動 (`synergos-net/src/config.rs:453-454`、`synergos-core/src/exchange/mod.rs:833-841`) と、全パス共通の保持 (`synergos-net/src/config.rs:288-299`) をやめる。
- 仕組み: Synergos は方針を解釈しない。commit 時に解決済みの `policy_rule_id` と `class` を tree エントリから読み、(a) 履歴の保管先を選ぶ (`working`・`submission`・`dataset` は archive で `min_copies` の対象、`reproducible` と `derived` は TTL 付きの cache 層で、ソースとしては保管しない)、(b) 同期の可否 (`sync`)、(c) チャンク層に入れるか (rule の `chunk`、§3.10)、(d) companion (§4.6)、(e) ロックの型 (§4.9)、(f) `min_copies` (§4.13) を決める。
- 不変条件: 方針を後で編集しても、既存のエントリは記録済みの `policy_rule_id` のまま (ファイルを開き直さない)。クラスの妥当性は Curare が受理時に検査する。
- 失敗時: 未知の `policy_rule_id` は `policy.unknown_rule` で保守的な既定 (`working`、archive、同期あり) を使い、警報を出す。
- 変更対象: 履歴の C++ 移植 (`synergos-core/src/history/` 相当)、§4.6。
- 担保手段: ランタイム。

### 4.9 AIGAME-M09 ロックの cache: LockAnnounce と読み取り専用ビット

- 区分: [契約変更] (SPEC-SY-lock-announce、A)。対象発見: AGD-GW-01, AGD-DM-05。フェーズ A3 (Curare のロック権威は A2)。
- 目的: ロックが無い現状 (IPC にロック系コマンドが無い) に、Curare 権威のロックを作業ツリーで見えるようにする。
- 契約 SPEC-SY-lock-announce (正本: 本書):

  > A JCS `LockAnnounce{scope, entry_id, path, holder_principal, destination_ref, state: held|retained|released, seq}`, signed by Curare under ctx `lock-announce`. It is gossiped as a **cache** and is never authoritative. The daemon sets the read-only attribute on exclusive-policy files that are not held locally (SVN needs-lock UX). The local broker exposes lock state.

- 仕組み: daemon は LockAnnounce を記述子の `verify_keys.lock-announce` (§4.3) で検証してノードローカルのロック cache `lock_cache(scope, entry_id, destination_ref, holder_principal, state, seq)` に置く。方針が `lock = exclusive` で手元が保持していないファイルには OS の読み取り専用属性を立てる。取得・解放は local broker / CLI から Curare (SPEC-CU-lock-service、LFS locking 互換) に要求する。オフライン時は局所の `lock_intent` を記録し、再接続時に取得するか、publish の前に Conflict にする。commit の拒否は Curare が行う (cache で拒否を決めない)。
- 不変条件: cache は判定に使わない。`seq` が後退した LockAnnounce は捨てる。
- 失敗時: 署名不正は捨てて `lock.bad_signature`。Curare 不達のときは読み取り専用ビットを維持し、取得要求を `lock.authority_unreachable` で返す。
- 変更対象: 新設 `synergos-core/native/lock_cache/`、gossip の C++ 移植。
- 担保手段: ランタイム、ローカル検証ゲート (`main` でロック → feature ブランチで commit → `retained`、merge → 解放、の試験は Curare 文書の試験 5 で、Synergos は読み取り専用ビットの切替を §6.2 S8 で検査する)。

### 4.10 AIGAME-M10 view と部分ワークスペース、shelf (Synergos 側)

- 区分: [契約変更] (SPEC-SY-workspace-view、A)。対象発見: AGD-DM-13, AGD-GW-06, AGD-GW-05。フェーズ A3。
- 目的: 常に空の `Selective` (`synergos-core/src/project.rs:55`) と manifest 全体の checkout (`synergos-core/src/checkout.rs:80`) をやめる。WIP の受け渡しを恒久履歴から分ける (`shelf`)。
- 契約 SPEC-SY-workspace-view (正本: 本書):

  > The node-local workspace record `{scope, view_id, ref}` in `.synergos/workspace.toml`. It is resolved from SPEC-CU-view-spec and cached. Checkout, auto-pull and want generation skip excluded subtrees **without fetching their tree nodes**. IPC: `project configure +/path -/path`.

- 仕組み: view は Curare が持ち (SPEC-CU-view-spec、読取 ACL と交差済み)、daemon は解決結果を cache する。prolly tree なので除外した部分木は node を取得しない (G-A5)。`project configure +/path -/path` は Curare の view を更新し、結果を待って反映する。shelf は `shelf/<principal>/<name>` の ref (種別 `shelf`、`max_ref_age` 30 日) で、未受理の changeset を指名した peer と共有する。受け取る側は明示的に `project shelf apply` したときだけ反映する。
- 不変条件: view の外のパスは取得も反映もしない。ACL で見えないパスは view に入らない (Curare 側で交差)。shelf の changeset は `shelf apply` 以外の経路で作業ツリーに入らない。
- 失敗時: view の解決に失敗したら最後に成功した view を使い、`view.stale` を出す。初回の解決に失敗したら checkout しない。
- 変更対象: §4.6 の worktree、`synergos-core/src/project.rs` 相当の C++。
- 担保手段: ランタイム、ローカル検証ゲート (§6.2 の S2 (G-A5 計測) と S13 (shelf の明示適用))。

### 4.11 AIGAME-M13 link の実体化 (Synergos 側)

- 区分: [契約変更] (SPEC-SY-object-model の `link` エントリ)。対象発見: AGD-DM-12, AGD-AI-09。フェーズ A3。
- 目的: 共有エンジンコンテンツ・外注パック・共有コーパスを、プロジェクトごとに重複保管・転送せずに使う (AGD-DM-12)。
- 仕組み: tree の `link{scope, ref|changeset, subpath, mode: pinned|track}` を、参照先 scope の capability で取得して読み取り専用で実体化する。参照先の `view` 権限が無ければ実体化しない。バイトは参照先の重複排除領域で取得し、プロジェクト横断の存在オラクルを作らない (D-A16)。`pinned` の更新は明示的な changeset、`track` は参照先 ref の RefUpdate に追従する。
- 不変条件: link の中身は書けない (書込は参照先の scope への commit)。参照先の `view` 権限が無い主体の作業ツリーに、link の中身は 1 ファイルも現れない。
- 失敗時: 参照先の権限が無いと `link.forbidden` を返し、何も実体化しない。状態表示に未解決の link を出す。
- 変更対象: §4.6 の worktree (`synergos-core/native/worktree/`)、ref 複製 (`synergos-core/native/ref_replica/`、`track` の追従)。
- 担保手段: ランタイム、ローカル検証ゲート (§6.2 S12)。

### 4.12 AIGAME-M19 消去: 墓標の消費

- 区分: [契約変更] (SPEC-SY-tombstone、W)。対象発見: AGD-AI-06。フェーズ A3 (Curare の消去台帳は A2 で dry-run)。
- 目的: 墓標の無い manifest、年齢だけの GC、hold の無い即時 reclaim (`synergos-custody/src/store.rs:210-215`)、暗号化経路の無い退避 (`synergos-core/src/history/rotation/mod.rs`) を、監査可能な消去に変える。
- 契約 SPEC-SY-tombstone (正本: 本書):

  > Synergos consumes Curare's signed purge list (SPEC-CU-erasure-ledger, ctx `purge-list`) `{seq, items[{blob, scope, reason, not_before}]}` from `GET /api/v1/purge-list?after=seq`. After `not_before`, and when no `legal_hold` applies, the bytes are purged through security's SPEC-CU-destructive-op GC identity, and a hash-only tombstone is kept. A peer offer, want or serve of a tombstoned blob is refused with a typed code. Trees still reference the blob id. Offload copies are crypto-shredded per SPEC-SY-envelope-encryption. Chunks shared with live objects are retained (Q-A7).

- 仕組み: 履歴・custody・edge・daemon は purge list を `GET /api/v1/purge-list?after=seq` (Curare 文書 §4.21 と同じ endpoint) で取り込み、org の purge-list 鍵 (§4.3) で検証して、ノードローカルの墓標集合 `tombstones(blob, scope, reason, not_before, purged_at)` に置く。`not_before` を過ぎ legal hold が無ければ、別 ID の GC (security の SPEC-SY-custody-lifecycle と SPEC-SY-retention-lock の手順) がバイトを消し、ハッシュだけを残す。security の保持ロックとの順序: governance モードのロックは destructive-op の step-up と 2 人目の承認でだけ短縮でき、compliance モードのロックは短縮しない (その対象は crypto-shred、Q-A7)。
- 不変条件: 墓標の blob は offer・want・配信のどれも行わない。tree は blob ID を参照し続ける (再現性の記録は残る)。
- 失敗時: 墓標の blob を要求されたら型付きコード `content_erased` で拒否し、受け手は 451 相当として扱う。purge list の署名不正は取り込まず警報。
- 変更対象: 履歴 (`synergos-core/src/history/` 相当)、custody (`synergos-custody/native/`)、gossip / TXF2 の受付。
- 担保手段: ランタイム、ローカル検証ゲート (Curare 文書の試験 15 のうち Synergos 部分と §6.2 S6: 履歴と offload から消えハッシュが残る、peer の offer を拒否する)。
- ドメイン: opaque-custody と mesh-core (履歴)。

### 4.13 AIGAME-M20 複製の勘定、履歴の役割、バックアップの完全性

- 区分: [契約変更] (SPEC-SY-replica-accounting、W) + [運用] (役割の宣言、export の照合)。対象発見: AGD-AI-18, AGD-OP-07, AGD-OP-14。フェーズ A3。
- 目的: 相互同期しない履歴ノード (`docs/versioning-design.md:104-106`)、複製数を見ない GC (`synergos-core/src/history/gc.rs:89-96`)、常に 1 か所の `locations` (`synergos-custody/src/handlers.rs:124`)、資産バイトを落としうる export (`scripts/site/export-data.mjs:10`、`scripts/site/run-control.mjs:16`) を直す。
- 契約 SPEC-SY-replica-accounting (正本: 本書):

  > GC may drop an object reachable from a protected ref only if at least `min_copies` (policy, default 2) distinct valid copies exist. The copy evidence is security's `RetentionAck` (SPEC-SY-retention-ack) or `locations[]` in SPEC-SY-custody-receipt-v2. Scheduled scrub re-verifies BLAKE3/Bao and withdraws acks for failed copies. The history role is declared to control via transfer's SPEC-SY-node-roles, with `{archive_lag, free_bytes}`. History archiving is class-aware: `working`, `submission` and `dataset` go to archive and count toward `min_copies`; `reproducible` and `derived` go to the cache tier with TTL and are never archived as source. Only entries whose ingest verdict is `clean` are archived (AIGAME-M18).

- 仕組み:
  - 証拠: 新しいメッセージ (`HaveVersion`) は作らず、security の RetentionAck と custody 受領証 v2 の `locations[]` を使う。
  - 役割: archive 役は SPEC-SY-node-roles (転送) で control に宣言し、`{archive_lag, free_bytes}` を報告する (X-AG10)。heartbeat (`synergos-control/src/api/heartbeat.rs:17-25`) に役割が無い現状を置き換える。X-AG10 が受理されない場合は、役割と `{archive_lag, free_bytes}` を heartbeat の拡張だけで報告する (§4.22 M28、§7 Q-A16)。複製の証拠は RetentionAck と受領証なので、役割報告の経路が変わっても勘定は変わらない。
  - scrub: archive 役は既定 30 日周期で BLAKE3 / Bao を再検証し、失敗した複製の ack を取り下げる。
  - バックアップ: サイト export は custody の実際の root (`SYNERGOS_CUSTODY_ROOT` を含む) を束ね、Curare の受領台帳と照合する。確定済み blob の欠落は失敗として報告する (成功にしない)。報告の形式は Curare 文書の `blob_availability` (C5 と共有)。bundle 形式は SECURITY-M28 (SPEC-CU-site-bundle-v2)。
- 不変条件: 保護 ref から到達可能なオブジェクトは、有効な証拠が `min_copies` 未満のまま消さない (G-A15)。
- 失敗時: 証拠が足りないとき GC は対象を残し `replica.under_replicated{blob, copies}` を出す。export の欠落は非ゼロ終了。
- 変更対象: 履歴 GC の C++ 移植、`synergos-custody/native/`、`scripts/site/export-data.mjs` と `scripts/site/lib/dir-bundle.mjs` (運用スクリプトでオーナー決定 6 の対象外。変更は security P0 / M28 と合わせて出す)。
- 担保手段: ランタイム、ローカル検証ゲート (§6 試験 18)。

### 4.14 AIGAME-M21 受信側の照合と汚染の封じ込め

- 区分: [契約変更] (SPEC-SY-object-model の受信規則。署名は SPEC-SY-signed-envelope)。対象発見: AGD-AI-21。フェーズ A3。
- 目的: 無署名の wire ヘッダから版を作る (`synergos-core/src/exchange/mod.rs:747`)、先着を勝たせて本物を拒否する (`synergos-core/src/history/store.rs:154-160`)、publisher を送信元で埋める (`exchange/mod.rs:840`) 挙動をやめる。
- 仕組み: 受信側と履歴ノードは、BLAKE3 が「署名済み ref (Curare の RefUpdate) から到達可能な tree のエントリ」に一致するバイトだけを受ける。期待する `blake3` と `size` は署名済みの HEAD または tree から取り、送り手から取らない (転送の SPEC-SY-txf2 と同じ規則)。一致しないバイトは隔離 (`quarantine/<blake3>`) して `integrity.poison_candidate{peer, entry_id, expected, actual}` を出し、版にしない。同じエントリに 2 つの候補が来ても先着を採らず、ref に一致する方だけを採る。Curare の証明 (SPEC-CU-snapshot-attestation) は tree を再計算して照合できるので、Synergos の archive 役は証明済み tree の受信時に tree ID を再計算し、記述子の `verify_keys.attestation` (§4.3) で署名を検証する。
- 不変条件: 照合していないバイトを作業ツリー・履歴・edge の配信に使わない (G-A13)。
- 失敗時: 隔離と警報。隔離物は SECURITY-M24 の検査経路へ渡せる。
- 変更対象: 受信と履歴の C++ 移植 (`synergos-core/src/exchange/mod.rs`、`history/store.rs` 相当)。
- 担保手段: ランタイム、ローカル検証ゲート (§6 試験 17)。

### 4.15 AIGAME-M22 学習クラスタ向けの読み出し: edge 役 (Synergos 側)

- 区分: [契約変更] (転送の SPEC-SY-node-roles への追補 X-AG10、SPEC-SY-object-get の利用)。対象発見: AGD-AI-08, AGD-AI-07。フェーズ A4。
- 目的: 固定スナップショットを GPU クラスタが範囲読みで取れるようにする (AGD-AI-08)。Curare の S3 互換 gateway のバイトの供給元を Synergos の `edge` 役に置く。
- 仕組み: Curare の S3 互換 gateway (SPEC-CU-snapshot-gateway) は、バイトを custody か Synergos の `edge` 役から Bao 検証済みの範囲として取る。edge は SPEC-SY-object-get (`GET /v2/objects/{blake3}` と `.bao?range=`) で配り、集合単位の LCT (`(project_ref, tree, purpose)` を名指す) を検査する (M17、§4.19)。edge は範囲を名指す capability (SPEC-SY-custody-capability-v2 の範囲 caveat) だけを受け、組織ごとの `maxObjectBytes` (SPEC-CU-tenant-v1 の org 設定。上限値そのものは転送効率の TG-06 と SPEC-SY-custody-transfer-v2 の `Upload-Limit` が持つ) を超えるオブジェクトを配らない。G-A10 の 60 秒失効は SPEC-SY-revocation-bundle。チャンク層に入れるかは §3.10 の方針 rule。5 GB の単発 PUT 上限・1800 秒の custody 上限 (`synergos-custody/src/handlers.rs:236`)・resume は転送効率文書の SPEC-SY-custody-transfer-v2 とチャンク層が解く。
- 不変条件: edge は目的と tree を名指さない capability で学習用データを配らない。範囲を名指さない capability で範囲応答を返さない。`clean` でないエントリを配らない (§4.20)。
- 失敗時: capability 不足は 403、範囲の検証失敗は配信を止めて `integrity.violation`、`maxObjectBytes` 超過は 413。
- 変更対象: edge 役の C++ (`synergos-core/native/` の edge モード。転送効率文書の T4 と同時)、`synergos-custody/native/` (範囲 caveat の検査)。
- 担保手段: ランタイム、ローカル検証ゲート (Curare 文書の試験 19 の Synergos 部分: 全範囲が Bao 検証済み。§6.2 S11: 目的付き LCT の無い範囲要求が 100 % 403)。

### 4.16 AIGAME-M23 ツール連携: LFS custom-transfer agent とローカル broker

- 区分: [契約変更] (SPEC-SY-lfs-transfer-agent と SPEC-SY-local-broker、ともに A)。対象発見: AGD-GW-07, AGD-GW-16, AGD-AI-17。フェーズ A3 (agent と broker)、A5 (adapter)。
- 目的: 既存の Unity / Unreal の LFS プラグインと stock の `git lfs` を変更なしで使えるようにし (AGD-GW-07, AGD-GW-16)、DCC adapter が使う API を 1 つに絞る。
- 契約 SPEC-SY-local-broker (正本: 本書):

  > A per-user endpoint on SPEC-SY-ipc-v2 with SPEC-SY-ipc-capability tokens (`client_kind=plugin`). It is the only API the DCC adapters use: `status(paths)`, `lock/unlock`, `commit`, `view.configure`, `preview(path)`, `conflicts`, `resolve`. It extends Curare `[id: SPEC-CU-dcc-commands]`.

- 契約 SPEC-SY-lfs-transfer-agent (正本: 本書):

  > The `lfs.customtransfer.synergos` standalone agent, per the Git LFS custom-transfer protocol. LFS sha256 oids are resolved to `b3:` ids through SPEC-CU-object-registry. Bytes move over transfer's SPEC-SY-txf2 or the custody path.

- 仕組み: broker は daemon の IPC の上に置き、adapter (Unreal の `ISourceControlProvider`、Unity の VCS package、Blender・Maya・Photoshop UXP のパネル) が使う唯一の API にする。現行の ars plugin (`ars-plugin-synergos/src/bridge.rs:1-4`) はイベントを受けるだけなので、broker の client として `client_kind=plugin` の最小 scope を持たせる。LFS agent は Git LFS の custom-transfer 規約 (stdin/stdout の JSON 行) に従い、LFS の sha256 oid を Curare の object registry で `b3:` に変えて、TXF2 か custody の経路で運ぶ。LFS の batch と locking の API 本体は Curare (SPEC-CU-lfs-compat)。UE Virtual Assets の backend (FIoHash = BLAKE3-160) は A5。
- 不変条件: adapter は broker 以外の経路で daemon を呼ばない。LFS の token は短寿命で、Cernere の token を貼らせない。agent を通したバイトは oid の sha256 と一致する。
- 失敗時: broker の scope 外の要求は `ipc.forbidden`。LFS agent は oid を解決できなければ LFS の `error` 応答で失敗させる (黙って別経路にしない)。
- 変更対象: `synergos-ipc/native/` (broker の endpoint)、新設 `synergos-core/native/lfs_agent/`、`ars-plugin-synergos` (A5 で C++ 側に移す。cpp-migration 手順 5)。adapter の置き場所は Q-A4。
- 担保手段: ローカル検証ゲート (§6.2 S9: stock の `git lfs push` / `git lfs pull` が `lfs.customtransfer.synergos` 経由でバイト一致 100 %。lock 系は Curare 文書の G-A17)、ランタイム (IPC capability)。
- ドメイン: ars-plugin (broker の client)、network-transport (IPC)。

### 4.17 AIGAME-M25 ワークステーション daemon のネイティブ要件、format skew

- 区分: [実装要件] (マシンモード、設定、単一インスタンス、配布) + [契約変更] (SPEC-SY-format-skew は W。SPEC-SY-local-broker は §4.16)。対象発見: AGD-OP-03, AGD-OP-04, AGD-OP-05, AGD-OP-13, AGD-OP-08。フェーズ A3 (C++)、A5 (配布リング)。
- 目的: ビルド・cook・GPU ノードを常駐 seed / archive にし、80 台規模の Windows に配備でき、混在する版の fleet で旧クライアントを取り残さない。
- 契約 SPEC-SY-format-skew (正本: 本書):

  > On-disk and object formats: readers support N and N−1. A writer writes format N only when `min_fleet_version ≥ N`, computed from signed heartbeats. `synergos-core migrate --dry-run` reports what would change. Wire version negotiation stays in transfer's SPEC-SY-peer-wire and SPEC-SY-ipc-v2 (`Hello{protocol, min_protocol, features}`).

- ネイティブ実装要件:

  | ID | 要件 | 現状の根拠 |
  |---|---|---|
  | NR-AG-5 | マシンモード: Windows Service (状態は `%ProgramData%\Synergos`、明示した DACL)、macOS LaunchDaemon、systemd unit。ユーザーモードは非 roaming の LocalAppData | `docs/two-node-operations.md:255-258`、`docs/platforms.md:111-115` |
  | NR-AG-6 | identity 鍵は DPAPI / Keychain / libsecret (security の SPEC-SY-node-enrollment)。見つからなければ起動失敗し、cwd や `/tmp` に落ちない | `synergos-net/src/identity.rs:125-137,196-217` |
  | NR-AG-7 | 設定の解決器を 1 つにする。優先順位は CLI > `SYNERGOS_CONFIG` > マシン > ユーザー。明示したパスが無いと起動失敗。IPC に `ConfigDescribe` / `ConfigApply` を置き、適用できない項目は「再起動が必要」を型で返す | `synergos-core/src/daemon.rs:581-595`、`synergos-core/src/ipc_server.rs:2042-2047` |
  | NR-AG-8 | 単一インスタンス (Windows は名前付き mutex、Unix は flock)。ソケットを無条件に消さない | `synergos-core/src/ipc_server.rs:85-88` |
  | NR-AG-9 | `projects.json` は耐久書込 (security の単一の耐久書込関数) で前世代を残す。読込失敗は safe mode で起動し、上書きしない。root の絶対パスは再リンク (`synergos relink`) で直せる | `synergos-core/src/project.rs:313-331`、`synergos-core/src/daemon.rs:163` |
  | NR-AG-10 | `synergos doctor` (設定・鍵・ソケット・ref 複製の鮮度・容量を検査) | — |
  | NR-AG-11 | 署名付きの MSI / pkg / deb と canary / studio / all のリング。Tauri の bundle は daemon を同梱して起動する。updater の鍵は **Excubitor Vault** に置く (署名と検証は SECURITY-M31) | `synergos-tauri/src-tauri/tauri.conf.json:4,26-33` |

- 不変条件: 2 つ目のインスタンスは起動しない。混在する fleet は `min_fleet_version` に達するまで旧形式で書く (新形式のオブジェクトを 1 つも書かない)。
- 失敗時: 設定不在・鍵不在・二重起動はすべて非ゼロ終了と理由のログ。
- 変更対象: `synergos-core/src/main.rs`・`daemon.rs`・`project.rs` 相当の C++、`synergos-net/src/identity.rs` 相当、`synergos-ipc/native/`、配布物。
- 担保手段: ローカル検証ゲート (§6 試験 20、§6.2 S10 の混在 fleet)、ランタイム。

### 4.18 AIGAME-M26 の Synergos 半分: egui / desktop の外殻 (quick-xml 経路の除去)

- 区分: [運用] (workspace 構成。作業そのものは SECURITY-M30) + [実装要件] (置き換える外殻のアクセシビリティ)。対象発見: AGD-OP-15。フェーズ A0 (SECURITY-M30 の workspace 分割)、A5 (C++ desktop-gui の受入後に egui を撤去。運用画面の 1 本化は Curare 文書 §4.27)。
- 目的: オーナー決定 5 の quick-xml 0.30 の経路を、除外指定ではなく依存ごと消す。デスクトップの外殻を最終的に C++ desktop-gui の 1 つにする。
- 仕組み: SECURITY-M30 のとおり、`Cargo.toml:2-12` の workspace を `server/` と `desktop/` に分け、`synergos-gui` を `desktop/` へ移して `eframe = { version = "0.31", default-features = false, features = ["default_fonts", "glow", "wayland", "x11"] }` にする (accesskit を外す)。egui バイナリは C++ desktop-gui (`cpp-migration.md` 手順 5) が受入ゲートを通るまで build でき、`cpp-migration.md:48` の受入面と parity の基準であり続ける。受入後に撤去する。security Q3 で利用者がいないと分かれば、それより前に撤去してよい。egui にしかない競合・設定・転送要求・取消の画面は C++ desktop-gui の要件に書く (Q-A14 の既定 A)。Dioxus の管理画面 (`synergos-admin-ui`、workspace 外: `Cargo.toml:15-19`) は拡張せず、運用画面は Curare API 上の 1 つに置き換える (A5)。その backend として `synergos-control` の管理 API を security の SPEC-SY-control-admin-auth で使う。
- ai-game の追加要件: C++ desktop-gui はスクリーンリーダー対応を明示要件に持つ (accesskit が担っていた分)。外殻は SPEC-SY-local-broker だけを使い、daemon の内部 API を直接呼ばない。
- 不変条件: サーバ成果物の依存グラフに GUI の crate が入らない。`audit.toml` の除外を置かない。
- 失敗時: SECURITY-M30 に従う。feature を外しても経路が残る場合は egui バイナリを配布物から外し、parity の基準としてだけ build して `desktop/` の audit に赤のまま残す。workspace の分割で Rust の Revisor ケースが壊れたら、分割を 1 commit で戻す。
- 変更対象: `Cargo.toml`、`server/Cargo.toml` と `desktop/Cargo.toml` (新設)、`synergos-gui/Cargo.toml` (desktop-gui ドメイン)。変更は security P0 の M30 として 1 回で出し、本書は同じ変更に乗る。
- 担保手段: ローカル検証ゲート (security の AT-SY-SEC-33 = §6 試験 21)。

### 4.19 AIGAME-M17 の Synergos 半分: 配信制限

- 区分: [契約変更] (C++ の配信経路の規則。LCT の形式は security の SPEC-SY-lct-v1)。対象発見: AGD-AI-16。フェーズ A3 (履歴)、A4 (edge)。
- 目的: 履歴ノードが接続中の任意の peer に任意の版を返す挙動 (`synergos-core/src/exchange/mod.rs:959-990`、`docs/versioning-design.md:101`) をやめ、ライセンス・同意で範囲が限られたデータセットを Curare の方針の外へ出さない。
- 仕組み: 履歴と edge は、Curare が gated とした tree (SPEC-CU-rights-purpose。用途の判定は Curare) のエントリを、`(project_ref, tree, purpose)` を名指す LCT (security D-18 の集合単位 LCT) を示した要求にだけ配る。gated かどうかは署名済み RefUpdate の ref 種別 (`dataset_snapshot`) と記述子の方針から決まり、Synergos は用途を解釈しない。会員資格そのものは SECURITY-M03 (SPEC-SY-membership-token)。現行の「接続中の任意の peer に任意の版」は移植しない。
- 不変条件: gated な tree のバイトは、その tree と用途を名指す有効な LCT 無しには 1 バイトも出ない。権利の変更による失効は SPEC-SY-revocation-bundle で 60 秒以内 (G-A10)。
- 失敗時: LCT 不足は 403 (`serve.capability_required{tree, purpose}`)、失効済みは 403 (`serve.revoked`)。
- 変更対象: 履歴の配信経路 (`synergos-core/src/exchange/mod.rs` 相当の C++)、edge 役 (§4.15)。
- 担保手段: ランタイム (LCT の検査)、ローカル検証ゲート (§6.2 S11)。

### 4.20 AIGAME-M18 の Synergos 半分: 取込判定ゲート (履歴の複製と配信)

- 区分: [契約変更] (C++ の受信・履歴・edge の規則。判定の状態は security の SPEC-CU-ingest-verdict)。対象発見: AGD-AI-21 (汚染の封じ込め)、AGD-DM-10 (受信物を全部保管)。Curare 側の発見 AGD-AI-11 の Synergos 半分。フェーズ A3 (履歴)、A4 (edge)。
- 目的: Curare 文書 §4.20 が Synergos に課す義務「`clean` の無い blob を archive しない」を、Synergos 側の規則として持つ。検査されていない内容が履歴に複製され、edge から配られることを止める。
- 仕組み:
  - 状態の名前は security の SPEC-CU-ingest-verdict (`scanning → clean | quarantined → review`) をそのまま使う。判定は Curare が作り、Synergos は作らない。
  - データの出所: tree は内容アドレスで不変なので、変わる判定を tree に入れない。判定は SPEC-SY-ref-sync のページが運ぶ `verdicts[{blob, scope, state, seq}]` (ctx `ref-log` の同じ署名の下) で配る (X-AG12)。ノードは `ingest_verdicts(blob, scope, state, seq)` に置く。
  - 履歴 (archive 役): `clean` のエントリだけを archive に入れる。`scanning` のエントリは `<state_dir>/pending-screen/` に保持だけして archive に入れず、`min_copies` の勘定にも数えない。`quarantined` のエントリはソースとして archive せず、受信した複製は隔離 (`quarantine/<blake3>`) に移して TTL で消す。
  - edge と履歴の配信: `clean` 以外のエントリの offer・serve は型付きコード `screening_withheld{blob, state}` で断る。
  - 作業ツリーへの展開は SECURITY-M24 の展開ゲートが扱い、本機構は変えない。
  - X-AG12 が受理されない場合: archive と edge は、Curare が `clean` の tree でしか確定しない `release` と `dataset_snapshot` ref (Curare 文書 §4.20) から到達可能なエントリだけを archive・配信し、その他のエントリは `pending-screen` に保持するだけにする。
- 不変条件: `clean` でないエントリは archive されず、履歴・edge から配られない。`quarantined` のエントリがソースとして残る件数は 0。
- 失敗時: 判定が未着 (`verdicts` に行が無い) のエントリは `scanning` として扱う (fail-closed)。判定の署名不正は取り込まず、その scope の archive を止めて警報。
- 変更対象: 履歴の C++ 移植 (`synergos-core/native/` の履歴と archive モード)、edge 役、`synergos-core/native/ref_replica/` (`verdicts` の取込み)、`synergos-custody/native/` (custody が履歴の受け手になる場合の同じ規則)。
- 担保手段: ランタイム、ローカル検証ゲート (§6.2 S15)。

### 4.21 AIGAME-M27 の Synergos 半分: テナントと配置の識別

- 区分: [契約変更] (SPEC-CU-tenant-v1 を control と custody が消費) + [運用] (catalog の既定値、配備プロファイル)。対象発見: AGD-OP-06, AGD-OP-09, AGD-OP-16。フェーズ A1 (env の撤去、Curare 側)、A5 (Synergos C++)。
- 目的: `SYNERGOS_ORGANIZATION_BINDINGS` と control の slug + メール台帳 (`synergos-control/src/store/mod.rs:9-17`) による、全組織が 1 つの保管ノードと鍵空間を共有する構成をやめる。catalog の固定 node ID と localhost の公開 URL (Synergos `excubitor.catalog.yaml:29-30`、Curare `excubitor.catalog.yaml:33`) を配置のデータモデルに置き換える。
- 仕組み: control と custody は Curare の SPEC-CU-tenant-v1 を取得して、組織ごとの `storage_policy`・`quota_bytes`・`dedup_scope`・`min_copies_default`・`verify_keys.purge-list` (X-AG13) を使う。配置は転送の SPEC-CU-storage-placement に従い、ノード ID はセットアップ時に作る。配備プロファイルは hosted / on-prem appliance / hybrid の 3 つで、on-prem の秘密は Excubitor Vault か OS の鍵保管から取る (Cloudflare への依存は on-prem で必須にしない)。
- 不変条件: 1 つの保管ノードの鍵空間に 2 つの組織の実体が混ざらない (保管名は SPEC-SY-envelope-encryption の org 鍵で導出)。catalog に固定のノード ID を置かない。
- 失敗時: tenant record を取得できない間は、最後に検証した record で動き `tenant.stale` を出す。初回取得に失敗したら組織の受付を開かない。
- 変更対象: `synergos-control` と `synergos-custody` の C++ 移植 (`synergos-control/native/`、`synergos-custody/native/`)、`excubitor.catalog.yaml` (固定 ID の削除。security P0 と同じ変更)。
- 担保手段: ランタイム、ローカル検証ゲート (2 組織を同じ custody に置き、片方の LCT で他方の実体に届かない試験。security の封じ込め試験と共有)。

### 4.22 AIGAME-M28 の Synergos 半分: 運用性

- 区分: [実装要件] (トレース、heartbeat、health) + [運用] (catalog の自己回復)。対象発見: AGD-OP-11, AGD-OP-12, AGD-OP-08。フェーズ A3。
- 目的: 詰まった履歴ノード・満杯のディスク・遅い拠点回線を crunch の前に見つけ、走っている版を外から分かるようにする。
- 仕組み: OpenTelemetry のトレースに changeset ID と W3C trace context を載せ、Curare → custody → daemon まで通す。相関フィールドは security の SPEC-CU-security-event。heartbeat に役割・`archive_lag`・`free_bytes`・ref 複製の鮮度・`min_fleet_version` の算出に使う版を足す。health と readiness を分け、health は版を返す (SECURITY-M32)。catalog の `autostart` / `restart_policy` は Excubitor の catalog 側で直す (`excubitor.catalog.yaml:17-18`。値の判断は Curare Q-CU-5 と合わせる)。GUI の帯域表示は計測値だけを出す。AGD-OP-08 の版付き移行経路は SPEC-SY-format-skew と SPEC-SY-legacy-import が持つ。
- 不変条件: health が版を返さない build はリリースしない。計測していない値を GUI に出さない。
- 失敗時: トレースの送出先が落ちても処理は止めない (送出は捨てる)。heartbeat が `free_bytes` の下限を下回ったら archive 役の受付を止めて警報。
- 変更対象: `synergos-core/src/main.rs`・`control_heartbeat.rs` 相当の C++、`synergos-control/native/` (health と heartbeat の受付)、`excubitor.catalog.yaml`。
- 担保手段: ローカル検証ゲート (health の版の試験: health の応答に build の版が含まれ heartbeat の版と一致すること)、ランタイム。
- AGD-OP-10 は Synergos 側に Cernere ID を持ち込まない。会員資格は control が Cernere 所属と SPEC-CU-membership-export から作る (security D-19) ので、本書の追加作業は無い。AGD-AI-19 は Curare の M29 (ペルソナとフロー) が持つ。Synergos は §6 のシナリオを Augur に登録する。

### 4.23 参照する他正本の契約

Curare が正本の契約のうち、Synergos が消費するものは定義を 1 回だけ原文で引用する (全文は Curare リポジトリの `spec/plan/redesign-ai-game-dev.md`)。

- SPEC-CU-project-descriptor:

  > A Curare-signed descriptor (ctx `project-descriptor`) `{scope_uuid, org_uuid, project_ref, display_name, path_policy{unicode: NFC, case, max_segment_utf16: 255, max_path_utf16: 240, forbid_windows_reserved}, policy_blob, ref_log_verify_keys[{kid, pub}], dedup_scope: project|org}`. Synergos opens a Curare-bound project only from a descriptor. The gossip topic and DHT key are derived from `project_ref`. **For Curare-bound projects, `project_ref = blake3("curare" ‖ org_uuid ‖ scope_uuid)`.** This fills the binding that security's SPEC-CU-membership-export delegates to this doc. Security's `blake3(org ‖ host_pubkey ‖ local_id)` stays for P2P-only legacy projects. Free-form names are display-only.

- SPEC-CU-ref-authority (Synergos は `ref_log` を複製し、CAS を commit 点として呼ぶ):

  > Tables `refs(scope_id, name, kind: branch|tag|release|alias|dataset_snapshot|shelf, target_cs, protected, retention{min_snapshots, max_snapshot_age, max_ref_age}, stream?{parent_ref, flow: merge_down|copy_up|none, lock_destination_ref}, created_by_principal)`, unique per (scope, name), and the append-only `ref_log(seq, scope, name, old_cs, new_cs, principal, reason, request_id, at, sig)`. `PUT /api/v1/scopes/{id}/refs/{name} {expected_cs, target_cs, reason}` is a compare-and-swap and **the single commit point**. 409 `ref_moved{current}`. Tags, releases and dataset snapshots are immutable. Aliases move with audit rows. Pins become path-scoped aliases that need the `review`/`release` permission (security SPEC-CU-authority-model). `release` refs are created only by finalize on a reviewed head. Expiry follows the Iceberg four-step model. `GET /api/v1/resolve?uri=curare://org/scope@alias`. The signing key is a dedicated Vault Transit key (X-AG7).

- SPEC-CU-path-conflict:

  > A conflict exists when a path changed between `base` and the ref head **and** in the incoming changeset (entry_id-aware, so renames are tracked). The response is 409 `path_conflict{path, entry_id, base_entry, head_entry, incoming_entry}`. Clients keep `Conflict{scope, path, base_blob, ours_blob, theirs_blob, ours_cs, theirs_cs}` with both blobs retained. Resolution is a two-parent `merge` changeset. A policy merge driver (for example `unityyaml`) runs only during explicit resolution, under SPEC-SY-sandbox-profile, and never auto-merges binaries.

- SPEC-CU-project-policy:

  > A versioned policy blob referenced by every changeset. Ordered rules `{match[], class: working|submission|reproducible|derived|dataset, lock: exclusive|soft|none, retain{versions, days}, sync, compress, chunk, min_copies, merge, companions[]}`. Rules are resolved at add/commit time and the resolved `policy_rule_id` is recorded on the tree entry, so a later policy edit does not reopen files. Curare validates the class at acceptance: `reproducible`/`derived` need a recipe key (SPEC-CU-derived-cache), and `dataset` needs a dataset scope.

- SPEC-CU-lock-service (Synergos は LockAnnounce の発行元として参照):

  > `asset_locks(id, scope, entry_id, path, destination_ref, holder_principal, device_id, mode: exclusive|soft, state: held|retained|released, acquired_at, expires_at?, released_by?, force_reason?)`, unique on (scope, entry_id, destination_ref) while held or retained. Unity Smart Lock semantics: a check-in on another branch moves the lock to `retained`, and a merge into the destination releases it. Force-unlock needs `manage` + SPEC-CR-step-up-assertion, writes an audit row and warns the next holder. A commit touching an `exclusive` entry locked by someone else for that destination is rejected. Offline editing creates a local `lock_intent`, which on reconnect either acquires the lock or produces a Conflict before any publish. The wire is Git-LFS-locking compatible via SPEC-CU-lfs-compat.

- SPEC-CU-view-spec:

  > `views(id, scope, owner_principal, name, spec_blob)` with spec `{include[], exclude[], classes[], follow_links, depth?}`. `POST /api/v1/acquisition/plan {ref|cs, view}` returns a paged plan of `{path, blob, size}` that keeps relative paths. The plan is intersected with the read ACL. It feeds transfer's SPEC-CU-fetch-session.

- SPEC-CU-erasure-ledger (Synergos は purge list を消費):

  > `erasure_requests{id, target: blob|entry_id|scope, reason, requested_by, step_up_ref, grace_until, legal_hold, state}`. `POST /api/v1/erasures` requires `manage` + SPEC-CR-step-up-assertion and runs SPEC-CU-destructive-op (dry-run, approval, tombstone, GC). It adds `tombstoned_at, tombstone_reason, retain_until, legal_hold, retention_class` columns. The migration 005 trigger is amended additively to let only these columns change. Reads of erased content return 451 `content_erased` with hash and metadata kept. Derived artifacts and fingerprints of the target are invalidated. The signed purge list `{seq, items[]}` is published under ctx `purge-list`.

- SPEC-CU-snapshot-attestation (Synergos の archive 役は tree を再計算して照合):

  > Finalizing a release or dataset snapshot emits a DSSE/in-toto statement `{ref, cs, tree, digests_alg, policy_blob, rights_summary_digest, screening_summary_digest}`. It is signed by a dedicated Vault Transit key (ctx `attestation` for the internal envelope; a DSSE envelope for external verifiers) and appended to a transparency log (the form is open, Q-A6). Verification is offline: recompute the tree and compare. Runs record the attestation digest as `datasetVersion`. A drift assertion between consecutive snapshots is stored as a quality facet.

- SPEC-CU-snapshot-gateway (Synergos の edge 役がバイトの供給元):

  > A read-only S3-compatible endpoint, `s3://curare-{org}/{snapshot_ref|tree}/{original/path}`, serving ListObjectsV2, GetObject with Range, and HeadObject. SigV4 maps to a session exchanged by `POST /api/v1/gateway/sessions{dataset, ref, purpose}`, which issues a set-scoped LCT per security D-18. Each session is ledgered (SPEC-CU-capability-ledger), charged to the egress budget, and checked against rights, screening and erasure state. Bytes come from custody or a Synergos `edge` node (transfer SPEC-SY-node-roles) as Bao-verified ranges.

- SPEC-CU-tenant-v1 (Synergos の control と custody が消費):

  > `tenants{org_id PK, plan, status, entitlements{order_management, magic_links, webhooks, max_sites, …}, storage_policy, quota_bytes, egress_quota, dedup_scope, min_copies_default}`, managed through an admin API authorized by Cernere org-admin plus a platform-operator role and hot-reloaded. It replaces `SYNERGOS_ORGANIZATION_BINDINGS`. A usage ledger covers stored bytes, egress, seats and API calls. Deployment profiles: hosted multi-tenant, on-prem appliance (pluggable OIDC IdP, secret provider = Excubitor Vault or OS keystore, no Cloudflare dependency) and hybrid. Quota enforcement uses security's SPEC-CU-quota-reservation; placement uses transfer's SPEC-CU-storage-placement.

セキュリティ・転送効率が正本の契約は再定義しない。本書での使い方だけを示す。

| 契約 ID | 正本 | 本書での使い方 |
|---|---|---|
| SPEC-SY-signed-envelope | `spec/plan/redesign-security.md` | changeset・RefUpdate・LockAnnounce・purge list・記述子・証明の署名。ctx の追加は X-AG7 |
| SPEC-SY-lct-v1, SPEC-SY-custody-capability-v2 | 同上 | edge の範囲配信 (範囲 caveat)、LFS の action、gated tree の配信 |
| SPEC-SY-hlo2-session-binding, SPEC-SY-membership-token | 同上 | ref-sync・LockAnnounce・墓標を運ぶ peer セッション。`project_ref` による会員資格 |
| SPEC-SY-ipc-capability | 同上 | commit-ipc (`project:<ref>:publish`) と local broker (`client_kind=plugin`) |
| SPEC-SY-path-policy | 同上 | §4.7 の正規化。記述子の `path_policy` を重ねる |
| SPEC-SY-pull-only-receive, SPEC-SY-displaced-preimage | 同上 | §4.6 の受信安全 |
| SPEC-SY-retention-ack, SPEC-SY-retention-lock, SPEC-SY-immutable-replica | 同上 | §4.13 の複製の証拠、§4.12 の消去の順序 |
| SPEC-SY-envelope-encryption | 同上 | 退避先の crypto-shred、保管名の導出、重複排除領域 |
| SPEC-SY-sandbox-profile | 同上 | merge driver の実行 (X-AG9) |
| SPEC-SY-custody-receipt-v2, SPEC-SY-custody-lifecycle | 同上 | `locations[]` を複製の証拠にする。commit 時の受領証。墓標の物理削除の手順 |
| SPEC-SY-revocation-bundle | 同上 | 権利変更による失効 (G-A10) を edge が受ける |
| SPEC-SY-chunk-access | 同上 | チャンク層のアクセス不変条件 (転送効率の X5 で無条件化) |
| SPEC-CU-ingest-verdict | 同上 (Curare に置く security の契約) | §4.20 の状態名。展開ゲートは SECURITY-M24 |
| SPEC-SY-content-identity | `spec/plan/redesign-transfer-efficiency.md` | Blob の digest (BLAKE3-256 + SHA-256 を 1 走査) |
| SPEC-SY-manifest-v2 | 同上 | `manifest.json` format 2。X-AG1 の受理後はノードローカルの実体化索引 (Q-A12) |
| SPEC-SY-local-index | 同上 | §4.6 の dirty 判定、NR-AG-1 の履歴索引の置き場 |
| SPEC-SY-ipc-v2 | 同上 | commit-ipc・broker の framing と stream 応答 |
| SPEC-SY-peer-wire, SPEC-SY-txf2, SPEC-SY-replication-stream, SPEC-SY-node-roles, SPEC-SY-object-get | 同上 | wire、範囲取得、HEAD / HAVESET (X-AG3)、`edge` / `archive` 役 (X-AG10) |
| SPEC-SY-custody-transfer-v2 | 同上 | commit 時の custody への上げ、edge の `maxObjectBytes` |
| SPEC-SY-chunk-store, SPEC-SY-range-plan, SPEC-SY-format-policy | 同上 | 方針の rule が `chunk=true` で 8 MiB 以上のパスのチャンク層 |
| TRANSFER-M30 (`sync_mode` の尊重) | 同上 | §4.6 の同期モードの下の wanted-set (X-AG11) |
| SPEC-CU-changeset-v2, SPEC-CU-fetch-session, SPEC-CU-object-registry, SPEC-CU-storage-placement, SPEC-CU-collection-version | Curare リポジトリの `spec/plan/redesign-transfer-efficiency.md` | commit の受付 (X-AG2)、取得計画、`sha256 ↔ b3` の対応と重複排除領域、配置、大きい集合の paging (X-AG4) |

他テーマ正本への追補 (相手文書が受理する必要がある。受理までは「変更提案 (他テーマ正本への追補)」。未受理の項目と受理されない場合の代替は §7):

| # | 対象契約 (正本) | 追補 | 受理されない場合 |
|---|---|---|---|
| X-AG1 | SPEC-SY-manifest-v2 (転送) | manifest v2 はノードローカルの実体化索引で、コミットせず権威でなく SPEC-SY-object-model から再構築できる。エントリに `entry_id, class, policy_rule_id` を足す。git にコミットするのは別ファイルの SPEC-SY-workspace-pointer (`.synergos/head.json`)。「readers accept v1 and v2」は `manifest.json` に適用する | manifest v2 は転送の定義どおり git にコミットされる。ポインタは別ファイルなので番号は衝突しない。SPEC-SY-workspace-pointer は採らず、git の索引の衝突 (AGD-GW-14) は残る (Q-A12) |
| X-AG3 | SPEC-SY-replication-stream (転送) | Curare 紐付けプロジェクトの HEAD フレームは Curare 署名の `RefUpdate` を運ぶ。メンバー署名の head は P2P だけの旧プロジェクトと未受理の shelf / draft にだけ有効 | 追いつき (G-A6) は SPEC-SY-ref-sync の poll だけで行う (Q-A15) |
| X-AG5 | SPEC-CU-membership-export (セキュリティ) | Curare 紐付けプロジェクトの `project_ref` 導出は SPEC-CU-project-descriptor が固定する | — (security Q8 が本書に委ねている) |
| X-AG7 | SPEC-SY-signed-envelope の ctx 登録 (セキュリティ) | `changeset, ref-log, lock-announce, purge-list, project-descriptor, attestation` を追加。ref-log と attestation の鍵は受領証の鍵と共有しない専用の Vault Transit 鍵 | — |
| X-AG9 | SPEC-SY-sandbox-profile (セキュリティ) | 利用者に merge driver、プレビュー / turntable / diff の描画、export / embedding のジョブを足す | — |
| X-AG10 | SPEC-SY-node-roles (転送) | `edge` 役は gateway の範囲を配ってよい。`archive` 役は replica-accounting のために `{archive_lag, free_bytes}` を報告する | archive 役の報告は heartbeat の拡張だけで行う (§4.22)。edge の gateway 供給は A4 まで custody だけ (Q-A16) |
| X-AG11 | TRANSFER-M30 (転送) | TRANSFER-M30 を T4 から T2 (= A3 の flag day) へ前倒しし、本書 SPEC-SY-worktree-apply の同期モードの意味の下に置く | A3〜T4 の間は取得が全量のまま。作業ツリーの正しさは変わらない (§4.6、Q-A17) |
| X-AG12 | SPEC-CU-ref-authority の ref-log endpoint (Curare) と SPEC-CU-ingest-verdict (セキュリティ) | ref-log のページが `verdicts[{blob, scope, state, seq}]` を運ぶ。署名は ctx `ref-log` | archive・edge は `release` / `dataset_snapshot` から到達可能なエントリだけを扱う (§4.20、Q-A18) |
| X-AG13 | SPEC-CU-project-descriptor と SPEC-CU-tenant-v1 (Curare) | 記述子に `verify_keys{<ctx>: [{kid, pub, not_after?}]}` (lock-announce・attestation・purge-list)。purge-list の鍵の権威は tenant record の `verify_keys.purge-list` | Curare Q-CU-2 と同じ問い。代替は無い (検証鍵の出所が無いと M09・M19・M21 が検証できない) |

X-AG2 (SPEC-CU-changeset-v2)・X-AG4 (SPEC-CU-collection-version)・X-AG6 (SPEC-CU-capability-ledger) は Curare 側の契約、X-AG8 (SPEC-CR-service-principal) は Cernere への依頼 (security Q7、オーナー決定 4) への追補で、いずれも Curare 文書が扱う。

### 4.24 Cernere への依存 (SPEC-CR-*、オーナー決定 4。Synergos / Curare には実装しない)

| 契約 ID | 本書での使い方 | 暫定策 (Cernere が出すまで) |
|---|---|---|
| SPEC-CR-capability-signing-keys | edge の範囲配信・LFS action の capability の発行鍵の根 | Vault に pin した発行鍵 (security と同じ) |
| SPEC-CR-ssf-caep-transmitter | 主体の失効、エージェント token の停止経路 | 60 秒の会員資格 poll (既知の穴として記録) |
| SPEC-CR-step-up-assertion | ロックの強制解除、消去、権利の上書き | SPEC-CU-destructive-op による 2 名承認 |
| SPEC-CR-service-principal (追加提案、security Q7) | エージェント / サービス主体の changeset の `author_principal`。`kind=agent` と `operator_user_id`、タスクごとの発行を足す (X-AG8) | mTLS のサービス ID + Curare が発行するタスクごとのエージェント鍵 (≤ 1 時間) |
| SPEC-CR-guest-identity (追加提案、任意) | 外注先の guest 主体 | Curare のマジックリンク主体 |

### 4.25 新設データ (RULE_DATA_SCHEMA)

種類は RULE_DATA_SCHEMA §1 (`RULE_DATA_SCHEMA.md:20-23`) の master / user。どちらとも言い切れないものは寄せた側と理由を書く。保護の判断は Curare 文書 §4.34 と揃える (未発表の資産名とパス、誰が何を編集中かは秘匿対象)。

| データ名 (形式) | 種類 | 権威ソース | 保存先 | 保護要否 | 保護方法 |
|---|---|---|---|---|---|
| ローカルオブジェクトストア (blob / tree / changeset、内容アドレス) | master (Curare の版の写し。利用者の操作で増えるが、内容は Curare が受理した版に従う) | なし (自己証明。権威は Curare の ref) | `<state_dir>/objects/` (作業ツリーと同じボリューム) | 要 (完全性 + 秘匿: tree と changeset のバイトは未発表の資産名とパスを含む) | 取込み時に BLAKE3 照合。`<state_dir>` に OS のファイル ACL (所有者とサービスアカウントだけ)。保管時の暗号化は SPEC-SY-envelope-encryption |
| 作業ツリー索引 (`files` 表 + `entry_id, class, policy_rule_id`) | user (利用者の作業ツリーの状態) | ノードローカル (使い捨て、再構築可能) | `.synergos/index.db` | 要 (秘匿: パスを含む) | コミットしない。`.synergos` に OS のファイル ACL。デバイス間で信用しない |
| staging / incoming (一時 blob + 反映 journal) | user | daemon | `.synergos/incoming/`、`.synergos/apply.journal` | 要 (完全性 + 秘匿) | 耐久書込、journal で回復、`.synergos` の ACL |
| ref 複製 (`ref_replica(scope, name, kind, target_cs, protected, seq, sig, fetched_at)`) | master | Curare の ref log | `<state_dir>/refs.db` | 要 (完全性) | 署名検証 (ctx `ref-log`)、`seq` の後退拒否、鮮度 TTL |
| 取込判定 (`ingest_verdicts(blob, scope, state, seq)`) | master | Curare (SPEC-CU-ingest-verdict) | `<state_dir>/refs.db` | 要 (完全性) | 署名検証 (ctx `ref-log`、X-AG12)、未着は `scanning` 扱い |
| ワークスペース記録 (TOML `{scope, view_id, ref}` + view の cache) | user | Curare の view | `.synergos/workspace.toml` | 要 (秘匿: view のパスを含む) | コミットしない。`.synergos` の ACL |
| ワークスペースポインタ (1 行 JSON `{pointer_format:1, scope, ref, cs, tree}`) | master (Curare の ref log の写し) | Curare の ref log | `.synergos/head.json` (git へのコミットは任意) | 不要 (ID だけでパスを含まない) | merge driver で衝突を解く |
| ロック cache (`lock_cache(scope, entry_id, destination_ref, holder_principal, state, seq)`) | user (誰が何を編集中かは行動記録) | Curare のロックサービス | `<state_dir>/locks.db` | 要 (行動記録) | 主体は Cernere の ID だけで氏名・メールを持たない。署名検証 (ctx `lock-announce`)。`<state_dir>` の ACL |
| 墓標集合 (`tombstones(blob, scope, reason, not_before, purged_at)`) | master | Curare の消去台帳 (purge list) | 履歴・custody・edge・daemon の `<state_dir>` | 要 (完全性 + 秘匿: 消去対象の存在そのもの) | 署名検証 (ctx `purge-list`)、追記のみ、項目は blob ID と scope だけ |
| 複製の証拠 (RetentionAck と受領証の `locations[]` の集約) | master | 各 archive / custody ノード | 履歴ノードの sidecar (security の保持メタデータ) | 要 (完全性) | 署名、scrub による取り下げ |
| 利用可能な更新の台帳 (`{scope, ref, cs, files, bytes, seen_at}`) | user | daemon | `<state_dir>` | 要 (秘匿: changeset の存在) | `<state_dir>` の ACL。Manual モードの表示用 |
| 隔離物 (blob `quarantine/<blake3>` と `pending-screen/` + 理由) | user (受信の副産物) | daemon / 履歴ノード | `<state_dir>/quarantine/`、`<state_dir>/pending-screen/` | 要 (完全性 + 秘匿) | 作業ツリー・配信に使わない。TTL 付き。SPEC-SY-envelope-encryption |

秘密の新設は無い。ref-log・attestation・purge-list・lock-announce・project-descriptor の署名鍵は Curare 側 (Excubitor Vault の Transit)、updater の鍵は Excubitor Vault に置く。

### 4.26 ドメイン帰属 (`spec/domains/*.domain.json`)

| ドメイン | 帰属する機構 | 備考 |
|---|---|---|
| mesh-core (`mesh-core.domain.json`) | M01 (`object-model`)、M02 (ポインタ、旧形式取込み)、M04 (`ref-replica`)、M05 (commit)、M06 (`worktree`)、M07、M08、M09 (lock cache)、M10、M13、M17 (履歴の配信制限)、M18 (履歴の門)、M21、M23 (LFS agent)、M25 | 説明文「メッシュの中核。ノードの状態と経路、リアルタイム同期の土台」に「版のオブジェクトモデルと作業ツリー反映」を足す。3 つのモジュール `object-model`・`ref-replica`・`worktree` を `synergos-core/native/` の下に置く (新ドメインは作らない) |
| opaque-custody (`opaque-custody.domain.json`) | M19 (墓標、custody 側)、M20 (複製の勘定、受領証の `locations[]`)、M27 (tenant の消費) | 説明文の「版・提出・保持許可はCuへ委ねる」は維持 |
| network-transport (`network-transport.domain.json`) | M05 / M23 の IPC コマンド、M22 と M17 の edge 配信 (転送効率の正本) | 本書の新契約は IPC コマンドの追加だけ |
| control-plane (`control-plane.domain.json`) | M20 (役割の宣言)、M27 (テナントの消費、catalog の固定 ID の廃止)、M28 (heartbeat、health) | |
| desktop-gui (`desktop-gui.domain.json`) | M26 の Synergos 半分 (`desktop/` workspace の egui、C++ desktop-gui の要件) | 分割の作業は SECURITY-M30 |
| admin-ui (`admin-ui.domain.json`) | M26 (拡張しない。運用画面は Curare 側) | |
| ars-plugin (`ars-plugin.domain.json`) | M23 (local broker の client) | |

DCC / エンジン adapter を別 repo にする場合 (Q-A4) は、その repo に `dcc-adapters` ドメインを新設する案を出す (REVIEW_DESIGN §0)。

### 4.27 `cpp-migration.md` "Compatibility contracts" の 3 文書共通の改訂表

オーナー決定 6 により、本書の機構は次の改訂で C++ 移行の互換契約として扱う。security 文書 §4.37 と転送効率文書 §4.31 は同じ行 (`:39-40`・`:41-43`・`:44-47`・`:48`) に別々の改訂案を書いており、そのままでは食い違う。そこで、この 4 行と SPEC-SY-native-metadata については **次の表を 3 文書共通の改訂案** とし、security §4.37 と転送効率 §4.31 の該当行はこの表への参照に置き換える (§5.3)。改訂は本書とは別の Revisor 変更 1 つで、Q-A1 / Q-A12・security Q1 / Q3・transfer Q1 / Q2 の回答後に出す。左列は現行の文言 (`spec/plan/cpp-migration.md:39-48`、main @ d404f20)。各行のうち 3 文書で立場が分かれた節は、他文書の原案を引用して [変更提案] として示す。

| 現行の行 | 共通の改訂案 | 区分と調整点 |
|---|---|---|
| `:39-40` "Peer protocol: preserve protobuf schemas, QUIC ALPN, peer authentication, CRC/hash validation, transfer cancellation, and catalog exchange." | "Peer protocol: ALPN `synergos/2` per SPEC-SY-peer-wire (MessagePack frames, versioned stream magics, capability bits; the current wire has no protobuf schemas); session per SPEC-SY-hlo2-session-binding (mTLS, SESS; HLO1 is rejected with a typed close code) with SPEC-SY-membership-token enforced per stream; all signatures follow SPEC-SY-signed-envelope; decoders follow SPEC-SY-decode-bounds. Transfer per SPEC-SY-txf2 and replication per SPEC-SY-replication-stream; content identity per SPEC-SY-content-identity, and CRC32 is a pre-check only. Preserve transfer cancellation. Catalog exchange is replaced by SPEC-SY-ref-sync for Curare-bound projects and preserved for P2P-only legacy projects. TXFR v1, DHT1, BSW1 and FileWant are frozen for golden-vector interop only." | [契約変更] (置換)。security と transfer の 2 案を合わせた文。ai-game の追加は「CRC32 is a pre-check only」と catalog exchange の扱い。security 原案の "preserve transfer cancellation and catalog exchange semantics" に対し、catalog exchange は Curare 紐付けで置換とする [変更提案 (Q-A1、security Q1、transfer Q1)] |
| `:41-43` "Versioning: preserve `.synergos/manifest.json`, local high-water marks, restore/checkout semantics, tags and protected history. Reject path escapes, symlinks/junctions and version overflow before modifying data." | "Versioning: objects follow SPEC-SY-object-model and refs come from SPEC-SY-ref-sync. `.synergos/manifest.json` readers accept format 1 and format 2 (SPEC-SY-manifest-v2); C++ never writes format 1 and writes format 2 only under SPEC-SY-format-skew. The optional git-committed file is the separate `.synergos/head.json` (SPEC-SY-workspace-pointer). Local high-water marks (`state.json`) and node-local tags are read only through SPEC-SY-legacy-import. Tags are immutable Curare refs; tag deletion and history purge follow SPEC-CU-destructive-op tombstones (SPEC-SY-tombstone) and SPEC-SY-retention-lock; protected history is the SPEC-SY-ref-sync keep-set under SPEC-SY-replica-accounting, and history GC never trusts `index.json` alone. Worktree changes follow SPEC-SY-worktree-apply; checkout and restore are resumable verified jobs (SPEC-SY-ipc-v2). Apply SPEC-SY-path-policy (one normalizer, case-fold, deny list at any depth, handle-relative operations) and SPEC-SY-pull-only-receive; every received replace follows SPEC-SY-displaced-preimage. Reject path escapes, symlinks/junctions and non-canonical paths before modifying data." | [契約変更] (置換)。明示した選択: (a) 高水位とタグは legacy-import だけで読む。security 原案 "preserve `.synergos/manifest.json` readers (format 1 and the transfer doc's format 2) and local high-water marks" の後半を変える [変更提案 (Q-A1、security Q1)]。(b) `manifest.json` の format 2 は転送の SPEC-SY-manifest-v2 で、ポインタは別ファイル。X-AG1 の採否は manifest v2 をコミットするかだけを変える [Q-A12、transfer Q1]。(c) タグは存在し続ける (Curare の不変 ref)。削除は security 原案どおり tombstone |
| `:44-47` "Custody: … Preserve canonical receipt bytes, HMAC, capability method/object/expiry binding, exclusive publication, file/directory durability and tombstones. Cu remains the authorization owner." | security §4.37 の改訂文 (SPEC-SY-custody-receipt-v2 ほか) に転送 §4.31 の追加文 (SPEC-SY-custody-transfer-v2、batch admission、replication) を並べ、末尾に ai-game の文を足す: "Copy evidence for SPEC-SY-replica-accounting is `locations[]` in SPEC-SY-custody-receipt-v2; custody consumes SPEC-SY-tombstone and archives only `clean` entries (AIGAME-M18)." | [契約変更] (置換)。行の正本は security 文書 (§4.37) と転送効率文書 (§4.31)。ai-game の追加は replica-accounting・tombstone・取込判定の 3 点 |
| `:48` "Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces." | "Existing CLI, admin UI and Ars adapter remain acceptance surfaces. The native GUI surface is the C++ desktop-gui; until it passes its acceptance gate, the egui `synergos-gui` (built from the `desktop/` workspace without the accesskit feature) remains the parity baseline, and `synergos-tauri` is an interim surface. The C++ desktop-gui uses only SPEC-SY-local-broker over SPEC-SY-ipc-v2 and supports screen readers." | [契約変更] (置換)。security §4.37 の文 (SECURITY-M30) に従い、ai-game は最後の 1 文 (broker だけを使う、スクリーンリーダー対応) を足す [変更提案 (Q-A1、security Q1 / Q3)]。「Tauri が暫定面」は推定 (security Q3) |
| (新規行) | "Collaboration: SPEC-SY-commit-ipc, SPEC-SY-workspace-view, SPEC-SY-lock-announce, SPEC-SY-tombstone, SPEC-SY-local-broker, SPEC-SY-lfs-transfer-agent." | [契約変更] (追加) |
| (新規行) | "Formats: SPEC-SY-format-skew (readers N and N−1; writers N only when `min_fleet_version ≥ N`)." | [契約変更] (追加) |

SPEC-SY-native-metadata (`native-contracts.md:15-24`) の共通の改訂文 (転送の "Unchanged means equal BLAKE3" と本書の legacy-import 限定を合わせる): "Format 1 entries (read only through SPEC-SY-legacy-import): unchanged size/CRC retains the existing version. Format 2 entries (SPEC-SY-manifest-v2): unchanged means equal BLAKE3 per SPEC-SY-content-identity. Version numbers are display-only and never sent on the wire (SPEC-SY-object-model)." [契約変更] (置換) [変更提案 (Q-A1、transfer Q2)]。

「Implementation and verification order」(`cpp-migration.md:50-62`) の手順 3 "Port OS transports, message codecs, project/history services and durable I/O." を 2 つに分ける。

- 3a: OS transports、message codecs、durable I/O (契約に関係しないので続ける)。
- 3b: project / history services。**object-model の vector (G-A19) が C++ で通るまで着手しない**。3b は format 1 の parity ではなく新契約の振る舞い試験 (§6) を合格条件にし、format 1 は取込みの parity だけを求める。

「Completion gate」(`:64-71`) の "All existing feature families above must have passing parity evidence" に、「format 1 の取込み parity と、§6 の試験 2・3・4・7・8・10・17・18・20 と §6.2 の S1・S5・S6・S9・S10・S11・S12・S13・S14・S15・S16 の合格」を加える (security の AT-SY-SEC と転送の TG の追加と並べる)。最初のネイティブ増分の「manifest/state JSON」(`:82-83`) は旧形式取込み専用の読取に用途を限る旨を注記する。

### 4.28 `native-contracts.md` へ追加・変更する `[id:]` 節

| `[id:]` | 扱い | 本書の節 | 実装の置き場所 |
|---|---|---|---|
| SPEC-SY-native-metadata (既存、`native-contracts.md:15-24`) | §4.27 の共通の改訂文に置き換える | §4.2, §4.27 | `synergos-core/native/manifest_codec.cpp` (format 1 は読取専用) |
| SPEC-SY-object-model | 追加 | §4.1 | `synergos-core/native/object_model/` |
| SPEC-SY-ref-sync | 追加 | §4.4 | `synergos-core/native/ref_replica/` |
| SPEC-SY-workspace-pointer / SPEC-SY-legacy-import | 追加 (workspace-pointer は X-AG1 の受理後) | §4.2 | `synergos-core/native/` |
| SPEC-SY-commit-ipc | 追加 | §4.5 | `synergos-ipc/native/`、`synergos-core/native/commit/` |
| SPEC-SY-worktree-apply / SPEC-SY-workspace-view | 追加 | §4.6, §4.10 | `synergos-core/native/worktree/` |
| SPEC-SY-lock-announce | 追加 | §4.9 | `synergos-core/native/lock_cache/` |
| SPEC-SY-replica-accounting / SPEC-SY-tombstone | 追加 | §4.12, §4.13 | `synergos-core/native/` (履歴)、`synergos-custody/native/` |
| SPEC-SY-local-broker / SPEC-SY-lfs-transfer-agent | 追加 | §4.16 | `synergos-ipc/native/`、`synergos-core/native/lfs_agent/` |
| SPEC-SY-format-skew | 追加 | §4.17 | `synergos-core/native/` |

`docs/versioning-design.md` には冒頭に「2026-10-23 の再設計で §0 の索引の権威・§3.5 のタグ・§4 のロックを変更提案中 (`spec/plan/redesign-ai-game-dev.md`)」の注記を足す。本文の書換えは Q-A1 の回答後。

### 4.29 crate 別の変更マップ

「現行 Rust」の列は、オーナー決定 6 に従い変更しないことを示す (SECURITY-M30 の workspace 分割を除く)。実装はすべて C++ 側。

| crate / 面 | ドメイン | C++ 側で入る機構 | 現行 Rust への変更 |
|---|---|---|---|
| synergos-net (QUIC、gossip、DHT、bitswap、transfer、identity、catalog、chain、content) | network-transport | M03 (topic の導出)、M06 の HEAD 受信 (転送の replication stream)、M09 の gossip、NR-AG-6 (鍵保管)。`RootCatalog`・`ChainBlock`・`ChunkDag` は移植しない (M01) | なし (`server/` workspace へ移るだけ、SECURITY-M30) |
| synergos-core (daemon、exchange、manifest、history、hooks、checkout、conflict、ipc_server、project) | mesh-core | M01、M02 (ポインタ、取込み、NR-AG-1〜4)、M04、M05、M06、M07、M08、M09 (cache)、M10、M13、M17 (履歴の配信制限)、M18 (履歴の門)、M19 (履歴)、M20 (履歴)、M21、M22 (edge モード)、M23 (LFS agent)、M25 (NR-AG-5〜10)、M28 (トレース、health) | なし (`server/` へ移るだけ) |
| synergos-control | control-plane | M20 (役割と `{archive_lag, free_bytes}`)、M27 (SPEC-CU-tenant-v1 の消費、slug 台帳の置換)、M28 (heartbeat、health に版) | なし (`server/` へ移るだけ) |
| synergos-custody | opaque-custody | M01 (`b3:` 鍵、保管名は envelope-encryption)、M18 (`clean` だけを archive)、M19 (墓標)、M20 (`locations[]` を証拠に)、M22 (範囲 caveat)、M27 (tenant の消費) | なし (`server/` へ移るだけ) |
| synergos-relay | network-transport | なし (転送効率とセキュリティの正本) | なし (`server/` へ移るだけ) |
| synergos-ipc | network-transport | M05 (`Commit` 系、`CommitRejected`)、M06 (`Status` の拡張)、M10 (`project configure`、`shelf apply`)、M23 (broker の endpoint)、M25 (`ConfigDescribe` / `ConfigApply`) | なし (`server/` へ移り、`desktop/` からは path 依存) |
| synergos-gui (egui) | desktop-gui | なし (C++ desktop-gui に置き換わる。要件はスクリーンリーダー対応と broker 経由) | `desktop/` workspace へ移し、eframe の `accesskit` を外す (SECURITY-M30)。C++ desktop-gui の受入後に撤去 |
| synergos-tauri | desktop-gui | daemon の同梱と起動 (NR-AG-11)。暫定面かどうかは推定 (security Q3) | `desktop/` workspace へ移る (SECURITY-M30) |
| synergos-admin-ui | admin-ui | 拡張しない (運用画面は Curare 側、A5) | なし |
| ars-plugin-synergos | ars-plugin | M23 (broker の client、`client_kind=plugin`) | なし (`server/` へ移るだけ) |
| `scripts/site/*`、`excubitor.catalog.yaml` | control-plane | M20 (export の照合)、M27 (固定 node ID の廃止)、M28 (`autostart` / `restart_policy`) | security P0 / M28 と同じ変更で出す |
| `spec/vectors/object-model/` (新設) | (vector) | M01 | Rust 旧形式リーダーの vector 試験を Revisor ケースに登録 |

### 4.30 契約変更と実装の非効率の区別 (`spec/plan/redesign-analysis/HANDOFF.md` §6)

| 機構 | 契約変更 (互換契約を更新) | 実装の非効率 (C++ が最初から避ける) |
|---|---|---|
| M01 | SPEC-SY-object-model (CRC32 の同一性・7 つの表現を置換) | — |
| M02 | SPEC-SY-workspace-pointer、SPEC-SY-legacy-import、SPEC-SY-native-metadata の共通改訂 | NR-AG-1 (全書換え)、NR-AG-2 (起動時全走査)、NR-AG-3 (1 MiB の結果上限)、NR-AG-4 (多重読み) |
| M04 | SPEC-SY-ref-sync (局所タグ・高水位を置換) | — |
| M05 | SPEC-SY-commit-ipc (1024 パス上限と 128 件 batch を置換、型付き拒否) | gossip を changeset ごとに 1 回 |
| M06 | SPEC-SY-worktree-apply | wanted set の永続化、pin の転送 ID への結び付け |
| M07 | object-model の rename / 削除規則、path-policy (security) | — |
| M09 / M10 / M13 / M17 / M18 / M19 / M20 / M21 | 各契約 (lock-announce、workspace-view、link、配信制限、取込判定の門、tombstone、replica-accounting、受信照合) | — |
| M25 | SPEC-SY-format-skew、SPEC-SY-local-broker | NR-AG-5〜11 (マシンモード、鍵保管、設定解決、単一インスタンス、原子的保存、doctor、配布) |
| M26 | `cpp-migration.md:48` の受入面 (共通改訂表) | — (運用。作業は SECURITY-M30) |
| M27 | SPEC-CU-tenant-v1 の消費 | — |
| M28 | — | トレース、heartbeat の項目、health の版 |

## 5. 実装ステップ・影響範囲・ロールバック方針

### 5.1 フェーズ表

| フェーズ | 内容 | 対象 crate / module | 依存 | 検証方法 (テスト種別) | ロールバック |
|---|---|---|---|---|---|
| **A0** (今すぐ。文書・vector・ゲート) | 本書・Curare 文書・`findings/ai-game-verified.json` を Revisor で取り込む。§5.3 の文書間整合を先に済ませる。`cpp-migration.md` / `native-contracts.md` / `docs/versioning-design.md` の改訂 (§4.27, §4.28) は Q-A1 の回答後に別の Revisor 変更で出す。object-model の vector (M01) と vector のフック。SECURITY-M30 の workspace 分割 (M26 の Synergos 半分。security P0 の変更) | `spec/`、`spec/vectors/object-model/`、Cargo workspace (security の変更) | オーナー決定 1, 2, 5, 6 | vector の自己試験 (TS と Rust リーダー。C++ は A3 で参加)、AT-SY-SEC-33 (§6 試験 21)、§5.3 の ID 照合 | 文書の revert。workspace 分割は 1 commit で戻す (SECURITY-M30) |
| A1 / A2 (Curare の土台と面) | Curare 文書が持つ。Synergos 側の作業なし。Curare の ref log・CAS・記述子・方針・ロック・view・消去台帳 (dry-run)・取込判定の状態がここで入る | (Curare) | A0、security P2、transfer T1 | Curare 文書の検証 | Curare 文書のロールバック |
| **A3** (Synergos C++ のオブジェクトモデル) | object model の codec、ref-sync (`verdicts` を含む)、commit-ipc、worktree-apply (同期モードの意味)、ポインタと view、shelf、lock-announce、replica-accounting、取込判定の門 (履歴)、墓標の消費、受信時の照合、配信制限 (履歴)、旧形式取込みツール、format skew、マシンモード・設定・単一インスタンス、local broker、LFS transfer agent | `synergos-core/native/` (mesh-core)、`synergos-ipc/native/`、`synergos-custody/native/`、`synergos-control/native/` | A0 の vector。`cpp-migration.md` 手順 3b は **vector が通るまで保留**。security P3 / P4 (HLO2、会員資格、IPC capability)。transfer T2 (内容 ID、ipc-v2、TXF2、replication stream。X-AG11 が受理されれば TRANSFER-M30 も)。Curare A1 / A2 | C++ vector (G-A19)、Excubitor 経由の複数ノード試験 (試験 2・3・4・7・17・18、S1〜S17)、3 OS のパスコーパス (G-A18)、format 1 取込みの parity、インストール試験 (G-A21) | C++ の受入ゲートを通るまで、組織ごとに Rust の format 1 daemon を配備可能なまま残す (`cpp-migration.md:71`)。peer の flag day は security P4・transfer T2 と同じ 1 回 |
| **A4** (ML 向けの読み出し面、Synergos 側) | edge 役の範囲配信 (gateway の供給元、配信制限と取込判定の門、範囲 caveat、`maxObjectBytes`)、方針 rule が `chunk=true` のパスのチャンク層の利用、archive 役の証明照合 | Sy の edge / archive 役 (C++) | A3、Curare A2 (gateway、証明)、transfer T4 / T5 (配置、チャンク保管、TRANSFER-M30)、security P5 (envelope 暗号) | Curare 文書の試験 19 の Synergos 部分、試験 17 の汚染試験、S11・S15 の edge 部分、G-A16 の転送ベンチゲート | `edge` 役を設定で無効化、`project.chunking = off` (転送効率) |
| **A5** (エコシステムと退役) | Unity / Unreal の adapter、次に DCC パネル (broker 上)、UE Virtual Assets backend、運用画面の 1 本化 (Curare 側)、配布リング、C++ desktop-gui の受入後の egui 撤去、format 1 書込の退役 | 両 repo + adapter の repo (Q-A4)、`desktop/` workspace | A3、A4、SPEC-CR-guest-identity (任意)、C++ desktop-gui の受入ゲート | プラグインの smoke 試験、`min_fleet_version` が新形式に達したことを確認してから書込を退役、desktop-gui の受入試験 | adapter は独立。format 1 の読取は無期限に残す。egui は撤去前の commit に戻せば build できる |

### 5.2 互換性と移行

- `manifest.json` format 1 は無期限に読める。A5 までは Rust daemon だけが format 1 を書く。C++ は format 1 を書かない。
- 混在する fleet は `min_fleet_version ≥ N` になるまで format N−1 で書く (SPEC-SY-format-skew)。
- peer の flag day は組織ごとに 1 回 (security P4、transfer T2 と共有)。同じ組織の中で Rust ノードと C++ ノードを混在させない。
- 既存プロジェクトの移行手順: (1) Curare で scope と記述子を作る。(2) 運用者が `synergos migrate --to object-model --map <legacy_id>=<project_uuid> --dry-run` で差分を確認。(3) 対応表を承認して実行し、プロジェクトごとの `import` changeset と版の連鎖ごとの changeset を作る。(4) 旧タグは「提案された alias」として Curare の承認を待つ。(5) git に `manifest.json` を入れていたチームは、X-AG1 が受理された場合、`.synergos/head.json` のコミットと `.gitattributes` の追加を行い、`manifest.json` を追跡から外す (`git rm --cached`、ファイルは消さない)。受理されない場合は `manifest.json` (format 2) のコミットを続け、`head.json` は任意。
- 局所状態 (`state.json`、`projects.json`、履歴索引) は取込み後も読取専用で残し、削除しない。Curare 側の migration は追加のみ (DROP 禁止) で、Curare 文書が持つ。
- 重複排除領域の契約 ID は SPEC-CU-object-registry + SPEC-SY-envelope-encryption である (旧 ID `SPEC-SY-dedup-domain` は転送効率文書で廃止)。転送効率の登録表が ID をさらに変えた場合は、転送効率の登録表を正とする。

### 5.3 提出前の文書間整合 (両 repo 共通の事前検査)

本書と同じ Revisor 変更で出す前に、次の整合を取る。Revisor のマージ前ケースとして、両 repo の 6 文書から `SPEC-(SY|CU|CR)-[a-z0-9-]+`・`AIGAME-M[0-9]+`・`X-AG[0-9]+`・`Q-A[0-9]+` を抽出して比べる照合スクリプトを置き、ai-game の 2 文書の間で ID の集合が一致しないとき (片方にしか無い契約 ID、同じ契約 ID の引用文の不一致) は赤にする。

1. Curare 文書 §4.2 の参照表 (`SPEC-SY-chunk-store, SPEC-SY-range-plan, SPEC-SY-dedup-domain` の行) を `SPEC-SY-chunk-store, SPEC-SY-range-plan` と `SPEC-CU-object-registry + SPEC-SY-envelope-encryption` (security D-10/D-11) に置き換える。
2. Curare 文書 §4.0 に本書 §4.0 の機構の所有表を写す (Synergos 半分の一覧に M08・M17・M18・M27・M28 を足し、M26 を「M26 の Synergos 半分 (egui / desktop)」と書く)。
3. Curare 文書 §4.1 の SPEC-SY-replica-accounting・SPEC-SY-tombstone の引用と §4.24 の大きいオブジェクトの記述を、本書の現行の文 (`dataset` を archive に含める、`clean` だけ archive、purge-list の endpoint、`model` / `checkpoint` / `dataset_shard` の代わりに rule の `chunk=true`) に揃える。
4. Curare 文書 §7 に Q-A12〜Q-A18 を足すか本書への参照を置き、本書 §7 から Curare の Q-CU-2・Q-CU-3 を参照する。
5. Curare 文書の [CHALLENGE] と high/medium/low を、本書と同じ [変更提案 (neco 再判断)] と 高/中/低 に揃える。
6. security 文書 §4.37 と転送効率文書 §4.31 の `:39-40`・`:41-43`・`:44-47`・`:48` の行と SPEC-SY-native-metadata の扱いを、§4.27 の共通改訂表への参照に置き換える。転送効率文書に X-AG1・X-AG3・X-AG10・X-AG11 の受理状況 (受理 / 保留) を書く。
7. Curare 文書 §8 と本書 §8 で、同じ外部資料には同じ URL と同じ † を使う (Perforce streams は p4 stream の cmdref)。

## 6. 検証計画

受入条件は Revisor のマージ前ケースか Augur のシナリオとして登録する。複数ノード試験は Excubitor 経由で起動し、利用できない前提 (別 OS の runner、外部クラウド) は未確認として報告する (`cpp-migration.md:69-70`)。

### 6.1 ワークフロー受入シナリオ (両文書で共有する 1〜21)

受入試験 1〜21 は Curare 文書と共有する通し番号で、この表に全体を 1 回だけ載せる。「担当」が Sy の行は合格条件を本書が持ち、Cu の行は Curare 文書 §6 が持つ。両方の行は Synergos 部分の合格条件だけを書く。

| # | シナリオ | Synergos 部分の手順と合格条件 | 担当 | ゴール |
|---|---|---|---|---|
| 1 | 失われる更新 | 同じ base からの 2 つの commit。Synergos は commit-ipc 経由で参加 (S1) | Cu (Sy 参加) | G-A1 |
| 2 | dirty な作業の上書き | 未 publish の編集があるファイルに、別ノードの新版を Full / Manual / Selective で受ける。ファイルは不変、`<name>.conflict-<short>.<ext>` がある、`ConflictDetected` が出る。Manual では台帳に載るだけで作業ツリーは不変 (`project pull` 無しの反映 0 件) | Sy | G-A2 |
| 3 | 途中で切れる集合 | 5,000 ファイルの OFPA changeset (`.umap` と外部 actor) の反映中に受信側を kill し、再起動する。作業ツリーは旧 tree か新 tree のどちらか。asset と `.meta` の組が分かれない (Unity の scene + `.meta` の組でも同じ) | Sy | G-A3 |
| 4 | rename と削除 | 500 MB の PSD を rename、別のファイルを削除し、peer が再接続する。rename で内容転送 0 バイト (転送カウンタで計測)。削除したファイルが再接続後に戻らない | Sy | G-A4 |
| 5 | ロック | `main` でロック → feature ブランチで commit → `retained` → merge で解放。Synergos 部分は読み取り専用ビットの切替 (S8) | Cu (Sy 参加) | G-A17 |
| 6 | view | 10^6 エントリと 2 % の view。Synergos 部分は S2 | Cu (Sy 参加) | G-A5 |
| 7 | 再接続の追いつき | ノードを 30 回分の publish の間オフラインにし、再接続する。生の offer なしで 60 秒 + 転送時間以内に head。状態表示が遅れの changeset 数とファイル数を出す。X-AG3 未受理の構成 (poll だけ) でも同じ条件 | Sy | G-A6 |
| 8 | パス | `.Synergos/hooks.toml`、`SYNERG~1`、`CON.txt`、NFD / NFC のかなの組、大小文字だけ違う組、末尾ドット、UTF-16 で 241 文字のパスを publish と受信の両方で試す。windows-latest・macos-latest・ubuntu の 3 runner で全件が拒否または隔離 | Sy | G-A18 |
| 9 | ベンダー | magic link と納品 | Cu | G-A20 |
| 10 | vector | object-model の vector (≥ 40 件、10^5 エントリの prolly tree を含む) を C++・TS・Rust リーダーで符号化・復号。全件がバイト一致 | Sy + Cu | G-A19 |
| 11 | データセットの規模 | 1.28 M エントリの取込み | Cu | G-A7 |
| 12 | clone の往復 | fetch-session の回数 | Cu | G-A8 |
| 13 | 系譜 | 下流照会の p95 | Cu | G-A9 |
| 14 | 権利 | `notAllowed` の伝播。Synergos 部分は edge の失効 60 秒 (SPEC-SY-revocation-bundle) | Cu (Sy 参加) | G-A10 |
| 15 | 消去 | Synergos 部分は S6 (履歴と offload から消えハッシュが残る、peer の offer を拒否) | Cu (Sy 参加) | G-A11 |
| 16 | エージェント | run と `digital_source_type` の無い commit の拒否 | Cu | G-A12 |
| 17 | 汚染 | 悪意ある peer が署名済みエントリに対して改変したバイトを先に配る。履歴ノードは改変バイトを隔離し、本物を受け入れる (先着を勝たせない)。作業ツリーと edge は改変バイトを使わない。`integrity.poison_candidate` が出る。lineage での影響 run の列挙は Curare 文書の試験 | Sy + Cu | G-A13 |
| 18 | 耐久性 | 2 つの複製のうち 1 つを壊して scrub を走らせ、GC を走らせる。scrub が壊れた複製の ack を取り下げ、GC は残る 1 つを消さない。続けてサイト export を custody root 移設の状態で取り、確定済み blob の欠落を非ゼロ終了で報告する (export の報告形式は Curare C5 と共有) | Sy (export は Cu C5 と共有) | G-A15 |
| 19 | GPU | gateway の最初のバッチ。Synergos 部分は edge の全範囲が Bao 検証済み | Cu (Sy 参加) | G-A14 |
| 20 | ワークステーション | Windows でマシンサービスとして入れ、2 つ目のインスタンスを起動し、存在しない `--config` を渡す。状態が `%ProgramData%\Synergos` 配下、2 つ目は拒否、設定不在で起動失敗 (3 つとも 100 %) | Sy | G-A21 |
| 21 | quick-xml (= security AT-SY-SEC-33) | `server/` と `desktop/` の両 workspace で `cargo tree -e features -i quick-xml` を実行する。両方とも出力が空。サーバ workspace の audit が必須で緑、`audit.toml` の除外指定が 0 件。desktop workspace で経路が残る場合は egui バイナリが配布物に無いこと (SECURITY-M30 の失敗時) | Sy (security) | オーナー決定 5 |

### 6.2 追加の受入条件 (Synergos 固有、S 番号)

| # | 対象 | 手順 | 合格条件 | 担保する機構 |
|---|---|---|---|---|
| S1 | 同じ `base_cs` からの 2 つの commit | 2 つの daemon が同じ base から同じパスへ commit-ipc で commit。10^4 回の無作為な交錯は Curare 文書の試験 1 に参加 | 2 つ目は `PathConflict` を返し、両方の blob が残る。失われる更新 0 件 | M05 |
| S2 | 部分ワークスペース (G-A5) | 10^6 エントリの合成 tree と 2 % の view で checkout。母集団は合成 tree (ファイルサイズ分布は転送効率文書の SPEC-SY-transfer-budget のプロファイル) | 取得バイト ≤ 5 %、取得した tree node 数が変更部分木の数に比例 | M10 |
| S3 | ポインタのマージ | 別ファイルを publish した 2 つの git ブランチをマージ | `.synergos/head.json` の衝突 0 件 (`synergos git-merge-pointer`) | M02 |
| S4 | 旧形式取込み | format 1 のプロジェクト (manifest、`state.json`、履歴索引、タグ) を取り込む | 全 blob の BLAKE3 一致 100 %、タグが「提案された alias」になる | M02 |
| S5 | ref 複製の改ざん | 署名を変えた RefUpdate を注入。複製を `max_staleness_s` 超過にする | 改ざん RefUpdate の適用 0 件。古い複製で GC と保護 ref への publish が 100 % 止まる | M04 |
| S6 | 墓標 | purge list の blob を peer が offer・want・配信要求する | 型付きコード `content_erased` で 100 % 拒否。履歴と offload にハッシュだけが残る | M19 |
| S7 | 小ファイル (AGD-GW-19) | 10^4 ファイルの changeset を publish | HEAD の放送が 1 回。`WANT_MANY` の計測は転送効率文書の TG-* | M05 |
| S8 | ロックの cache | Curare で `exclusive` のロックを他者が取り、解放する | 手元のファイルが読み取り専用になり、解放で戻る (切替の取りこぼし 0 件) | M09 |
| S9 | LFS agent | stock の `git lfs push` と `git lfs pull` を `lfs.customtransfer.synergos` 経由で、100 MB × 20 と 1 GB × 2 のファイルで実行 | アップロード・ダウンロードしたバイトが元とバイト一致 100 %。oid を解決できない場合は LFS の `error` 応答で失敗 (別経路への黙った切替 0 件) | M23 |
| S10 | format skew | 新形式の版 N のノードと N−1 のノードを混在させ (`min_fleet_version < N`)、N のノードで 100 回 publish | 書かれた format N のオブジェクト 0 件。N−1 のノードを上げた後に format N の書込が始まる | M25 |
| S11 | edge の配信制限 | gated な `dataset_snapshot` の範囲を、(a) 目的付き LCT 無し、(b) 別の tree を名指す LCT、(c) 範囲を名指さない capability で、各 100 回要求する | (a)(b)(c) すべて 100 % 403。正しい LCT では全範囲が Bao 検証済み | M17, M22 |
| S12 | link の権限 | 参照先の `view` 権限を持たない主体の作業ツリーに `link` エントリを含む tree を反映 | 実体化されたファイル 0 件、`link.forbidden` が出る | M13 |
| S13 | shelf | 指名 peer へ shelf を 10 回共有し、受け手は `shelf apply` をしない | 受け手の作業ツリーに反映された shelf の changeset 0 件。`shelf apply` の後だけ反映 | M10 |
| S14 | 同名プロジェクト | 2 つの org に同じ表示名のプロジェクトを作り、それぞれで 100 回 publish | gossip topic が別。他 org の peer に届いた gossip 0 件 | M03 |
| S15 | 取込判定の門 | `scanning` と `quarantined` のエントリを含む changeset を受理させ、archive 役と edge に複製・配信を要求する | archive に入ったエントリ 0 件、配信 0 件 (`screening_withheld`)。`clean` になった後だけ archive。`quarantined` がソースとして残る件数 0 件 | M18 |
| S16 | commit の型付き拒否 | Curare に 409 `durability_pending`、423 `locked`、422 `companion_missing` を順に返させる | それぞれ `CommitRejected` の対応コードになり、stage した blob が残る。`locked` では `lock_intent` が残り、`companion_missing` は欠けた組を返す。同じ `operation_id` の再送で二重の changeset 0 件 | M05 |
| S17 | 検証鍵の切替 | lock-announce の鍵を切り替え、直前の kid の署名と未知の kid の署名を流す | 直前の kid は `not_after` まで 100 % 受理、未知の kid は記述子を 1 回取り直した後に 100 % 拒否 | M03, M09 |

## 7. 未決事項と判断者

Q-A1〜Q-A11 は Curare 文書と共有する番号。Q-A12 以降は本書が足した問いで、Curare 文書 §7 にも同じ番号で載せる (§5.3)。Curare 文書の Q-CU-2 (記述子の `verify_keys`) と Q-CU-3 (`curare_version_identity()` の置換順序) は Synergos にも影響するので、ここから参照する。

| # | 問い | 判断者 | 本書が置いた既定 |
|---|---|---|---|
| Q-A1 | D-A01 の受理: Curare の ref log を履歴の権威にし、git の索引を任意のポインタにする。`docs/versioning-design.md:10,28,191,205-209,346` と `cpp-migration.md:39-48` (共通改訂表、§4.27) を変える。Synergos が tree を解釈すること (2026-09-08 の精緻化) を認める | neco | 変更提案として記載。`cpp-migration.md` の改訂は回答を待って別に出す。回答までネイティブの versioning / history の移植 (手順 3b) は保留 |
| Q-A2 | D-A02: Curare の無い Synergos (LAN だけのスタジオ) を旧形式の猶予期間後に退役させるか、on-prem appliance のプロファイルだけで提供するか | neco | オブジェクトモデルは Curare 必須。format 1 は読取を無期限に残す |
| Q-A4 | DCC / エンジン adapter の置き場所: Synergos の `clients/` か、LUDIARS の新 repo (プロジェクトコードが要る) | neco | 未決。broker の契約だけを本書で固定する |
| Q-A6 | 証明の透明性ログ: 自前の Rekor か、SPEC-CU-security-event 内の署名 checkpoint 連鎖か | セキュリティ担当 | Curare 文書で扱う。Synergos は tree の再計算照合だけを行う |
| Q-A7 | compliance モードの保持ロック下の消去と、生きたオブジェクトとチャンクを共有するオブジェクトの消去: per-object DEK の crypto-shred か `retain_until` まで待つか。共有チャンクはどちらでも残す | セキュリティ担当 + neco | 共有チャンクは残す。compliance ロック下は crypto-shred |
| Q-A11 | 重複排除の既定範囲: プロジェクト (採用) か組織か (security Q6、transfer Q5 と同じ問い) | neco | プロジェクト |
| Q-A12 | X-AG1 の受理: SPEC-SY-manifest-v2 を「git にコミットする format 2」から「ノードローカルの実体化索引」に変えるか。ポインタは別ファイル `.synergos/head.json` にしたので format 番号は衝突しない。受理しない場合は manifest v2 のコミットを続け、SPEC-SY-workspace-pointer は採らず、git の索引の衝突 (AGD-GW-14) が残る | 転送効率文書の担当 / neco (transfer Q1 と合わせて) | X-AG1 を受理する前提。受理までは SPEC-SY-workspace-pointer を [変更提案] と表記 |
| Q-A13 | `ref_sync.max_staleness_s` (300 秒) と `ref_sync.poll_interval_s` (60 秒) の既定値、および `min_copies` の既定 2 | neco / プロダクトオーナー | 提案値 |
| Q-A14 | egui にしかない競合・設定・転送要求・取消の画面を、egui に残して C++ desktop-gui の要件にするか、今 Tauri へ移すか (§3.9 の比較) | プロダクトオーナー | egui に残し、C++ desktop-gui の要件に書く (A)。security Q3 で egui の利用者がいない、かつ Tauri が暫定面と確かめられたら B を再検討 |
| Q-A15 | X-AG3 の受理: Curare 紐付けプロジェクトの HEAD フレームに Curare 署名の RefUpdate を載せるか | 転送効率文書の担当 / neco | 受理する前提。受理されない場合、追いつき (G-A6) は SPEC-SY-ref-sync の poll (60 秒) だけで行う |
| Q-A16 | X-AG10 の受理: `edge` 役が gateway の範囲を配り、`archive` 役が `{archive_lag, free_bytes}` を node-roles で報告するか | 転送効率文書の担当 / neco | 受理する前提。受理されない場合、M20 の役割報告は heartbeat の拡張だけで行い、gateway の供給元は A4 まで custody だけ |
| Q-A17 | X-AG11 の受理: TRANSFER-M30 を T4 から T2 (A3 の flag day) へ前倒しするか | 転送効率文書の担当 / neco | 前倒しを提案。受理されない場合、A3〜T4 の間は Manual / Selective でも取得は全量 (帯域の非効率だけ) |
| Q-A18 | X-AG12 の受理: ref-log のページに取込判定 `verdicts[]` を載せるか | セキュリティ担当 + Curare 文書の担当 | 受理する前提。受理されない場合、archive・edge は `release` / `dataset_snapshot` から到達可能なエントリだけを扱う |
| Q-CU-2 (Curare) | 記述子の `verify_keys` を入れるか (本書 X-AG13 と同じ問い。purge-list の鍵の権威を tenant record に置く案を本書が足す) | セキュリティ担当 | 入れる。purge-list は org の 1 鍵で、権威は SPEC-CU-tenant-v1 |
| Q-CU-3 (Curare) | `curare_version_identity()` の置換を security の migration と 1 本にまとめる順序。Synergos への影響は A1 の完了時期 (A3 の依存) | セキュリティ担当 + プロダクトオーナー | Curare 文書の既定に従う |

## 8. 参考資料

設計の根拠にした外部資料だけを載せる (出典は `research/vcs-game-ai.md`、`research/ai-dev-data.md`、`research/transfer-efficiency.md`)。† は調査環境からベンダーのサイトへ直接届かず、検索の抜粋で内容を確かめたもの (調査ノートの冒頭の注記による) で、設計を確定する前に読み直す。Curare 文書と同じ資料には同じ URL と同じ † を使う。

**ゲーム制作の版管理**

1. Unity Version Control Smart Locks † — https://docs.unity.com/en-us/unity-version-control/smart-locks
2. Unity Version Control Smart Locks の使い方 † — https://docs.unity.com/en-us/unity-version-control/use-smart-locks
3. Unity Gluon の部分ワークスペース † — https://docs.unity.com/en-us/unity-version-control/gluon/configure-workspace
4. UnityYAMLMerge (Smart Merge) † — https://docs.unity3d.com/ru/current/Manual/SmartMerge.html
5. Plastic SCM xlinks (プロジェクト横断の参照) † — https://docs.plasticscm.com/xlinks/plastic-scm-version-control-xlinks-guide
6. Perforce typemap † — https://perforce.com/manuals/cmdref/Content/CmdRef/p4_typemap.html
7. Perforce streams † — https://ftp.perforce.com/perforce/r19.1/doc/manuals/cmdref/Content/CmdRef/p4_stream.html
8. SVN needs-lock の実装メモ † — https://apache.googlesource.com/subversion/+/fsx/notes/locking/svn-needs-lock-impl.txt
9. Diversion soft locks † — https://docs.diversion.dev/soft-locks
10. Git LFS batch API — https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md
11. Git LFS locking API — https://github.com/git-lfs/git-lfs/blob/main/docs/api/locking.md
12. Git LFS custom transfers — https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md
13. Git LFS の symlink 勧告 CVE-2025-26625 — https://github.com/git-lfs/git-lfs/security/advisories/GHSA-6pvw-g552-53c5
14. Unreal One File Per Actor † — https://dev.epicgames.com/documentation/en-us/unreal-engine/one-file-per-actor-in-unreal-engine
15. Unreal Virtual Assets † — https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-virtual-assets-in-unreal-engine
16. Unreal `FIoHash` † — https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Core/FIoHash
17. Unreal の共有 DDC (Zen) † — https://dev.epicgames.com/documentation/en-us/unreal-engine/set-up-zen-storage-server-as-shared-ddc-for-unreal-engine

**データの版管理と大きいオブジェクト**

18. lakeFS の版管理の内部 (metarange) † — https://docs.lakefs.io/understand/how/versioning-internals/
19. Apache Iceberg spec (refs、保持、スナップショットの期限) † — https://iceberg.apache.org/spec/
20. Dolt storage engine (prolly tree) † — https://docs.dolthub.com/architecture/storage-engine
21. Hugging Face Xet storage † — https://huggingface.co/docs/hub/xet
22. Hugging Face Xet の重複排除 — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/xet/deduplication.md
23. Hugging Face storage limits (フォルダ・repo あたりのファイル数) — https://raw.githubusercontent.com/huggingface/hub-docs/main/docs/hub/storage-limits.md
24. git-annex numcopies † — https://man.archlinux.org/man/git-annex.1
25. DVC run cache † — https://dvc.org/doc/user-guide/pipelines/run-cache
26. git の large-object promisors † — https://git-scm.com/docs/large-object-promisors
27. Oxen (データセット版管理の性能) † — https://docs.oxen.ai/features/performance
28. BLAKE3 — https://github.com/BLAKE3-team/BLAKE3
29. Bao の仕様 (検証付き範囲取得) — https://github.com/oconnor663/bao/blob/master/docs/spec.md
30. FastCDC (Rust 実装) — https://github.com/nlfiedler/fastcdc-rs/blob/master/README.md

**符号化・系譜・学習クラスタ**

31. RFC 8949 §4.2 決定的 CBOR † — https://www.rfc-editor.org/rfc/rfc8949#section-4.2
32. RFC 8785 JSON Canonicalization Scheme † — https://www.rfc-editor.org/rfc/rfc8785
33. OpenLineage 仕様 (`datasetVersion` に tree ID を載せる根拠) — https://github.com/OpenLineage/OpenLineage/blob/main/spec/OpenLineage.md
34. safetensors — https://github.com/huggingface/safetensors
35. s3-connector-for-pytorch (gateway の利用者) — https://github.com/awslabs/s3-connector-for-pytorch
36. Amazon S3 の性能設計 (範囲読み・並列) — https://github.com/awsdocs/amazon-s3-userguide/blob/main/doc_source/optimizing-performance.md

RFC 8949 と RFC 8785 は調査ノートに URL が無いため、規格の正本 URL を直接載せた。調査環境から rfc-editor.org に届かず本文を本調査で読んでいないので † を付けた (`research/transfer-efficiency.md` の冒頭の注記)。DSSE / in-toto の仕様 URL は調査ノートに無いため未確認 (一次資料を本調査で取得していないため。証明の形式は Curare 文書が持ち、本書は tree の再計算照合だけに依存する)。
