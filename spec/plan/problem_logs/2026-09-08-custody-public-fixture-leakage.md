# 受領証テストの公開固定値によるレビュー停止

- Date: 2026-09-08
- Status: fixed in working tree
- Area: synergos-custody / Cuとの受領証互換テスト
- Severity: Revisor #1543の自動審査を停止

## Summary

Revisorが情報漏えい候補1件を検出し、security scanと後続審査を停止した。

## Evidence

基準c5d4ca4からc163c3eの差分をRevisorのscanAddedDiffForLeaksで再検査すると、
synergos-custody/src/signature.rs:64にembedded-credentialが1件検出された。
該当箇所は新設したcfg(test)内の署名互換テストで、Cu側の固定値と対になる公開のテスト専用鍵。
実行環境から取得した秘密情報ではなく、運用設定からの漏えいはこの指摘から確認されていない。

## Regression Context

新しいテストデータに対する事前の漏えいスキャンを省略したため、自動審査で初めて止まった。
既存運用機能の回帰ではなく、公開テスト値の表現と提出前確認の不足。

## Cause

公開の固定値にテスト用途は記されていたが、一般的な資格情報代入の形になっていた。
Revisorの資格情報検出はテスト関数を特別扱いせず、この値を候補として扱った。

## Fix Requirements

- 値そのものを明確なdummyデータへ変更し、CuとSyの共通署名期待値を再計算する。
- Routerテストの資格も公開dummy値に統一する。
- 本番の資格注入、署名検証、検査ルール、除外設定は変更しない。
- 元の実装を含むPR全体の差分を再検査する。

## Verification

修正後のSy PR全体で漏えい候補0件、Syの署名と保管Routerの既存4テスト、
CuのSy保管adapterの既存2テストで署名一致・改変拒否を確認する。
サービスやlistenerの起動を伴う検証ではない。実行前後にCc claim/releaseを行う。

## Follow-up

修正をcommit後、既存のRevisor #1543とCu #1544へ再審査を依頼する。
Revisorの審査通過は別途通知で確認し、この作業記録だけでマージ可能とはしない。
