# Synergos とバージョン管理の兼ね合い — 設計

対象: 「バイナリ / 大きいファイルの差分をどう管理するか、あるいは管理しないか」と、
git (テキストのバージョン管理) との役割分担。

## 0. 結論 (先に)

| 問い | 答え |
|---|---|
| Synergos で **履歴 (差分管理)** を持つか | **持たない**。Synergos は「今の資産集合を全ノードに揃える転送層」に徹する。履歴は git 側 (`.synergos/manifest.json` をコミット) に持たせる |
| バイナリ / 大きいファイルの **差分転送** をするか | **する (Phase 2)**。ただし「バイト差分 (patch)」ではなく **内容アドレスのチャンク重複排除** で実現する。差分"管理"と差分"転送"を分けて考える |
| git との関係 | **git-LFS の P2P 版**。git はテキスト + `manifest.json` (=ポインタ集合) を管理し、実体 (blob) は Synergos が運ぶ。git にバイナリを入れない |
| 競合 | 同じ版番号で内容が違う Offer は **衝突として通知し、ローカル版を保持**。自動マージはしない (バイナリはマージ不能)。退避・選択 UI とロック機構は Phase 3 |

理由: 3 択を LUDIARS の判断 4 軸 (AI 学習量 / 作業コスト / 目的達成度 / 主目的との一致度) で比べると
下表のようになり、案 B (ポインタ + チャンク重複排除) が主目的「チームでアセットを揃えて
作業を続ける」に最も一致し、実装コストも既存コード (CID / Block / Bitswap 骨格) の延長で済む。

| 案 | 内容 | AI 学習量 | 作業コスト | 目的達成度 | 主目的との一致 |
|---|---|---|---|---|---|
| A. 差分管理しない (最新ミラーのみ、現状) | 版番号 + 全体転送。履歴なし | 小 | **済** (現行実装) | 中: 揃うが大容量更新のたび全転送・巻き戻し不可 | 中 |
| **B. ポインタ + チャンク重複排除 (推奨)** | `manifest.json` を git 管理、blob は CDC チャンクで P2P、差分転送のみ | 中 (CDC / DAG は既存設計の実装) | 中 (Phase 2 で 3〜4 タスク) | **高**: 大容量差分・巻き戻し (git checkout + `synergos checkout`)・重複排除 | **高** |
| C. Synergos 内蔵の履歴チェーン (DESIGN §14 原案) | ファイルごとの直列チェーン、text diff / binary CID を Synergos が保持・GC | 大 (VCS を作り直す) | 大 | 高だが git と二重管理になり運用が割れる | 低〜中 (主目的は同期であって VCS 再発明ではない) |

案 C は DESIGN.md §14 に残っている原案だが、**git が既にある場所で第二の履歴系を持つと
「どちらが正か」が常に問題になる**。Synergos は履歴を持たず、履歴は git に一本化する。

---

## 1. 用語を分ける

| 用語 | 意味 | Synergos での扱い |
|---|---|---|
| **差分管理 (delta storage / history)** | 旧版を復元できるように差分や旧版を保存すること | **しない** (git に委ねる) |
| **差分転送 (delta transfer)** | 変わった部分だけ送ること | **する** (Phase 2: チャンク重複排除) |
| **重複排除 (dedup)** | 同じ内容を二度持たない/送らない | チャンクの blake3 で行う。差分転送はこの副産物 |
| **バージョン (version)** | 「同じパスの内容が何回変わったか」の単調増加番号 | `manifest.json` の `version`。転送の要否判定に使う (§2) |
| **ポインタ / ロック** | パス → 内容ハッシュ (+size, version) の表 | `manifest.json` そのもの。git にコミットすると「そのコミット時点の資産集合」が固定される |

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
  .synergos/objects/
  .synergos/state.json
```

- テキスト (コード / .meta / prefab など git で diff できるもの) → **git**
- バイナリ・大容量 → **Synergos** (`project publish`)。git には `.gitignore` で入れない
- `manifest.json` を **コミットする**と「このコミットの時点で assets/big.bin は v3 (crc …)」が残る。
  巻き戻しは Phase 2 の `synergos project checkout` (§3.4) で実現。Phase 1 では
  記録としてだけ使う (人が「v3 に戻して」と依頼する材料)

---

## 3. Phase 2: チャンク重複排除による差分転送 (推奨案 B の本体)

### 3.1 データモデル

```
File (path)  ──►  FileManifest { chunks: [ChunkRef], total_hash, size, version }
                          │
                          └─► ChunkRef { hash: blake3, offset, len }
                                        │
                                        └─► Object store  .synergos/objects/<hh>/<hash>
