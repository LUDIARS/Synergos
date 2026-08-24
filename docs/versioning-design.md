# Synergos とバージョン管理の兼ね合い — 設計

対象: 「バイナリ / 大きいファイルの差分をどう管理するか、あるいは管理しないか」と、
git (テキストのバージョン管理) との役割分担。

## 0. 結論 (先に)

| 問い | 答え |
|---|---|
| Synergos で **履歴 (差分管理)** を持つか | **差分管理はしない**。履歴の「索引」は git 側 (`.synergos/manifest.json` をコミット)。履歴の「実体」は **フラグを立てた履歴ノード (history node) が版ごとに丸ごと保持**する (Phase 2)。通常ノードは最新版のみ |
| バイナリ / 大きいファイルの **差分転送** をするか | **しない**。転送は Phase 1 のまま全体転送 (blake3 検証)。差分の検知は `version` + size + CRC で足りる。CDC チャンク重複排除案は neco 決定 (2026-08-16) で取り下げ |
| git との関係 | **git-LFS の P2P 版**。git はテキスト + `manifest.json` (=ポインタ集合) を管理し、実体 (blob) は Synergos が運ぶ。**履歴ノード = LFS サーバに相当**。git にバイナリを入れない |
| 競合 | 同じ版番号で内容が違う Offer は **衝突として通知し、ローカル版を保持**。自動マージはしない (バイナリはマージ不能)。退避・選択 UI とロック機構は Phase 3 |

理由: 4 択を LUDIARS の判断 4 軸 (AI 学習量 / 作業コスト / 目的達成度 / 主目的との一致度) で比べると
下表のようになる。当初は案 B (チャンク重複排除) を推していたが、**neco 決定 (2026-08-16) で案 D
(履歴ノードフラグ) を採用**した。差分の機構を作らなくても「巻き戻せる」「揃う」は満たせ、
実装は Phase 1 の転送層と manifest の上に薄く載る。

| 案 | 内容 | AI 学習量 | 作業コスト | 目的達成度 | 主目的との一致 |
|---|---|---|---|---|---|
| A. 差分管理しない (最新ミラーのみ、現状) | 版番号 + 全体転送。履歴なし | 小 | **済** (現行実装) | 中: 揃うが大容量更新のたび全転送・巻き戻し不可 | 中 |
| B. ポインタ + チャンク重複排除 (取り下げ) | `manifest.json` を git 管理、blob は CDC チャンクで P2P、差分転送のみ | 中 (CDC / DAG は既存設計の実装) | 中〜大 (Phase 2 で 3〜4 タスク、チャンク化・DAG・GC) | 高: 大容量差分・巻き戻し・重複排除 | 高 |
| C. Synergos 内蔵の履歴チェーン (DESIGN §14 原案) | ファイルごとの直列チェーン、text diff / binary CID を Synergos が保持・GC | 大 (VCS を作り直す) | 大 | 高だが git と二重管理になり運用が割れる | 低〜中 (主目的は同期であって VCS 再発明ではない) |
| **D. 履歴ノードフラグ (採用)** | 転送は A のまま。`history.enabled = true` のノードだけが受信/publish した全版の実体を丸ごと保持し、旧版 `FileWant` に応答する | 小 (既存の版付き Want/Offer と manifest の延長) | **小〜中** (保存・索引・応答・checkout/restore・保持ポリシー) | 高: 巻き戻し・喪失防止は満たす。差分転送はしない (帯域は LAN/Mesh で足りる前提) | **高** |

案 C は DESIGN.md §14 に残っている原案だが、**git が既にある場所で第二の履歴系を持つと
「どちらが正か」が常に問題になる**。Synergos は「何が何版か」の履歴を持たず、それは git に一本化する。
案 D の履歴ノードが持つのは **版の実体だけ**で、索引の正は依然 git 側の manifest にある
(履歴ノードは「消えた実体を取り直せる倉庫」であって、第二の履歴系ではない)。

---

## 1. 用語を分ける

