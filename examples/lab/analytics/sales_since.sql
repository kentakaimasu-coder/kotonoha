-- 指定した日時以降の売上合計
SELECT COALESCE(SUM("金額"), 0) AS "売上", COUNT(*) AS "件数"
FROM "注文"
WHERE "確定日時" >= :開始
