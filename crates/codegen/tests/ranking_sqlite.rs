//! AT-07 / TC-001 / TC-002: 売れ筋(直近30日・売上金額・20件)の SQL と期待結果。
//!
//! フィクスチャの業務仮定(状態=確定・取消除外・税込JPY・Asia/Tokyo・区間 [基準-30日, 基準))は
//! 技術仕様書 §8 の「テストフィクスチャ用の仮定」であり、正式採用には承認が必要(ADR-0108)。

use kotonoha_codegen::sql::{lower_ranking, sqlite_ddl, window_ms, Dialect, SqlParam};
use kotonoha_diag::FileId;
use kotonoha_knowledge::{build_registry, Knowledge, RankingSpec};
use kotonoha_types::{collect_env, Env};
use rusqlite::{params_from_iter, Connection};

const RANKING: &str = include_str!("../../../examples/ranking/src/app.koto");
const GOLDEN_SQLITE: &str = include_str!("../../../examples/ranking/expected/ranking.sqlite.sql");
const GOLDEN_PG: &str = include_str!("../../../examples/ranking/expected/ranking.postgres.sql");

/// 2026-11-03T00:00:00+09:00
const BASE_MS: i64 = 1_793_631_600_000;
const DAY: i64 = 86_400_000;

fn load() -> (Env, RankingSpec) {
    let (f, pd) = kotonoha_parser::parse_file(RANKING, FileId(0));
    assert!(pd.is_empty());
    let (env, ed) = collect_env(&[&f]);
    assert!(ed.is_empty(), "{ed:#?}");
    let (reg, kd) = build_registry(&[&f], &env);
    assert!(kd.is_empty(), "{kd:#?}");
    let Knowledge::Ranking(r) = reg.resolve("売れ筋商品").unwrap().clone();
    (env, r)
}

#[derive(Clone, Debug)]
struct Row {
    id: String,
    product: String,
    amount: i64,
    status: &'static str,
    at: i64,
}

/// 決定的なフィクスチャ。
fn fixture() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut n = 0;
    let mut push = |product: String, amount: i64, status: &'static str, at: i64, rows: &mut Vec<Row>| {
        n += 1;
        rows.push(Row {
            id: format!("L{n:04}"),
            product,
            amount,
            status,
            at,
        });
    };
    // 期間内の確定明細: P01..P25 に p%7 で同額が出るよう配置(同順位の並びを検証する)
    for p in 1..=25 {
        let pid = format!("P{p:02}");
        for k in 0..3 {
            let amount = 1000 * ((p % 7) + 1) + 10 * k;
            push(pid.clone(), amount, "確定", BASE_MS - (k + 1) * 3 * DAY - p * 60_000, &mut rows);
        }
    }
    // 取消は除外(巨額でも順位に影響しない)
    push("P25".into(), 1_000_000, "取消", BASE_MS - 2 * DAY, &mut rows);
    // 期間外(31 日前)は除外
    push("P24".into(), 2_000_000, "確定", BASE_MS - 31 * DAY, &mut rows);
    // 区間の始点ちょうどは含む
    push("P23".into(), 500_000, "確定", BASE_MS - 30 * DAY, &mut rows);
    // 区間の終点ちょうど(基準日時)は含まない
    push("P22".into(), 900_000, "確定", BASE_MS, &mut rows);
    // 始点の 1ms 前は含まない
    push("P21".into(), 800_000, "確定", BASE_MS - 30 * DAY - 1, &mut rows);
    rows
}

/// SQL を使わない参照実装(仕様の独立な読み)。
fn reference(rows: &[Row], limit: usize) -> Vec<(String, i64)> {
    let (ws, we) = (BASE_MS - 30 * DAY, BASE_MS);
    let mut sums: std::collections::BTreeMap<String, i64> = Default::default();
    for r in rows.iter().filter(|r| r.status == "確定" && r.at >= ws && r.at < we) {
        *sums.entry(r.product.clone()).or_default() += r.amount;
    }
    let mut v: Vec<(String, i64)> = sums.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(limit);
    v
}

