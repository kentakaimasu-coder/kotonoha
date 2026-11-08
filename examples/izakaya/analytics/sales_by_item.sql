-- 品名ごとの数量と売上(多い順)
SELECT "品名" AS "品名", SUM("数量") AS "数量", SUM("金額") AS "売上"
FROM "注文"
GROUP BY "品名"
ORDER BY "売上" DESC, "品名" ASC
