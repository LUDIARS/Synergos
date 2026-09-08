# Cu Server向けの実体保管

2026-09-08 neco指示によるCu Server/Client構成とSynergos保存取得の接続整理。
Cu Serverが版・提出・許可・共有ノードの利用方針、Crが本人・所属・組織役割を所有する。
Synergosはopaque objectの転送と保管の実行を担当し、制作履歴や承認を作らない。

## [id: SPEC-SY-opaque-custody] 受付・保管・受領証

既存synergos-controlへsynergos-custodyのRouterを明示設定で追加する。別listenerを起動しない。
操作ID・opaque object ID・共通SHA-256・サイズ・期限・content typeを受付時に固定する。
同一操作の再送は同じ実体を指す。内容の差替え、別実体へのoperation ID流用、回収後の再受付を拒否する。
管理者APIの対象はCu Server。Clientには用途・期限・実体を限定したHMAC署名URLだけを渡す。
PUTにはIf-None-Match:*と申告SHA-256を要求し、ストリームの実サイズと実SHA-256を検証してから公開する。
公開済み実体へのPUTは412、内容不一致は409。途中失敗の一時ファイルは回収する。

receiptは検証済み実体に対してのみ発行する。operationId/opaqueObjectId/verifiedDigests/byteLength/durability/locations/expiresAtを含める。
JSONキーを再帰的に辞書順で整形し、HMAC-SHA256を計算する。Cuの署名consumerと共通テスト値で照合する。
期限切れ受付に新たなreceiptは発行しない。保管済み実体の取得期限は短期URLで別に制御する。

実装対応: lib.rsがRouterを組み込み、contract.rsが受付と取得要求を検証、handlers.rsがHTTP権限境界を担う。
signature.rsのcanonical/sign/verifyはCuと共有する署名表現、error.rsは失敗を公開エラーへ変換する。
JSONの再帰整形とエラー列挙の変換はそれぞれ一つの契約を担うため、静的結合数だけを下げる目的では分割しない。

## [id: SPEC-SY-custody-durability] 耐久性と寿命

本段階のdurableは、明示した永続ボリューム上の単一保管ノードへの保存を意味する。
ファイルsync_all、同一ディレクトリへの排他的公開、Unixのディレクトリfsync / WindowsのMoveFileExW WRITE_THROUGHを行う。
receipt発行時にも実ファイルを再検証しsync_all成功を要求する。キャッシュ保存や通信受信のみをdurableと呼ばない。
OSファイルロックをStoreの寿命に結び付け、同じ保管rootを別プロセスから開くことを拒否する。クラッシュでロックは解放される。
起動時は保管rootの公開試験と前回の一時ファイル回収を行い、設定/権限/保存能力が不足すれば起動を失敗させる。
これは複数拠点への複製、媒体障害からの復旧、停電試験の完了を保証しない。複製数を要求する保全方針は別のreceipt capabilityとして拡張する。

実装対応: config.rsが有効化前提、store.rsが実体の保管寿命、durable_file.rsが排他的公開と一時ファイルの解放を担う。

## [id: SPEC-SY-custody-acquisition] 取得・中断・回収

Cuが全対象の取得権限を確認してから、最大900秒のGET URLを要求する。名前は安全なASCII basenameに限定する。
取得時に実体を検証し、Cu Clientも受信サイズとSHA-256を検証して作業中ファイルを保護する。
ブラウザーOriginは設定したCu Clientだけ許可する。URL署名はHTTPメソッド・実体キー・期限・取得名へ結び付ける。
中断後は同じ受付から再PUTできる。バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送。
Cuが参照のない未確定実体だけをreclaimする。Syは永続tombstoneを先に記録して削除し、古いPUTで実体を復活させない。
本機能の保管rootはSyの旧history GC対象にしない。Cuの業務保持参照をSy独自ポリシーで無視しない。

## 構成

| 環境変数 | 内容 |
|---|---|
| SYNERGOS_CUSTODY_ENABLED | 1で有効、未設定/0では経路を公開しない |
| SYNERGOS_CUSTODY_ROOT | 永続保管専用の絶対パス |
| SYNERGOS_CUSTODY_PUBLIC_URL | Excubitor catalog/ProcessMapで解決した既存ControlのHTTP(S) origin |
| SYNERGOS_CUSTODY_NODE_ID | 受領証へ記録する物理ノードUUID |
| SYNERGOS_CUSTODY_TOKEN | Cu Serverだけへ渡す32文字以上の資格 |
| SYNERGOS_CUSTODY_RECEIPT_SECRET | Cuと共有する32文字以上の署名鍵 |
| SYNERGOS_CUSTODY_MAX_OBJECT_BYTES | 明示した1実体の上限バイト数 |
| SYNERGOS_CUSTODY_CLIENT_ORIGINS | 許可するCu Client origin、カンマ区切り |

Cu側のSYNERGOS_STORAGE_URL/TOKEN/RECEIPT_SECRETと対応させる。秘密情報をDB・ログ・URL・説明書へ保存しない。
設定はInfisical/Excubitorの注入を使用する。本変更で本体サービスの設定変更や起動は行わない。

## HTTP契約

管理Bearer必須: GET /v1/custody/health、POST admissions / verify / head / download / reclaim。
admissionsはoperationId, opaqueObjectId, contentHash, byteLength, expiresAt, contentTypeを受け、uploadUrlとrequiredHeadersを返す。
verify/head/reclaimはopaqueObjectIdを受ける。verifyはreceipt/signature、headはcontentLength/contentHash/contentTypeまたはnull、reclaimはreclaimed:true。
downloadはopaqueObjectId, expiresIn, downloadNameを受けdownloadUrlを返す。
Client転送はPUT/GET /v1/custody/transfer/:key?expires=...&signature=...&name=...。保管資格をURLへ含めない。

## 検証

Router内のテストで未認証拒否、偽造/用途違いURL拒否、誤内容拒否、重複PUT、再送、再オープン後保持、取消後再送拒否を確認する。
実績: cargo check -p synergos-custody -p synergos-control --offline成功。custody.rsの3件とsignature.rsの共通署名値1件が成功。
Storeの再オープンは同一プロセス内であり、実サービスの再起動テストと区別する。
Anatomiaにはopaque-custodyドメインと仕様参照を記録した。ただし現行AnatomiaのANALYZED_SOURCE_EXTSはRustを含まず、
verifyの未知拡張子はC++として解析される。今回のRust diffでは孤児20件・結合2箇所と表示されたため、
Rustの意味に沿った仕様リンク/結合検証が完了したとは扱わない。CuのTypeScript側verifyは全ゲート成功。
本体での起動とCu/Cr/Sy接続確認はCc claimの下、Excubitor経由で別途実施する。
参照: [Rust Fileのロック寿命](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)、
[Windowsの公開操作](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)。
