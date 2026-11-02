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

未作成(未着手のため): `adapters/{db,auth}`, `targets/{web,backend,ios,android}` の生成器本体, `runtime/*`, `stdlib/*`。

## 3. CLI 契約(実装状況)
| コマンド | 状況 |
|---|---|
| `koto init` | 実装 |
| `koto fmt [--check]` | 実装(テキスト正規化のみ) |
| `koto check` | 実装 |
| `koto resolve [--adapter offline\|fixture]` | 実装(AI プロバイダなし) |
| `koto approve <id> <n> --approver …` / `--write-lock` | 実装(署名なし) |
| `koto graph [--draft] [--canonical]` | 実装 |
| `koto build --target … [--locked]` | 検証と対応検査まで実装。生成は backend の SQL のみ |
| `koto test` | **未実装**(終了コード 7) |
| `koto doctor` | 実装 |
| `koto ast <file>` | 開発用 |
共通: `-C <dir>`、`--json`(機械可読診断)。

## 4. ターゲット機能名(`supports(node_kind, feature)`)
| node_kind | feature | web | backend | ios | android |
|---|---|---|---|---|---|
| screen | screen.page | – | – | – | – |
| display | ui.list / ui.detail / ui.button | – | – | – | – |
| query | query.ranking | – | ✔(SQL のみ) | – | – |
| navigation | nav.select | – | – | – | – |
| action | action.strict | – | – | – | – |
| validation | policy.public_read / policy.auth_required | – | – | – | – |
未対応が 1 つでもあれば TGT001 で停止する(黙って欠落させない)。backend は画面ノードを生成しないが、現在は「画面を含むプロジェクト全体の生成」を対象とするため TGT001 になる(ターゲット別の対象ノード範囲は M2 で決める: 残課題 R-05)。