| 用語 | 意味 | Synergos での扱い |
|---|---|---|
| **差分管理 (delta storage)** | 旧版を差分として保存すること | **しない** |
| **履歴の実体保持 (history retention)** | 旧版を丸ごと保存すること | **履歴ノードだけがする** (Phase 2、フラグで指定)。索引は git の manifest |
| **差分転送 (delta transfer)** | 変わった部分だけ送ること | **しない** (全体転送のまま。差分検知は version + size + CRC) |
| **重複排除 (dedup)** | 同じ内容を二度持たない/送らない | 履歴ノードの保管庫でファイル全体の blake3 で行う (同じ内容の版は 1 回しか置かない) |
| **バージョン (version)** | 「同じパスの内容が何回変わったか」の単調増加番号 | `manifest.json` の `version`。転送の要否判定に使う (§2) |
| **ポインタ / ロック** | パス → version / size / CRC の表 | `manifest.json` そのもの。git にコミットすると「そのコミット時点の資産集合」が固定される |

---

## 2. Phase 1 (現行実装): 版番号 + 全体転送 + マニフェスト

```
<project root>/.synergos/manifest.json
{
  "format": 1,
  "project_id": "myproj",
  "files": {
    "assets/big.bin": { "version": 3, "size": 524288000, "crc": 2894113452,
                        "updated_at": 1755250000000, "publisher": "peer-abcd..." },
    "levels/01.unity": { "version": 1, ... }
  }
}
```

- publisher: `project publish` のたびに **内容 (size+CRC) が変わっていれば version+1**。同じなら据え置き (再送しない)
- receiver: 受信完了時に `version` を記録。手元の版 ≥ 提示版 なら auto-pull しない
- daemon 再起動時はマニフェストから Offer 台帳を復元 (publisher が落ちても再 publish 不要)
- 転送は **ファイル全体** (64 KiB フレーム + blake3 検証)。大きいファイルは丸ごと流れる
- パスは `/` 区切りに正規化 (Windows ↔ Linux 混在で壊れない)

**git との付き合い方 (Phase 1 の運用ルール)**

```
.gitignore
  # Synergos が運ぶもの (git に入れない)
  assets/**/*.png
  assets/**/*.fbx
  *.bin
  # Synergos の作業領域 (manifest.json だけはコミットする)
  .synergos/incoming/
  .synergos/history/
  .synergos/state.json
```

- テキスト (コード / .meta / prefab など git で diff できるもの) → **git**
- バイナリ・大容量 → **Synergos** (`project publish`)。git には `.gitignore` で入れない
- `manifest.json` を **コミットする**と「このコミットの時点で assets/big.bin は v3 (crc …)」が残る。
  巻き戻しは Phase 2 の `synergos project checkout` / `restore` (§3.4) で、履歴ノードから
  実体を取り直して実現。Phase 1 では記録としてだけ使う (人が「v3 に戻して」と依頼する材料)

---

## 3. Phase 2: 履歴ノード (history node) — 実体の履歴を持つノードをフラグで指定

neco 決定 (2026-08-16): 差分管理 (CDC チャンク化・差分転送) は**やらない**。代わりに、
設定フラグを立てたノードが publish / 受信した **各 version の実体を丸ごと保持**し、
他ノードからの旧版要求に応答する。差分の検知は Phase 1 の `version` + size + CRC で行う。

### 3.1 役割

| ノード種別 | 作業ツリー | 保持する実体 | 応答する FileWant |
|---|---|---|---|
| 通常ノード (既定) | 最新版 | 最新版のみ (Phase 1 のまま) | 手元の manifest と一致する version だけ |
| **履歴ノード** (`history.enabled = true`) | 最新版 (同じ) | 対象プロジェクトの **全 version** (保持ポリシー内) | **任意の version** (保管庫にあれば) |

