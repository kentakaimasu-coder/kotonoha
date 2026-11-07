SELECT "会場" AS "会場", COUNT(*) AS "イベント数", SUM("予約数") AS "予約数", SUM("定員") AS "定員"
FROM "イベント"
GROUP BY "会場"
ORDER BY "予約数" DESC, "会場" ASC
