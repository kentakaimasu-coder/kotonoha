# エラーコード一覧

状態: 提案(仕様書 §15 + 追加コード) / 最終更新: 2026-11-02 / 実装: `crates/diag`

全診断は `code, severity(error|warning|info), message_ja, file, span{line,column,start,end}, hints[]` を持つ(`--json` で出力)。

| コード | 出所 | 意味 | 終了コード | 主な発生箇所 |
|---|---|---|---|---|
| SYN001 | 仕様 | 不正字句・インデント・構文 | 2 | lexer, parser |
| SYN002 | **追加** | ソースが NFC でない(fmt --check の未整形も) | 2 | lexer, `koto fmt --check` |
| TYP001 | 仕様 | 型不一致(暗黙変換・通貨違い・未決定の演算を含む) | 2 | types, knowledge, intent |
| TYP002 | **追加** | 代入規則違反(引数への再代入、取引外のエンティティ更新、ID 更新、入れ子取引) | 2 | types |
| TYP003 | **追加** | 戻り値の欠落 | 2 | types |
| REF001 | 仕様 | 未定義参照(型・データ・フィールド・呼出先・画面・アプリ宣言欠落) | 2 | types, knowledge, intent |
| REF002 | **追加** | 重複定義・名前の衝突 | 2 | types, knowledge |
| AMB001 | 仕様 | 未解決意図・未確定の業務決定(質問をヒントに付ける) | 3 | intent, knowledge |
| AMB002 | 仕様 | 複数解釈(別名の衝突・矛盾する約束) | 3 | knowledge, validator |
| APP001 | 仕様 | 未承認・承認記録の不正(改ざん・成立しない解釈・真正性未検証での本番ビルド) | 3 | lock, intent, driver |
| LOCK001 | 仕様 | ロック不整合(LOCK_STALE)・ロック欠落 | 5 | lock, driver |
| SEC001 | 仕様 | 宣言外の効果 | 4 | validator |
| SEC002 | 仕様 | Capability 不足(未宣言 = デフォルト拒否) | 4 | validator |
| TGT001 | 仕様 | ターゲット未対応(機能・時刻帯) | 6 | codegen, knowledge |
| GEN001 | 仕様 | 生成失敗(SQL に変換できない式、未決定の格納形式) | 7 | codegen |
| INT001 | **追加** | 内部障害・入出力障害・未実装コマンド | 7 | driver, cli |

`koto test` のテスト失敗は終了コード 1(ADR-0103 追補)。

## 終了コードの決め方
エラーが複数あるときは**最も早い段階**の終了コードを返す(2 < 3 < 4 < 5 < 6 < 7)。警告だけなら 0。
- APP001 → 3: 「意味がまだ確定していない」側に分類(ADR-0103)。
- REF001/REF002 → 2: 構文/型と同じ段階。
