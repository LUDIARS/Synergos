# 管理コンソール (synergos-admin-ui) のビルドと配信

`synergos-control` の管理 API をブラウザから操作するための Web UI。
Dioxus 0.7 (web renderer) の WASM アプリで、control の axum が `/ui/` から静的配信する。

## 1. 構成

```
 ブラウザ ──GET /ui/──▶ synergos-control (axum)
    │                     └─ [ui] dist_path のファイルをそのまま返す
    └──fetch /v1/...──▶ 既存の管理 API (Bearer 必須)
```

- UI は **既存の REST API を叩くだけ**。fullstack server functions は使わない
  (API を単一の正に保ち、CLI / curl と同じ経路にする)
- `/ui/` 自体は無認証で取得できる (静的アセット)。API 呼び出しはすべて
  `Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN` が必要
- 管理トークンは初回アクセス時に入力し、そのタブの `sessionStorage` にだけ保持する。
  タブを閉じれば消える (localStorage は使わない)

## 2. ビルド

`synergos-admin-ui` は wasm32 専用クレートのため、ルート workspace から `exclude` されている。
ビルドはクレートのディレクトリで行う。

```bash
# 初回のみ
rustup target add wasm32-unknown-unknown
cargo install dioxus-cli --locked    # dx コマンド

cd synergos-admin-ui
dx build --release --platform web
```

成果物は `target/dx/synergos-admin-ui/release/web/public` に出る
(`index.html` + JS + `.wasm` + assets)。`Dioxus.toml` の `base_path = "ui"` により、
生成される URL は `/ui/` 配下を指す。

コンパイルの確認だけなら `dx` 無しでもできる (CI の `Admin UI (wasm)` ジョブと同じ):

```bash
cd synergos-admin-ui
cargo build --target wasm32-unknown-unknown --release
```

## 3. 配信の設定

`control.toml` に成果物のパスを書く。

```toml
[ui]
dist_path = "synergos-admin-ui/target/dx/synergos-admin-ui/release/web/public"
```

- 未設定なら `/ui/` は 503 を返し、ビルド手順を案内する (API サーバー単体として動く)
- 設定したのにディレクトリが無い場合は**起動時に落とす**
  (起動は成功したのに `/ui/` だけ 503、という分かりにくい状態を作らない)

起動:

```bash
export SYNERGOS_CONTROL_ADMIN_TOKEN=$(openssl rand -hex 32)
export CLOUDFLARE_API_TOKEN=<token>
synergos-control serve --config control.toml
# → http://127.0.0.1:4250/ui/
```

管理面を Mesh へ広げる場合の bind 注意は `mesh-operations.md` §3.2 と同じ。
UI を足しても管理トークンが唯一の防壁である点は変わらない。

## 4. 画面

| 画面 | できること |
|---|---|
| ダッシュボード | 組織一覧、ノード数 / Mesh node 数 / heartbeat 未着数、dark node の点検 (レポートのみ) |
| 組織 / ノード管理 | 組織作成、ノード一覧 (種別・所有者・Mesh IP・heartbeat)、ノード登録 (connector_token / node_key / enroll_hint 表示)、登録トークン再発行、ノード削除 (2 段階確認) |
| Mesh 自動設定 | Cloudflare API トークンを渡して「検証 → 突合 → 登録トークン発行」を進捗表示付きで実行 |
| セットアップガイド | 管制サーバー起動からノード参加・点検までの手順。OS タブ (Windows / Linux / macOS) 切替とコピー可能なコマンドブロック付き |

各画面の主要操作には「?」ヘルプが付いていて、対応するガイドの節へ飛ぶ。
ノード登録の直後には「次に何をするか」(エンロール手順) への導線が出る。

画面遷移はルーターではなく単一の状態で切り替える。`/ui/` という部分パスで配信するため、
history API の base path 設定に依存しない方が壊れにくい。

## 5. 秘密情報の扱い

| 値 | 扱い |
|---|---|
| 管理トークン | ブラウザの `sessionStorage` のみ。サーバーへは Bearer ヘッダで送る |
| Cloudflare API トークン (UI 入力) | リクエスト本文で送り、そのリクエストの処理中だけ使う。**保存もログ出力もしない**。実行後は入力欄からも消す |
| `connector_token` / `node_key` | control は保存しない (node_key はハッシュのみ)。応答で一度返るだけなので、画面を離れる前にコピーする。紛失時は再発行 |

Mesh 自動設定の API (`/v1/mesh/*`) はすべて管理トークン層の内側にある。
Cloudflare トークンの持ち込み口を無認証にはしない。

また、UI からの突合は **レポートのみ** で `revoke_dark` は行わない。
失効は破壊的なので CLI から明示的に実行する (`mesh-operations.md` §3.5)。

## 6. 実環境での確認手順

Cloudflare 実疎通を含む確認は次の順で行う (CI ではモック / 形式検証のみ)。

1. Cloudflare で Account ID と API token を用意する
   (Account > Cloudflare Tunnel:Edit, Zero Trust:Edit, Device Posture:Read)
2. `dx build --release --platform web` → `control.toml` の `[ui] dist_path` を設定
3. `synergos-control serve` → `http://127.0.0.1:4250/ui/` を開く
4. 管理トークンを入力 → 組織を作成 (`POST /v1/orgs`) → ノード登録
5. 表示された `connector_token` をノードで `sudo warp-cli connector new <token>`
6. Mesh 自動設定タブで API トークンを入力し、3 ステップが完了することを確認
7. ダッシュボードの「dark node を点検」で未登録参加者が 0 であることを確認
