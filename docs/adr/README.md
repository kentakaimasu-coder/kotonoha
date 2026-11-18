# ADR 一覧

状態の定義: **未決定**(選択肢と推奨のみ。実装しない/ブロッカー)、**提案**(推奨案で仮実装済み。承認待ち)、**承認**、**却下**。
承認者が状態を「承認」に変更するまで、どの ADR も確定仕様ではない。

| ADR | 題目 | 状態 | ブロックしているもの |
|---|---|---|---|
| [ADR-001](ADR-001-license.md) | ライセンス | 未決定 | 公開配布 |
| [ADR-002](ADR-002-auth-provider.md) | 認証プロバイダー | 未決定(暫定: HS256 + JWKS + Web OIDC) | IdP の選定・モバイル OIDC |
| [ADR-003](ADR-003-database.md) | DB 方針 | 提案(SQLite / PostgreSQL 実装済み) | |
| [ADR-004](ADR-004-ui-framework.md) | UI 生成フレームワーク | 未決定(暫定: React SSR + esbuild) | |
| [ADR-005](ADR-005-mobile-min-os.md) | モバイル最低 OS | 未決定 | FR-013/014(P1) |
| [ADR-006](ADR-006-ai-provider.md) | AI プロバイダーと送信方針 | 未決定(手元のモデルは ADR-0127) | 外部 AI への送信 |
| [ADR-007](ADR-007-approval-signing.md) | 承認署名基盤 | 未決定 | 承認入りの `--locked` ビルド |
| [ADR-008](ADR-008-build-compat.md) | ビルド互換性ポリシー | 未決定(暫定運用あり) | コンパイラ更新時のロック再利用 |
| [ADR-0101](ADR-0101-lexical.md) | 字句規則(語の最長一致・NFC 拒否・コメント) | 提案 | |
| [ADR-0102](ADR-0102-strict-syntax.md) | 厳密層の追加構文(型/列挙/分岐/反復/否定/自動包装) | 提案 | |
| [ADR-0103](ADR-0103-exit-codes.md) | 終了コードの割当補完 | 提案 | |
| [ADR-0104](ADR-0104-data-adapter.md) | データアダプタ宣言 | 提案 | |
| [ADR-0105](ADR-0105-capabilities.md) | Capability 動詞・効果宣言・更新は読取を含まない | 提案 | |
| [ADR-0106](ADR-0106-numeric-precision.md) | 数値精度・金額演算・日時 | 未決定(暫定: 未決定演算は拒否) | 金額除算、小数、日時リテラル |
| [ADR-0107](ADR-0107-fmt.md) | `koto fmt` はテキスト正規化のみ | 提案 | |
| [ADR-0108](ADR-0108-ranking-decisions.md) | 売れ筋の業務決定と時刻境界 | 提案(業務値は未承認) | AT-07 の正式採用 |
| [ADR-0109](ADR-0109-lock-targets.md) | ロックの `targets` 配列 | 提案 | |
| [ADR-0110](ADR-0110-physical-mapping.md) | 物理マッピングと SQLite 格納形式 | 提案 | |
| [ADR-0111](ADR-0111-intent-patterns.md) | 意図文型・画面の入力・約束文型 | 提案 | |
| [ADR-0112](ADR-0112-flexible-mode-approval.md) | 曖昧性モードと承認の適用範囲 | 提案 | |
| [ADR-0113](ADR-0113-source-hash.md) | ロックのハッシュ範囲 | 提案 | |
| [ADR-0114](ADR-0114-crate-layout.md) | クレート構成の差分 | 提案 | |
| [ADR-0115](ADR-0115-form-pattern.md) | フォーム文型 P-FORM | 提案 | |
| [ADR-0116](ADR-0116-display-fields.md) | 一覧の表示列 | 提案 | |
| [ADR-0117](ADR-0117-wire-format.md) | ワイヤ形式と共通 API 契約 | 提案 | |
| [ADR-0118](ADR-0118-capability-granularity.md) | 動作単位の許可・行レベル認可 | 提案(実装済み) | |
| [ADR-0119](ADR-0119-principal-builtin.md) | 組込み値 `本人` | 提案 | |
| [ADR-0120](ADR-0120-security-and-obscurity.md) | セキュリティ強化と「知られていないこと」の扱い | 提案(実装) | |
| [ADR-0121](ADR-0121-japanese-usability.md) | 日本人の利用者のための使いやすさ | 提案(実装) | |
| [ADR-0122](ADR-0122-experiments.md) | 実験(A/B テスト) | 提案(実装) | |
| [ADR-0123](ADR-0123-extension-components.md) | 拡張部品 | 提案(実装) | |
| [ADR-0124](ADR-0124-functions-and-externals.md) | 関数・組込み関数・外部関数(TypeScript / Python) | 提案(実装) | 外部コードの隔離方式 |
| [ADR-0125](ADR-0125-analysis-sql.md) | 分析(読み取り専用 SQL) | 提案(実装) | SQL 方言差の吸収 |
| [ADR-0126](ADR-0126-roles.md) | 役割(ロール)と全件閲覧の許可 | 提案(実装) | 役割の継承・テナント |
| [ADR-0139](ADR-0139-no-terminal-and-publish.md) | 黒い画面を使わずに作る(Studio から始める)・公開の準備・個人情報の警告・操作の記録 | 提案(実装) | 置き場所への公開の代行 |
| [ADR-0142](ADR-0142-built-in-login.md) | 公開に外部のログインのサービスを要さない「アプリの中のログイン」 | 決定 | 案 2 を実装(scrypt・招待のリンク・失敗の制限・印の取り消し) |
| [ADR-0141](ADR-0141-login-only-read.md) | 「…はログインした人だけ閲覧可」と役割の無い人 | 決定 | 案 2(ログインしていれば役割に関わらず読める)を実装 |
| [ADR-0140](ADR-0140-general-user.md) | 役割を書かない人(一般の利用者)の権限をそろえる | 決定(実装) | 開発・確かめると本番の違い |
| [ADR-0138](ADR-0138-round6-fixes.md) | 第 6 回評価の修正(名前の変更の規則・役割だけの画面・Studio の指示の追加) | 提案(実装) | モバイルの役割だけの画面 |
| [ADR-0137](ADR-0137-round5-fixes.md) | 第 5 回評価の修正(名前の変更・まとめて消す・空でもよい項目・条件付きボタン) | 提案(実装) | モバイルの条件付きボタン |
| [ADR-0136](ADR-0136-computed-fields.md) | 計算項目と、集計・ログインした人との結びつけ | 提案(実装) | |
| [ADR-0135](ADR-0135-studio.md) | 画面で作る KOTONOHA Studio(日本語の指示・機種ごとの見た目の確認) | 提案(実装) | ネイティブ版の画面のシミュレータ表示 |
| [ADR-0134](ADR-0134-named-arguments-and-scenario-labels.md) | 名前付き引数と、「確かめる」で作った物に名前を付ける | 提案(実装) | 画面の文の名前付き引数 |
| [ADR-0133](ADR-0133-analysis-from-aggregate-sentence.md) | 分析を集計の文で書く(SQL なし) | 提案(実装) | 日ごと・月ごと、今日・本人の絞り込み |
| [ADR-0132](ADR-0132-role-named-actions-and-state-checks.md) | 役割で名指しした動作はその役割だけ・状態の順番の確かめ方 | 提案(実装) | 名指しの無い動作の広すぎる許可の警告 |
| [ADR-0131](ADR-0131-default-design-and-theme.md) | 生成する Web 画面の既定デザインとテーマ色 | 提案(実装) | モバイルへのテーマ色、部品ごとの見た目 |
| [ADR-0130](ADR-0130-plain-actions-and-verify.md) | 専門用語を覚えずに書ける動作と、日本語で確かめる「確かめる」 | 提案(実装) | 名前付き引数 |
| [ADR-0129](ADR-0129-data-rules-dates-forms.md) | 重複不可・状態の順番・日付の計算・応答の時刻帯・フォームの改善 | 提案(実装) | 取消など任意の値への遷移 |
| [ADR-0128](ADR-0128-list-views-and-aggregation.md) | 一覧の並べ替え・絞り込みと、日本語の集計 | 提案(実装) | |
| [ADR-0127](ADR-0127-local-model-and-learning.md) | 手元のオープンモデルと、承認から育つ解釈(学習記録) | 提案(実装) | 実モデルでの検証 |