fn run_sqlite(env: &Env, spec: &RankingSpec, rows: &[Row]) -> Vec<(String, i64)> {
    let conn = Connection::open_in_memory().unwrap();
    let ddl = sqlite_ddl(&env.data["注文明細"], env).unwrap();
    conn.execute_batch(&ddl).unwrap();
    for r in rows {
        conn.execute(
            "INSERT INTO \"注文明細\" (\"ID\", \"商品ID\", \"売上金額\", \"状態\", \"確定日時\") VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![r.id, r.product, r.amount, r.status, r.at],
        )
        .unwrap();
    }
    let q = lower_ranking(spec, Dialect::Sqlite).unwrap();
    let (ws, we) = window_ms(spec, BASE_MS).unwrap();
    let params: Vec<rusqlite::types::Value> = q
        .params
        .iter()
        .map(|p| match p {
            SqlParam::Text(s) => rusqlite::types::Value::Text(s.clone()),
            SqlParam::Int(i) => rusqlite::types::Value::Integer(*i),
            SqlParam::Bool(b) => rusqlite::types::Value::Integer(*b as i64),
            SqlParam::WindowStart => rusqlite::types::Value::Integer(ws),
            SqlParam::WindowEnd => rusqlite::types::Value::Integer(we),
        })
        .collect();
    let mut stmt = conn.prepare(&q.sql).unwrap();
    let out = stmt
        .query_map(params_from_iter(params.iter()), |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    out
}

#[test]
fn at07_golden_sql_sqlite_and_postgres() {
    let (_, spec) = load();
    let q = lower_ranking(&spec, Dialect::Sqlite).unwrap();
    assert_eq!(q.sql, GOLDEN_SQLITE.trim_end());
    assert_eq!(
        q.params,
        vec![SqlParam::Text("確定".into()), SqlParam::WindowStart, SqlParam::WindowEnd, SqlParam::Int(20)]
    );
    let pg = lower_ranking(&spec, Dialect::Postgres).unwrap();
    assert_eq!(pg.sql, GOLDEN_PG.trim_end());
}

#[test]
fn sec04_no_values_are_inlined_into_sql() {
    let (_, spec) = load();
    let q = lower_ranking(&spec, Dialect::Sqlite).unwrap();
    assert!(!q.sql.contains("確定\""), "状態の値はバインド変数");
    assert!(!q.sql.contains("'"), "文字列リテラルを含まない");
    assert!(!q.sql.contains("20"), "件数もバインド変数");
}

#[test]
fn at07_sqlite_results_match_reference_and_hand_checked_expectations() {
    let (env, spec) = load();
    let rows = fixture();
    let got = run_sqlite(&env, &spec, &rows);
    assert_eq!(got, reference(&rows, 20));
    assert_eq!(got.len(), 20);
    // 手で確認した期待値
    assert_eq!(got[0].0, "P23", "区間の始点ちょうどの明細を含む");
    assert_eq!(got[0].1, 500_000 + 3 * 1000 * ((23 % 7) + 1) + 30);
    let ids: Vec<&str> = got.iter().map(|r| r.0.as_str()).collect();
    assert_ne!(
        got.iter().find(|r| r.0 == "P22").map(|r| r.1),
        Some(900_000 + 3 * 1000 * ((22 % 7) + 1) + 30),
        "終点ちょうどは含まない"
    );
    assert!(!got.iter().any(|r| r.1 >= 800_000 && r.0 != "P23"), "取消・期間外・始点前の明細を含まない");
    // 同額(p%7 が同じ)のときは商品IDの昇順: P06 / P13 / P20 は同額(7000*3+30)
    let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap();
    assert!(pos("P06") < pos("P13") && pos("P13") < pos("P20"));
    assert_eq!(got[pos("P06")].1, got[pos("P20")].1);
}

#[test]
fn tc002_limit_override_changes_only_count() {
    let (env, mut spec) = load();
    spec.limit = 5;
    let rows = fixture();
    let got = run_sqlite(&env, &spec, &rows);
    assert_eq!(got, reference(&rows, 5));
    let q = lower_ranking(&spec, Dialect::Sqlite).unwrap();
    assert_eq!(q.sql, GOLDEN_SQLITE.trim_end(), "SQL 本文は同一で件数はパラメータだけが変わる");
    assert_eq!(q.params.last(), Some(&SqlParam::Int(5)));
}

#[test]
fn ddl_maps_types_and_enum_check() {
    let (env, _) = load();
    let ddl = sqlite_ddl(&env.data["注文明細"], &env).unwrap();
    assert!(ddl.contains("\"売上金額\" INTEGER NOT NULL"));
    assert!(ddl.contains("CHECK (\"状態\" IN ('確定', '取消'))"));
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&ddl).unwrap();
    let bad = conn.execute("INSERT INTO \"注文明細\" VALUES ('x','P','1','保留',0)", []);
    assert!(bad.is_err(), "列挙外の状態は DB でも拒否");
}
