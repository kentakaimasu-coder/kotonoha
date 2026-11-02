SELECT "商品ID", SUM("売上金額") AS "売上金額"
FROM "注文明細"
WHERE ("状態" = ?1)
  AND "確定日時" >= ?2
  AND "確定日時" < ?3
GROUP BY "商品ID"
ORDER BY "売上金額" DESC, "商品ID" ASC
LIMIT ?4;
