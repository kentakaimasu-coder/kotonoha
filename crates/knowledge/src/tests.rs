use super::*;
use kotonoha_diag::FileId;
use kotonoha_types::collect_env;

const RANKING: &str = include_str!("../../../examples/ranking/src/app.koto");

fn build(src: &str) -> (Registry, Vec<Diagnostic>) {
    let (f, pd) = kotonoha_parser::parse_file(src, FileId(0));
    assert!(pd.is_empty(), "{pd:#?}");
    let (env, ed) = collect_env(&[&f]);
    assert!(ed.iter().all(|d| !d.is_error()), "{ed:#?}");
    build_registry(&[&f], &env)
}

fn codes(d: &[Diagnostic]) -> Vec<Code> {
    d.iter().filter(|x| x.is_error()).map(|x| x.code).collect()
}

#[test]
fn tc001_ranking_meaning_is_fixed() {
    let (reg, d) = build(RANKING);
    assert!(d.is_empty(), "{d:#?}");
    let Knowledge::Ranking(r) = reg.resolve("売れ筋商品").unwrap();
    assert_eq!(r.window_days, 30);
    assert_eq!(r.metric.agg, Agg::Sum);
    assert_eq!(r.metric.field.as_deref(), Some("売上金額"));
    assert_eq!(r.metric.ty, Type::Money("JPY".into()));
    assert_eq!(r.limit, 20);
    assert_eq!(r.unit, "商品ID");
    assert_eq!(
        r.order,
        vec![
            OrderKey {
                column: "売上金額".into(),
                dir: Dir::Desc
            },
            OrderKey {
                column: "商品ID".into(),
                dir: Dir::Asc
            }
        ]
    );
    assert_eq!(r.time_zone, "Asia/Tokyo");
    assert_eq!(r.tax, Tax::Included);
    assert_eq!(kotonoha_parser::printer::expr_str(r.filter.as_ref().unwrap()), "状態 == \"確定\"");
    assert_eq!(r.result_type().to_string(), "一覧<記録{商品ID: 識別子, 売上金額: 金額<JPY>}>");
}

fn with(key: &str, value: &str) -> String {
    let mut out = String::new();
    for line in RANKING.lines() {
        if line.trim_start().starts_with(&format!("{key} =")) {
            if !value.is_empty() {
                out.push_str(&format!("    {key} = {value}\n"));
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn vague_or_missing_business_decisions_are_questions() {
    for (key, value) in [
        ("期間", "最近"),
        ("期間", ""),
        ("時刻帯", ""),
        ("税区分", ""),
        ("条件", ""),
        ("指標", "売上金額の平均"),
        ("税区分", "だいたい"),
    ] {
        let (_, d) = build(&with(key, value));
        assert!(codes(&d).contains(&Code::AMB001), "{key}={value}: {d:#?}");
        assert!(d.iter().any(|x| x.hints.iter().any(|h| h.starts_with("質問:"))), "{key}");
    }
}

#[test]
fn tie_breaker_is_required_for_determinism() {
    let (_, d) = build(&with("並び", "売上金額の降順"));
    assert_eq!(codes(&d), [Code::AMB001]);
}

#[test]
fn invalid_references_and_values() {
    assert_eq!(codes(&build(&with("対象", "存在しない")).1), [Code::REF001]);
    assert_eq!(codes(&build(&with("単位", "存在しない")).1), [Code::REF001]);
    assert_eq!(codes(&build(&with("日時列", "売上金額")).1), [Code::TYP001]);
    assert_eq!(codes(&build(&with("時刻帯", "America/New_York")).1), [Code::TGT001]);
    assert_eq!(codes(&build(&with("条件", "状態 == \"保留\"")).1), [Code::TYP001]);
    assert_eq!(codes(&build(&with("条件", "売上金額")).1), [Code::TYP001]);
    assert_eq!(codes(&build(&with("件数", "0")).1), [Code::AMB001]);
    assert_eq!(codes(&build(&with("並び", "状態の昇順, 商品IDの昇順")).1), [Code::REF001]);
}

#[test]
fn unknown_keys_are_rejected() {
    let src = RANKING.replace("    件数 = 20\n", "    件数 = 20\n    人気度 = 高\n");
    assert_eq!(codes(&build(&src).1), [Code::REF001]);
}

#[test]
fn aliases_resolve_uniquely() {
    let src = RANKING.replace("    対象 = 注文明細\n", "    別名 = [人気商品, ヒット商品]\n    対象 = 注文明細\n");
    let (reg, d) = build(&src);
    assert!(d.is_empty(), "{d:#?}");
    assert_eq!(reg.resolve("人気商品").unwrap().name(), "売れ筋商品");
    // 2 つの知識が同じ別名を持つと AMB002
    let dup_block = src.split("知識 売れ筋商品:").nth(1).unwrap().split("\n\n").next().unwrap().to_string();
    let src2 = format!("{src}\n知識 別ランキング:{dup_block}\n");
    let (_, d) = build(&src2);
    assert!(codes(&d).contains(&Code::AMB002), "{d:#?}");
}

#[test]
fn recent_days_parser() {
    assert_eq!(parse_recent_days("直近30日"), Some(30));
    assert_eq!(parse_recent_days("直近030日"), None);
    assert_eq!(parse_recent_days("最近"), None);
    assert_eq!(parse_recent_days("直近日"), None);
}
