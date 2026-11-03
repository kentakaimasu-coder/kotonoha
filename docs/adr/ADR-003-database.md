# ADR-003 DB 方針
状態: 提案(両方言を実装済み) / 日付: 2026-11-02 / 更新: 2026-11-03
## 推奨(要件 §11 の案どおり)
MVP は SQLite、本番は PostgreSQL。生成 SQL は 2 方言(プレースホルダ `?n` / `$n`)を出す。
## 現状(2026-11-03)
- backend は `KOTONOHA_DB` で方言を選ぶ: SQLite(node:sqlite、既定)/ PostgreSQL(`postgres://…`、任意依存 `pg`)
- 生成コードは非同期で、DB 層(`runtime/backend/db.ts`)が方言を吸収する。SQL は `?` で書き、PostgreSQL では `$n` に変換する
- 取引: SQLite は `BEGIN IMMEDIATE` + 接続の排他ロック、PostgreSQL は `SERIALIZABLE` + 直列化失敗の再試行。同時注文 20 件・在庫 10 で成功がちょうど 10 件になることを両方言で確認
- 整数・金額・日時・真偽は両方言とも 64 ビット整数列(SQLite INTEGER / PostgreSQL BIGINT)。PostgreSQL の識別子は 63 バイトまで(超えると起動時に拒否)
- 差分マイグレーションは両方言で動作(ADR-0110 の格納形式)
