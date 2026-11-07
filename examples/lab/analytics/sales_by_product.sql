-- 商品ごとの売上(多い順)
SELECT "商品ID" AS "商品ID", SUM("金額") AS "売上", COUNT(*) AS "件数"
FROM "注文"
GROUP BY "商品ID"
ORDER BY "売上" DESC, "商品ID" ASC