```

- **チャンク分割は Content-Defined Chunking (FastCDC)**: 平均 1 MiB (min 256 KiB / max 4 MiB)。
  固定長だと先頭に 1 バイト挿入されただけで全チャンクがずれるので、境界は内容で決める。
  PSD / FBX / 音声 / 圧縮済み .png のような「一部だけ変わる大ファイル」に効く
- **チャンク ID = blake3(内容)** (既存 `Cid` / `Block` をそのまま使う)
- **Object store はプロジェクト内 `.synergos/objects/`**。git ignore 対象。
  同じ内容のチャンクはパスが違っても 1 回しか持たない (重複排除)
- `manifest.json` の各エントリに `content_hash` (ファイル全体の blake3) と
  `chunks` の要約 (件数・マニフェスト blob の CID) を追加。**format: 2**
- git にコミットして意味がある値 (path → version / size / content_hash) と、ノード固有の値
  (`publisher`, `updated_at`, 受信時刻) を **`manifest.json` と `.synergos/state.json` に分ける**。
  Phase 1 の manifest は両方を 1 ファイルに持っているので、コミットすると A と B で
  `publisher` の差分ノイズが出る (Phase 2 で分離、format 2 への移行時に自動変換)

### 3.2 転送

1. publisher: publish 時にチャンク分割 → objects に無いチャンクだけ書く → FileManifest を作り
   `FileOffer{ version, content_hash, manifest_cid }` を gossip
2. receiver: 手元の旧版 FileManifest と突合し **無いチャンクだけ** Bitswap で要求 (`want` = chunk hash 一覧)
3. 受信したチャンクを objects に置き、旧版のチャンク + 新チャンクから
   `.synergos/incoming/<uuid>.part` を組み立て → blake3 全体検証 → rename
4. 期待効果: 500 MB のファイルの 10 % 更新 → 送るのは ~50 MB + マニフェスト

既存コードとの対応: `synergos-net/src/content/` (Block / Cid / MemoryContentStore / BitswapSession) は
このためのもの。**MemoryContentStore を `.synergos/objects/` 永続ストアに差し替える**のが最初の一歩。

### 3.3 GC

- objects は「現在の manifest から参照されているチャンク」+「直近 N 世代 (既定 2) の
  FileManifest が参照するチャンク」だけ残し、他は `synergos project gc` で削除
- 直近 N 世代を残す理由: 巻き戻し (§3.4) と、書き戻し途中の再送を安くするため。
  それ以上の履歴は git がポインタを持ち、実体は **publisher か常駐ノードが持っていれば取り直せる**
  (誰も持っていなければ取れない = git-LFS サーバが消えたのと同じ。常駐ノード運用が前提)

### 3.4 git との統合コマンド

| コマンド | 動作 |
|---|---|
| `synergos project publish <id> <files...>` | (既存) チャンク化 + manifest 更新 + Offer |
| `synergos project status <id>` | manifest と作業ツリーの差 (変更 / 未 publish / 未取得) を表示 — `git status` 相当 |
| `synergos project checkout <id> [--manifest <path>]` | 指定 manifest (既定: 作業ツリーの `.synergos/manifest.json`、= `git checkout` 後の状態) に**作業ツリーを合わせる**。足りないチャンクはピアから取得。これで「そのコミット時点のアセット」が復元できる |
| `synergos project gc <id>` | §3.3 |

想定フロー: `git pull` → `manifest.json` が更新される → `synergos project checkout myproj` →
アセットが揃う。逆に、アセットを publish したら `manifest.json` が変わるので
**それをコミットして push** する (テキストの変更と同じ PR に載る)。

### 3.5 なぜ text diff (unified patch) をやらないか

DESIGN §14.2 の「テキストは git diff をチェーンに書く」は捨てる。テキストは git が
既に完璧に扱うし、Synergos 側でパッチ適用を持つと **git と二重の真実**になる。
テキストを Synergos で運びたい場面 (git を使わない相手 / 生成物) でも、
チャンク重複排除で十分小さくなる (テキストは小さい)。

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

1. Synergos は履歴を持たない。`.synergos/manifest.json` が唯一の状態で、git がそれを版管理する
2. `manifest.json` は **git にコミットする**、`.synergos/objects/` `.synergos/incoming/` は **ignore**
3. version は path ごとの単調増加。転送要否は version と content_hash で決める (時刻は使わない)
4. 差分転送はチャンク重複排除 (CDC + blake3)。バイト差分 / パッチは作らない
5. テキストの diff は git の仕事。Synergos は種別を区別せず全部チャンクとして運ぶ
6. 競合は通知してローカル版を保持する。Phase 3 で別名退避と手動選択を追加し、自動マージはしない

---

## 6. 関連

- 現状の完成度と残作業: [operations-readiness.md](operations-readiness.md)
- 2 台で動かす手順: [two-node-operations.md](two-node-operations.md)
- 原案 (チェーン / Want-Offer 台帳): [../DESIGN.md](../DESIGN.md) §14–§16 (本設計で §14.2/§14.3 は置き換え)
