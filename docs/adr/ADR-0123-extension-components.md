# ADR-0123 拡張部品(手書きの UI 部品)
状態: 提案(実装) / 日付: 2026-11-07 / 関連: NR-04, FR-022

## 構文
```kotonoha
部品 星評価(値: 整数):
    web = "ext/StarRating.ts"
    ios = "ext/StarRating.swift"         # 任意
    android = "ext/StarRating.kt"        # 任意
```
画面では `星評価(商品.在庫)を表示`(引数は画面の引数式と同じ規則)。

## 規約
- web: 既定エクスポートが React 部品で、props は宣言の引数名(`{ 値: number }`)
- ios: `struct Ext_<名前>: View` で引数名のプロパティを持つ(整数は Int64)
- android: `@Composable fun Ext_<ASCII化した名前>(…)`
- 実装の無いターゲットでその部品を使うと TGT001
- 実装ファイルはプロジェクト内の相対パスのみ(`..` 不可)。内容のハッシュを VAG に記録し、ロック(source_hash)の対象にする(差し替えると LOCK001)
- 部品はブラウザ/端末で動く表示用のコードで、業務ロジックや認可を置かない(backend の API 以外に権限を持たない)。Web の CSP(`script-src 'self'`・`connect-src 'self'`)はそのまま適用される
