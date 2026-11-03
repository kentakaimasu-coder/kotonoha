# コンパイラ構成

状態: 実装中 / 最終更新: 2026-11-02 / 上位文書: 技術仕様書 §11, §12, §14, §17

## 1. パイプライン
```
UTF-8 ソース(複数ファイル、パス順)
  │ lexer      : 行分割・NFC/インデント/禁止文字検査・トークン化
  │ parser     : Surface AST(意図層は生テキスト)
  ▼
  │ types      : 宣言収集(Env)・型式の解決・アダプタ規則・厳密層の型検査 → Facts(呼出・更新)
  │ knowledge  : 知識 → RankingSpec、名前/別名レジストリ
  │ lock       : 承認記録の改ざん検査
  │ intent     : 有限文型の照合・承認済み解釈の適用・画面の入力推定 → IntentModel / Unresolved
  │ validator  : 効果/Capability(推移閉包・デフォルト拒否)・約束の整合
  │ graph      : VAG 構築・参照整合性・正規 JSON・ハッシュ
  ▼
  │ lock       : (--locked)ロック比較・承認の真正性検証(ADR-007 未決定のため承認入りは拒否)
  │ codegen    : supports() による対応検査(TGT001)・対応済み部分の生成(SQL)
  ▼
生成物(build/<target>/)
```
各段は前段の結果を読み取り専用で受け取り、診断(位置付き)を返す。全段が純粋関数で、AI・ネットワーク・時刻を使わない(時刻は承認記録の `approved_at` のみ)。

## 2. クレート
| クレート | 役割 | 技術仕様書 §17 との対応 |
|---|---|---|
| diag | Span・診断・エラーコード・終了コード | (新設) |
| ast | Surface AST | ast |
| lexer | 字句 | lexer |
| parser | 構文・正規形プリンタ | parser |
| types | 型・宣言環境・厳密層検査・効果語彙 | types |
| knowledge | 知識レジストリ | knowledge |
| intent | 意図文型・承認適用 | intent |
| resolver | 候補生成アダプタ(offline/fixture)・候補スキーマ検査 | resolver, adapters/ai |
| validator | 効果/Capability | validator |
| graph | VAG | graph |
| lock | 承認記録・ロック・真正性検証の差し替え点 | approval + lock(ADR-0114) |
| codegen | ターゲット契約・SQL 低水準化 | codegen, targets/* |
| driver | パイプライン統合・プロジェクト読込 | (新設) |
| cli | `koto` | cli |

生成器は `crates/codegen/src/{appir,backend,web,ios,android,ts,sql}.rs`、生成物に同梱するランタイムは `runtime/{backend,web,ios,android}/`。`stdlib/*` は未作成。

## 3. CLI 契約(実装状況)
| コマンド | 状況 |
|---|---|
| `koto init` | 実装 |
| `koto fmt [--check]` | 実装(テキスト正規化のみ) |
| `koto check` | 実装 |
| `koto resolve [--adapter offline\|fixture]` | 実装(AI プロバイダなし) |
| `koto approve <id> <n> --approver …` / `--write-lock` | 実装(署名なし) |
| `koto graph [--draft] [--canonical]` | 実装 |
| `koto build --target … [--locked] [--out dir]` | 実装(4 ターゲット) |
| `koto test [--keep]` | 実装: backend を生成し、API 契約から導いた契約テストと業務規則の雛形(todo)を実行。失敗は終了コード 1 |
| `koto impact <名前> \| --base <vag.json>` | 実装: 影響を受けるノードの列挙(AT-08) |
| `koto doctor` | 実装 |
| `koto ast <file>` | 開発用 |
共通: `-C <dir>`、`--json`(機械可読診断)。

## 4. ターゲット機能名(`supports(node_kind, feature)`)
現在の機能集合(screen.page / ui.list / ui.detail / ui.button / ui.form / query.ranking / nav.select / action.strict / policy.*)は**全ターゲットが対応**する。backend は UI ノードを「データ API の提供」として、クライアントは action を「API 呼出」として扱う。表にない機能は TGT001。型レベルの未対応(小数など)は各生成器が TGT001 を返す。

| ターゲット | 検証状況 |
|---|---|
| backend | 生成 → 起動 → API 結合テスト(tests/e2e/backend.test.ts) |
| web | 生成 → npm install → ビルド → 起動 → Playwright E2E(tests/e2e/web.test.ts) |
| ios | 生成 + 構文検査(tree-sitter、tools/mobile-syntax)。コンパイルは CI ジョブ `ios-build`(未実行) |
| android | 生成 + 構文検査 + 非 UI 層の kotlinc コンパイルと実 API 応答の解析(tools/verify_android_jvm.sh)。Compose 画面のビルドは CI `android-build`(未実行) |
構成の全体は [docs/architecture.md](../architecture.md)。
