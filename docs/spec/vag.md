# Verified Application Graph (VAG) v1

状態: 提案 / 最終更新: 2026-11-02 / 上位文書: 技術仕様書 §6 / スキーマ: [schemas/vag.schema.json](../../schemas/vag.schema.json) / 実装: `crates/graph` / 見本: [examples/shop/expected/vag.json](../../examples/shop/expected/vag.json)

## トップレベル
| キー | 内容 |
|---|---|
| schema_version | `"1.0"` |
| application | `{name}` |
| schema | `enums` `aliases` `data` `adapters`(schema_hash の対象) |
| knowledge | 確定済みの知識(knowledge_hash の対象) |
| policy | `public_read` / `auth_required`(約束から導出) |
| nodes | 下記。`id` 順にソート |
| edges | `{from, to, kind}`。kind = contains / navigates / invokes。(from,to,kind) 順にソート |
| status | `validated`(全検査通過)/ `draft`(参考出力。ロック・ビルド不可) |

## ノード
共通: `id, kind, title, inputs[], outputs[], effects[], capabilities[], knowledge_refs[], source_span{file,line,column}`(技術仕様書 §5 の項目)。

| kind | id | 追加項目 |
|---|---|---|
| screen | `screen.{名前}` | mode, inputs = 画面の入力データ |
| display | `{意図ID}` | sentence, resolution, detail.ui = list / detail / button |
| query | `{意図ID}` | sentence, resolution, detail.knowledge / limit / limit_override |
| navigation | `{意図ID}` | sentence, resolution, detail.event = tap, detail.to |
| action | `action.{名前}` | params[], auth = required |
| validation | `contract#{ハッシュ}` | sentence, detail.policy / data |

`resolution` は `{via: "pattern"}` または `{via: "approved", canonical, approval_hash}`。

## 正規化とハッシュ
- 正規 JSON: オブジェクトのキーは UTF-8 バイト順、空白なし、数値は整数のみ、非 ASCII はエスケープしない。
- `graph_hash` = SHA-256(正規 JSON(VAG 全体))。`knowledge_hash` / `schema_hash` はそれぞれの節。
- VAG はソースと承認記録だけの関数。時刻・環境変数・ネットワーク・乱数に依存しない(TC-009)。
- source_span を含むため、コメントの追加でも行がずれれば graph_hash は変わる(source_hash は常に変わる)。

## 参照整合性
ノード ID の重複、辺の端点の欠落は INT001 で `draft` に落とす。
