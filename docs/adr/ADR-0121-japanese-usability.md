# ADR-0121 日本人の利用者のための使いやすさ
状態: 提案(実装) / 日付: 2026-11-07 / 関連: NR-01、技術仕様 §3.2
- 全角記号は自動同一視しない(仕様維持)。`koto fmt --zenkaku` で明示的に半角化できるようにする(文字列リテラル・コメントの中は変えない)
- 診断に行の抜粋と `^` の位置表示。`koto explain <コード>`
- 生成フォーム: IME 変換中(`isComposing`)の Enter で送信しない。数値欄は NFKC 正規化で全角数字・全角マイナス(−/ー/－)を受け付ける
- 生成モバイルアプリの既定ロケールを日本語にする(iOS `CFBundleDevelopmentRegion = ja`、Android `values/strings.xml` を日本語で生成)
