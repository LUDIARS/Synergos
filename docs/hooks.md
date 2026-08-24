# publish / 受信時フック

git hooks 相当の仕組み。ファイル publish (配信) / 受信完了のタイミングで任意のスクリプト
(CI 起動・オートコンバート等) を実行できる。

## 定義場所 (2 層)

1. **プロジェクト単位**: `<project root>/.synergos/hooks.toml` — git にコミットして
   チームで共有できる。
2. **daemon 単位**: `synergos.toml` (daemon の設定ファイル) の `[hooks]` — ノード固有
   (CI キック等)。

### セキュリティ (opt-in)

`.synergos/hooks.toml` はリポジトリ由来スクリプトの自動実行になる。daemon 設定で

```toml
[hooks]
allow_project_hooks = true   # 既定 false
```

を立てたノードだけがプロジェクトフックを実行する。**既定は無効**。信頼できるノードだけで
有効化すること。この設定は daemon 全体に適用されるため、有効化したノードでは開く全プロジェクト
を信頼する必要がある。daemon 設定 (`[[hooks.hooks]]`) は常に有効
(リポジトリ由来ではないため)。

## イベント

| イベント | タイミング | 失敗時 |
|---|---|---|
| `pre-publish` | version 発番・manifest 更新の**前** (オートコンバート用: フックがファイルを書き換えたら書換後の内容で publish) | 非 0 exit で publish 中止・理由表示 |
| `post-publish` | manifest 更新・Offer 送出の後 | ログ警告のみ |
| `post-receive` | 受信ファイルの作業ツリー反映後 (CI/コンバート起動用) | ログ警告のみ |

`pre-publish` だけ同期待ち (publish をブロックしてよい)。`post-publish` / `post-receive` は
spawn するだけで待たない (転送・イベントループをブロックしない)。post 系フックの実行は
daemon 全体で最大 16 バッチに制限され、超過分は非同期に待機する。

## hooks.toml 形式

```toml
[[hook]]
event = "post-receive"
command = "python scripts/convert.py"   # project root を cwd に、シェル経由で実行
match = ["assets/**/*.png"]             # glob。省略 = 全ファイル
timeout_sec = 120                        # 既定 60。超過 kill
```

daemon 設定 (`synergos.toml`) では同じ形を `[hooks]` の下の配列で書く:

```toml
[hooks]
allow_project_hooks = true

[[hooks.hooks]]
event = "pre-publish"
command = "scripts/lint-assets.sh"
timeout_sec = 30
```

### フィールド

| フィールド | 必須 | 既定 | 説明 |
|---|---|---|---|
| `event` | ○ | — | `pre-publish` \| `post-publish` \| `post-receive` |
| `command` | ○ | — | project root を cwd に、シェル経由で実行 (Windows: `cmd /C`、Unix: `sh -c`) |
| `match` | — | 全ファイル | glob パターンの配列。`*` は `/` を跨がない、`**` は跨ぐ、`?` は任意 1 文字 |
| `timeout_sec` | — | `60` | 超過したらプロセスを kill する |

複数ファイルを 1 回の `publish` で送る場合、フックは **1 ファイルごとに発火**する
(`match` にマッチしたファイルだけ)。

## 環境変数

フックプロセスには以下が渡る:

| 変数 | 内容 |
|---|---|
| `SYNERGOS_EVENT` | `pre-publish` \| `post-publish` \| `post-receive` |
| `SYNERGOS_PROJECT` | project_id |
| `SYNERGOS_FILE` | プロジェクトルート相対パス (`/` 区切り) |
| `SYNERGOS_VERSION` | ファイルバージョン (`pre-publish` は発番前なので未設定) |
| `SYNERGOS_PEER` | 送信元ピア ID (`post-receive` のみ) |

## CLI

```bash
# 有効なフック一覧 (定義元 daemon/project の別と opt-in 状態を表示)
synergos-core hooks ls <project>

# 手動発火 (デバッグ用)
synergos-core hooks run <project> <event> <file>
# 例: synergos-core hooks run myproj post-receive assets/icon.png
```

手動発火では実際の publish/受信 version と送信元 peer を特定できないため、イベント種別に
かかわらず `SYNERGOS_VERSION` / `SYNERGOS_PEER` は設定されない。

## 設定例

### PNG 自動変換 (post-receive)

受信した PNG をローカルの用途向けフォーマットに変換する:

```toml
# <project root>/.synergos/hooks.toml
[[hook]]
event = "post-receive"
command = "python scripts/convert_png_to_ktx.py \"$SYNERGOS_FILE\""
match = ["assets/**/*.png"]
timeout_sec = 120
```

チーム全員のノードで動かしたいので `.synergos/hooks.toml` に置き、実行したいノードだけ
`allow_project_hooks = true` を立てる。

### CI キック (post-publish, daemon 単位)

publish されたビルド設定ファイルを CI に通知する。CI エンドポイントはノード固有の秘密
(トークン等) を含むことが多いので daemon 側 (`synergos.toml`) に置く:

```toml
# synergos.toml (daemon 固有、git にコミットしない)
[hooks]
allow_project_hooks = false

[[hooks.hooks]]
event = "post-publish"
command = "curl -X POST https://ci.example.invalid/trigger -H \"Authorization: Bearer $CI_TOKEN\" -d \"file=$SYNERGOS_FILE&version=$SYNERGOS_VERSION\""
match = ["levels/**/*.unity"]
timeout_sec = 30
```

### pre-publish によるオートコンバート

publish 前に大きい PSD をリポジトリ標準の PNG に変換してから配る (フックが書き換えた
後の内容で publish される):

```toml
[[hook]]
event = "pre-publish"
command = "python scripts/flatten_psd.py \"$SYNERGOS_FILE\""
match = ["design/**/*.psd"]
timeout_sec = 300
```

変換が失敗 (非 0 exit) すれば publish は中止され、manifest は変わらない。
