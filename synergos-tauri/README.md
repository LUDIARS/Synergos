# synergos-tauri

Synergos の Tauri 2 + React 19 + Vite 8 + Foundation UI ベース GUI。

既存の `synergos-gui` (egui) と並行で残しつつ、新規 GUI 開発の主軸はこちら。
LUDIARS Foundation 共通デザインシステムを使うことで Memoria / Cernere / Actio
等と見た目を揃える。

## レイアウト

- `src-tauri/` — Tauri Rust バックエンド (workspace member)
  - `src/lib.rs` — Tauri command handler エントリポイント
  - `src/ipc.rs` — synergos-core daemon との IPC bridge
  - `src/config.rs` — `synergos-net.toml` の read/write (Settings UI から)
- `frontend/` — React + Vite 8 + TypeScript フロントエンド
  - `src/global.css` — Foundation UI tokens (Cernere/frontend と同期)
  - `src/lib/tauri.ts` — Tauri command 型付きラッパー
  - `src/pages/Dashboard.tsx` — Project / Peer 一覧 + Add ボタン
  - `src/pages/Settings.tsx` — 接続設定の編集 (toml 直書き)

## 開発

事前条件:

- Node 22+
- Rust stable 1.89+
- Synergos workspace の依存ビルドが通ること

```bash
cd synergos-tauri/frontend
npm install
cd ..
# Tauri dev サーバ起動 (frontend Vite + Rust backend)
npx --prefix frontend tauri dev
```

または Rust 側だけビルド確認:

```bash
cargo build -p synergos-tauri --release
```

## 設計メモ

- Tauri command は `tauri::command` で 1 関数 = 1 RPC。React からは
  `@tauri-apps/api/core` の `invoke()` 経由で型付きコールする
- daemon との IPC は **コマンドごとに毎回新規 connect** して短命に終える
  (Tauri command が並行で来た時の mutex 衝突を避ける + daemon 再起動への耐性)
- `BridgeError` は `daemon_not_running` / `daemon` / `io` / `config` /
  `unexpected` のタグ付き enum で serde 経由 JS に渡る
- 設定編集は daemon に IPC せず toml ファイルを直接書く。daemon は起動時にしか
  config を読まないので IPC 経由でも結局再起動が要る → 直書きで単純化

## 既知の制約

- Conflict / Transfer 詳細 UI はまだ無い (Dashboard で件数表示のみ)
- WebSocket からのリアルタイムイベント subscribe はまだ無い (定期 poll で代替)
- Foundation UI の React コンポーネント package 化は未 (`global.css` を都度同期)