- 少人数チームでは **常駐ノード 1 台 (例: AWS Linux) を履歴ノード**にする。git-LFS サーバに相当する
- 履歴ノードは複数あってよい (全部が保持する = 冗長化)。相互の同期はしない (各自が見た版を持つ)。
  取り逃した版は、保持している別の履歴ノードから取得する。どの履歴ノードも保持していなければ、
  通常ノードだけでは旧版を復元できない
- 履歴ノードが 1 台も無い構成は Phase 1 と同じ挙動 (旧版は誰も持たない)。動くが巻き戻せない

### 3.2 設定 (フラグ)

daemon 設定 (`synergos.toml`) に `[history]` セクションを追加。**既定は無効**。

```toml
[history]
enabled = true                # このノードを履歴ノードにする
projects = ["*"]              # 対象プロジェクト (既定 "*" = 参加中すべて)
root = ".synergos/history"    # 保管庫 (プロジェクト root 相対。別ドライブを指す絶対パスも可)
max_versions_per_file = 0     # 0 = 無制限。N なら path ごとに新しい N 版を残す
max_age_days = 0              # 0 = 無制限
max_bytes = 0                 # 0 = 無制限。超えたら古い順に削る (manifest から参照中の版は削らない)
```

- 有効化/無効化は再起動で反映。無効化しても保管庫は消さない (`synergos history gc --purge` で明示削除)
- `projects` に無いプロジェクトの版は保持しない (通常ノードとして振る舞う)

### 3.3 保管庫のデータモデル

```
<root>/
  objects/<hh>/<blake3 hex>            # 実体。ファイル全体の blake3 で内容アドレス (同じ内容は 1 回)
  objects/<hh>/<blake3 hex>.meta.json  # 復旧用 sidecar (refs: project / path / version / size / stored_at)
  index.json                            # (project_id, path, version) -> ObjectRef
```

```json
{ "format": 1,
  "project_id": "myproj",
  "entries": {
    "assets/big.bin": {
      "3": { "hash": "<64桁 blake3 hex>", "size": 524288000, "crc": 2894113452,
             "stored_at": 1755250000000, "publisher": "peer-abcd…", "source": "received" },
      "2": { "hash": "<64桁 blake3 hex>", "size": 0, "stored_at": 0, "source": "published" }
    }
  }
}
```

絶対 `root` ではプロジェクト間の衝突・path traversal を避けるため、実際の保管先を
`<root>/<blake3(project_id)>/` とする。相対 `root` は `..` と symlink 経由の脱出を拒否する。

- 版の実体は **チャンク化しない**。ファイル全体を objects に置く。重複排除はファイル単位のみ
- 同一内容は複数の path / version から参照され得るため、sidecar の `refs` にはそれぞれの
  index エントリを列挙する。`index.json` は atomic write (tmp + rename)。破損時は objects の
  `.meta.json` を走査して再構築する
- 作業ツリー側は Phase 1 と同じ (最新版が置かれる)。履歴ノードは受信/publish 完了時に
  **作業ツリーへの反映と同時に objects へコピー**する。ハードリンクは使わない
  (publisher の作業ツリーは人が in-place 編集するので、リンクだと保管した旧版が後から書き換わる)
- Phase 1 の manifest.json は変更しない (format 1 のまま)。履歴ノードの索引は node ローカルの
  `<root>/index.json` (既定では `.synergos/history/index.json`) で、git には**入れない**
  (`.gitignore` に `.synergos/history/`)

### 3.4 転送と git 統合コマンド

転送プロトコルは Phase 1 のまま (`FileOffer{version}` / `FileWant{version}` / 全体転送 + blake3)。
変わるのは **誰が旧版 FileWant に応答するか**だけ:

1. 通常ノードは手元 manifest の version と一致する FileWant にだけ応答 (現状)
2. 履歴ノードは `index.json` に (project, path, version) があれば応答し、objects から送る
3. 要求側は最初に返ってきた Offer から受信 (Phase 1 と同じ)。blake3 で転送中の破損を検出する

