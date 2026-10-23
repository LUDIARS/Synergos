# Excubitor での site 運用 (synergos-control)

synergos-control (メッシュ登録簿 + Curare 向け不透明保管 custody) を Excubitor から導入・起動するための手順。
契約の正本は Castra の `.agents/skills/service-bootstrap/SKILL.md`。

## 構成

| ファイル | 役割 |
|---|---|
| `excubitor.catalog.yaml` | service `synergos-control` (port 4250、`autostart: false`)。`SYNERGOS_STORAGE_URL` を他サービスへ provides |
| `excubitor.bootstrap.json` | setup / export / import の入口 |
| `scripts/site/setup.mjs` | `cargo build --release --locked -p synergos-control`。再実行可、データには触れない |
| `scripts/site/run-control.mjs` | 常駐コマンド。環境変数から `data/site/control.toml` を生成し、release バイナリを前面起動 |
| `scripts/site/export-data.mjs` / `import-data.mjs` | `data/site/` (control-store.json と custody/) を単一 bundle に / から。`control.toml` は含めない |

## データ

既定の保存先は `<repo>/data/site/` (`SYNERGOS_SITE_DATA_DIR` で絶対パスに変更可、`.gitignore` 済み)。
custody の保管物は `data/site/custody/` (`SYNERGOS_CUSTODY_ROOT` で変更可)。

bundle 形式 (format 1): 1 行目が JSON ヘッダ `{magic:"synergos-site-bundle", format, service, files:[{path,size,sha256}]}`、
以降に各ファイルの生バイトを連結。import は bundle 全体の sha256・service・各ファイルの sha256 を検証し、
`..`・絶対パス・ドライブ指定を拒否する。復元先が空でなければ拒否し、一時ディレクトリへ書き切ってから rename する
(途中失敗で復元先は作られない)。export / import はサービス停止中に Excubitor から実行する。

## 秘密 (Excubitor Vault の binding で注入する)

| 環境変数 | 内容 |
|---|---|
| `CF_ACCOUNT_ID` / `CF_API_TOKEN` | Cloudflare (共通 entry を流用。2026-10-23 neco 決定) |
| `SYNERGOS_CONTROL_ADMIN_TOKEN` | 管理 API の bearer (32 byte 以上) |
| `SYNERGOS_CUSTODY_TOKEN` | custody API の bearer (32 byte 以上)。Curare の `SYNERGOS_STORAGE_TOKEN` と同値 |
| `SYNERGOS_CUSTODY_RECEIPT_SECRET` | 受領証 HMAC (32 byte 以上)。Curare の `SYNERGOS_RECEIPT_SECRET` と同値 |

非秘密の設定 (bind・custody の公開 URL / node id / 上限 / 許可 origin) は catalog の `env:` にある。

## 既知の制約

- custody の秘密は Synergos と Curare で同じ値を共有する (再設計 HANDOFF §3 SEC-AZ-13)。C++ リスペックで非対称署名へ置き換える予定。
- `setup` の release ビルドは初回に数分〜十数分かかる。Excubitor の 15 分上限を超える環境では先に手元でビルドしておく。
