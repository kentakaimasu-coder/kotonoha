# ADR-0125 分析(読み取り専用 SQL)
状態: 提案(実装) / 日付: 2026-11-07 / 関連: NR-07

## 構文
```kotonoha
分析 商品別売上() -> 一覧<記録{商品ID: 識別子, 売上: 金額<JPY>, 件数: 整数}>:
    対象 = [注文]
    SQL = "analytics/sales_by_product.sql"
```
```sql
SELECT "商品ID" AS "商品ID", SUM("金額") AS "売上", COUNT(*) AS "件数"
FROM "注文" GROUP BY "商品ID" ORDER BY "売上" DESC
```
画面では `商品別売上を表示`(引数なしの分析だけ)。API は `GET /api/analyses/{名前}?引数=値`。

## 検査(コンパイル時)
- SQL を構文解析し(sqlparser、SQLite 方言)、**SELECT 1 文だけ**(`SELECT … INTO` も不可)
- 参照できる表は `対象` に宣言したデータ(と WITH 句の名前)だけ。スキーマ付きの名前は不可
- 関数は許可リストのみ: count, sum, avg, min, max, total, coalesce, ifnull, nullif, abs, round, lower, upper, length, substr, substring, trim, ltrim, rtrim, replace, instr
- 引数は `:名前` だけ(`?`・`$`・`@` は不可)。宣言した引数をすべて使う。`?N` に置き換えてバインドする(文字列連結はしない: SEC-04)
- 戻り値は `一覧<記録{…}>` で、列はワイヤ形式のスカラー型
- 本人限定(C-OWNER)のデータは対象にできない(TGT001。行レベル認可と任意 SQL の関係が未定義)

## 実行時(多重防御)
- SQLite: `PRAGMA query_only = ON` の間だけ実行。PostgreSQL: `BEGIN TRANSACTION READ ONLY` + `statement_timeout`(`KOTONOHA_ANALYSIS_TIMEOUT_MS`、既定 10 秒)
- 結果は 10,000 行まで(超えたら 422 `too_many_rows`)。列の有無と型を宣言どおりか検査する
- アクセス: 対象の全データが「誰でも閲覧可」なら公開、それ以外は各データの `read` Capability が必要
- 未知のクエリ引数は 400

## 未決定
- 方言差(SQLite と PostgreSQL の関数・日時の扱い)の吸収。現状は両方で動く SQL を書く(識別子は二重引用符で囲む)
- 大規模集計のための非同期実行・キャッシュ