| コマンド | 動作 |
|---|---|
| `synergos project publish <id> <files...>` | (既存) manifest 更新 + Offer。履歴ノードなら自分の publish 版も objects に置く |
| `synergos project status <id>` | manifest と作業ツリーの差 (変更 / 未 publish / 未取得) を表示 — `git status` 相当 |
| `synergos project checkout <id> [--manifest <path>]` | 指定 manifest (既定: 作業ツリーの `.synergos/manifest.json`、= `git checkout` 後の状態) に**作業ツリーを合わせる**。手元に無い版は FileWant(version) を出し、その版を保持する履歴ノードから取る |
| `synergos project restore <id> <path> --version N` | 1 ファイルだけ指定版に戻す (manifest も N に書き戻す)。自ノードが履歴ノードで実体を持っていればネットワーク無しで差し替える |
| `synergos history ls <id> [<path>]` | 履歴ノード上の保持版一覧 (version / size / stored_at / source) |
| `synergos history gc [--purge] [--keep-manifest <path>...]` | §3.5 の保持ポリシーを適用。`--purge` は保管庫全消去 |
| `synergos tag add <id> <name> [--manifest <path> \| --file <path> --version N]` | 版タグを作成/上書き。省略時は現在の manifest、`--manifest` は指定 manifest、`--file`+`--version` は単一ファイル版をピン (§3.5) |
| `synergos tag ls <id>` | タグ一覧 (name / created_at / pin 数) |
| `synergos tag show <id> <name>` | タグのピン内容一覧 |
| `synergos tag rm <id> <name>` | タグ削除 (実体は消さない。以後 GC 対象に戻るだけ) |

想定フロー: `git pull` → `manifest.json` が更新される → `synergos project checkout myproj` →
アセットが揃う (新しい版は publisher から、古い版に戻す場合は履歴ノードから)。逆に、
アセットを publish したら `manifest.json` が変わるので**それをコミットして push** する。

**巻き戻し後の publish と版番号**: checkout / restore で v1 に戻したノードが再 publish
すると、単純な +1 では他ノードが既に持つ v2 と番号が衝突する (同 version 別内容 = Conflict)。
そこで publish の版は `max(手元 + 1, これまで manifest / 検証済み転送で確定した最大版 + 1)` にする。
観測最大版は node-local の `.synergos/state.json` に永続化し、daemon 再起動や git checkout
でも失わない。上の例では v3 が発番され、履歴ノードには v1/v2/v3 が残る。
checkout / restore で「手元より古い版」を受け入れるのは、その (project, file, version) を
明示要求 (pin) したときだけで、通常の Offer 経由では今までどおり古い版を拒否する。pin は
5 分で失効し、その後に publish / 受信した版があれば直ちに無効化して遅延応答の上書きを防ぐ。

### 3.5 保持ポリシー (GC)

履歴ノードだけが対象。通常ノードには GC 対象がない。

- 削除候補 = `max_versions_per_file` / `max_age_days` / `max_bytes` のいずれかを超えた版
- **削らない版**: 手元 manifest が参照する最新版、`--keep-manifest <path>` で渡された
  manifest (例: git の各リリースタグ時点) が参照する版、および**版タグが指す版** (下記)
- objects はどの index エントリからも参照されなくなったら削除 (参照カウントは index を走査して数える)

**版タグによる保護**: `synergos tag add` で名前付きの (path → version) ピン集合をタグとして
保存できる (git tag に相当)。保存場所は履歴ノードの保管庫直下 `<store_dir>/tags/<name>.json`
(node ローカル。git には入れない — 索引の正は git 側 manifest というルールに反しない: タグは
保持保護のためのローカル指定であって第二の履歴系ではない)。タグ名は `[A-Za-z0-9._-]{1,64}`
でパス脱出を拒否する。

- `history gc` は全タグの pins を保護集合に自動的に合流させる (`--purge` は例外で、タグの
  有無に関わらず保管庫を全消去する)
- タグが指す版が保管庫に無い場合は警告を出すだけでエラーにしない (削除済み・未取得でも
  タグ自体は残せる)
