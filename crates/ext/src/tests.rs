use super::*;

fn params(ps: &[(&str, Type)]) -> Vec<(String, Type)> {
    ps.iter().map(|(n, t)| (n.to_string(), t.clone())).collect()
}

fn ok(sql: &str, tables: &[&str], ps: &[(&str, Type)]) -> String {
    let t: Vec<String> = tables.iter().map(|s| s.to_string()).collect();
    check_sql(sql, &t, &params(ps), Span::default()).unwrap_or_else(|d| panic!("{d:#?}"))
}

fn ng(sql: &str, tables: &[&str], ps: &[(&str, Type)]) -> String {
    let t: Vec<String> = tables.iter().map(|s| s.to_string()).collect();
    check_sql(sql, &t, &params(ps), Span::default())
        .expect_err("拒否されるはず")
        .iter()
        .map(|d| d.message_ja.clone())
        .collect::<Vec<_>>()
        .join(" / ")
}

#[test]
fn accepts_select_with_declared_tables_and_params() {
    let s = ok(
        "SELECT \"商品ID\", SUM(\"売上金額\") AS \"売上\" FROM \"注文明細\" WHERE \"確定日時\" >= :開始 AND \"状態\" = '確定' GROUP BY \"商品ID\" ORDER BY \"売上\" DESC;",
        &["注文明細"],
        &[("開始", Type::DateTime)],
    );
    assert!(s.contains("\"確定日時\" >= ?1"), "{s}");
    assert!(!s.ends_with(';'));
    // 文字列の中の :x は引数ではない
    let s = ok("SELECT ':not_param' AS a FROM \"商品\"", &["商品"], &[]);
    assert!(s.contains("':not_param'"));
    // CTE は表として参照できる
    ok("WITH t AS (SELECT \"ID\" FROM \"商品\") SELECT \"ID\" FROM t", &["商品"], &[]);
}

#[test]
fn rejects_writes_other_tables_functions_and_bad_params() {
    assert!(ng("DELETE FROM \"商品\"", &["商品"], &[]).contains("SELECT"));
    assert!(ng("SELECT 1; DROP TABLE \"商品\"", &["商品"], &[]).contains("1 文"));
    assert!(ng("SELECT * FROM \"注文\"", &["商品"], &[]).contains("宣言されていません"));
    assert!(ng("SELECT * FROM \"商品\" JOIN \"利用者\" ON 1=1", &["商品"], &[]).contains("利用者"));
    assert!(ng("SELECT load_extension('x') FROM \"商品\"", &["商品"], &[]).contains("load_extension"));
    assert!(ng("SELECT * FROM \"商品\" WHERE \"ID\" = ?", &["商品"], &[]).contains(":名前"));
    assert!(ng("SELECT * FROM \"商品\" WHERE \"ID\" = :x", &["商品"], &[]).contains("宣言されていません"));
    assert!(ng("SELECT * FROM \"商品\"", &["商品"], &[("未使用", Type::Int)]).contains("使われていません"));
    assert!(ng("SELECT * FROM main.\"商品\"", &["商品"], &[]).contains("スキーマ付き"));
    assert!(ng("SELEC * FROM x", &["x"], &[]).contains("解析できません"));
}

#[test]
fn assignment_is_deterministic_and_follows_weights() {
    assert_eq!(assign("e", "u1", &[50, 50]), assign("e", "u1", &[50, 50]));
    let mut counts = [0usize; 2];
    for i in 0..10_000 {
        counts[assign("ボタン", &format!("user-{i}"), &[20, 80])] += 1;
    }
    assert!((1700..2300).contains(&counts[0]), "{counts:?}");
    // TypeScript ランタイムと同じ値になることの基準値(runtime/backend/experiments.ts のテストでも使う)
    assert_eq!(assign("購入ボタン文言", "anon-0001", &[50, 50]), 1);
}
