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
- web: `.ts` のみ(JSX 不可。SSR を Node.js の型除去で動かすため `React.createElement` で書く)。部品名と同じ名前の名前付きエクスポートで、props は宣言の引数名(`{ 評価: number }`。値はワイヤ形式)
- ios: 部品名と同じ名前の `struct 名前: View`。引数名のプロパティを持つ(整数・金額は Int64)
- android: 部品名と同じ名前の `@Composable fun `名前`(`引数`: Long)`(日本語名は `` ` `` で囲む)。パッケージは `app.kotonoha.generated`
- web の部品は `ExtBoundary` で囲み、描画に失敗しても画面全体を壊さない
- 実装の無いターゲットでその部品を使うと TGT001
- 実装ファイルはプロジェクト内の相対パスのみ(`..` 不可)。内容のハッシュを VAG に記録し、ロック(source_hash)の対象にする(差し替えると LOCK001)
- 部品はブラウザ/端末で動く表示用のコードで、業務ロジックや認可を置かない(backend の API 以外に権限を持たない)。Web の CSP(`script-src 'self'`・`connect-src 'self'`)はそのまま適用される

## 実装(2026-11-07)
- コンパイラ: `crates/ext`(実装ファイルの読込・拡張子・パス検査・SHA-256)。VAG の `extensions.components`。実装ファイルの内容はグラフハッシュに入り、差し替えるとロックが一致しない
- 生成: web は `src/ext/…` に複製して import、iOS は `Sources/Ext/…`、Android は `…/generated/ext/…`
- 例: `examples/lab`(星評価・数字評価)。web は E2E、iOS/Android は CI の xcodebuild / gradle で確認