- `tag rm` はタグの実体 (pin 定義) を消すだけで、それが指していた版は消さない。以後の GC で
  他の保護根拠が無ければ通常どおり削除候補に戻る
- 後続の外部ストレージローテーションからも同じ保護を再利用できるよう、「保護済み
  (path, version) 集合を返す」関数を history モジュールの公開 API として切り出している
  (`HistoryStore::protected_versions(project_root, project_id, extra_keep)`)。呼び出し側は
  手元 manifest 等の追加保護 (`extra_keep`) を渡すだけでよく、タグの合流は内部で行われる

### 3.6 なぜ差分 (CDC / text diff) をやらないか

- 主目的は「チームでアセットを揃えて作業を続ける」で、**巻き戻せる・消えない**が満たせれば足りる。
  差分転送は帯域の最適化であり、LAN / Cloudflare Mesh の帯域では必須ではない
- CDC + DAG + チャンク GC は実装・検証とも重く、壊れたときの調査が難しい。履歴ノードは
  「ファイルを丸ごと置いてあるだけ」なので、最悪 index が壊れても objects を見れば復旧できる
- テキストは git が既に完璧に扱う。Synergos 側でパッチ適用を持つと **git と二重の真実**になる
  (DESIGN §14.2 の text diff チェーンは捨てる)
- 将来「大容量の一部更新が頻繁で帯域が足りない」が実測で出たら、履歴ノードの objects を
  チャンク化する形で案 B を**履歴ノードの内側だけに**後付けできる (通常ノードの挙動は変えない)

---

## 4. Phase 3: 競合と所有権

バイナリはマージできないので、競合は **防ぐ**か **検出して人に返す**の二択。

- **検出 (Phase 1 の最小実装)**: 手元と同じ `version` なのに size / CRC が違う
  `FileOffer` は `ConflictManager` に登録して通知し、ローカルファイルを上書きしない。
- **退避・解決 (Phase 3)**: 親バージョンを Offer に載せて分岐を厳密に検出し、リモート版を
  `<name>.conflict-<peer>-v<N>` として別名受信する。人が採用版を選ぶまで現行版を維持する
- **防止 (任意)**: `synergos project lock <id> <path>` — git-LFS の file locking 相当。
  ロック情報は gossip で配り、ロック中ファイルの publish を他ノードで拒否する。
  少人数チームでは「Slack で一言」で足りることが多いので、Phase 3 で必要になってから

---

## 5. 決めごとまとめ (実装者向け)

1. git 管理する版ポインタは `.synergos/manifest.json`。単調増加発番用の観測高水位だけは node-local の `.synergos/state.json` に持つ
2. `manifest.json` は **git にコミットする**、`.synergos/history/` `.synergos/incoming/` `.synergos/state.json` は **ignore**
3. version は path ごとの単調増加。転送要否は version・size・CRC で決める (時刻は使わない)
4. 差分管理・差分転送はしない。**履歴の実体は `history.enabled = true` のノードだけが版ごとに丸ごと保持**し、旧版 FileWant に応答する
5. テキストの diff は git の仕事。Synergos は種別を区別せず全部ファイル単位で運ぶ
6. 競合は通知してローカル版を保持する。Phase 3 で別名退避と手動選択を追加し、自動マージはしない
7. 履歴ノードの保管庫は content-addressed (ファイル全体 blake3) + node ローカル index。git には入れない

---

## 6. 関連

- 現状の完成度と残作業: [operations-readiness.md](operations-readiness.md)
- 2 台で動かす手順: [two-node-operations.md](two-node-operations.md)
- 原案 (チェーン / Want-Offer 台帳): [../DESIGN.md](../DESIGN.md) §14–§16 (本設計で §14.2/§14.3 は置き換え)
- 決定履歴: 2026-08-15 案 B (CDC 差分転送) を推奨 → 2026-08-16 neco 決定で案 D (履歴ノードフラグ) に変更
