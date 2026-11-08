# ターゲットの対応表

状態: 提案 / 最終更新: 2026-11-08 / 上位文書: 技術仕様書 §11 / 実装: `crates/codegen/src/lib.rs`(`ALL_FEATURES`)

`koto build` は VAG の各ノードから (種別, 機能) を取り出し、ターゲットが対応していなければ TGT001 で止める(不変ルール 5)。
現在は 4 ターゲット(backend / web / ios / android)が同じ機能集合に対応する。型レベルの未対応(例: iOS のフォームで使えない型)は各生成器が TGT001 を返す。

| 種別 | 機能 | 文型・宣言 |
|---|---|---|
| screen | screen.page | `画面` |
| display | ui.list | P-LIST(並べ替え・絞り込みを含む) |
| display | ui.detail | P-FIELDS |
| display | ui.button | P-BUTTON |
| display | ui.form | P-FORM |
| display | ui.component | P-COMPONENT |
| query | query.ranking | P-KNOWLEDGE |
| query | query.analysis | P-ANALYSIS |
| query | query.aggregate | P-AGGREGATE |
| navigation | nav.select | P-NAV |
| action | action.strict | `動作` |
| validation | policy.public_read / auth_required / owner_read | C-PUBLIC / C-AUTH / C-OWNER |
| validation | policy.unique / policy.sequence | C-UNIQUE / C-SEQUENCE(backend で強制。クライアントは結果のメッセージを表示するだけ) |

クライアント側の差: フォームで「他のデータから選ぶ」入力と複数行の入力は Web だけ(iOS / Android は ID の入力欄・1 行の入力欄)。
