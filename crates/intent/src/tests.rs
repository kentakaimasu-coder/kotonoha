use super::*;
use kotonoha_diag::FileId;
use kotonoha_knowledge::build_registry;
use kotonoha_types::collect_env;

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");
const RANKING: &str = include_str!("../../../examples/ranking/src/app.koto");

fn run(src: &str, approvals: &[ApprovedInterpretation]) -> (IntentModel, Vec<Diagnostic>) {
    let (f, pd) = kotonoha_parser::parse_file(src, FileId(0));
    assert!(pd.is_empty(), "{pd:#?}");
    let (env, ed) = collect_env(&[&f]);
    assert!(ed.iter().all(|d| !d.is_error()), "{ed:#?}");
    let (reg, kd) = build_registry(&[&f], &env);
    assert!(kd.is_empty(), "{kd:#?}");
    resolve_intents(&[&f], &env, &reg, approvals)
}

fn codes(d: &[Diagnostic]) -> Vec<Code> {
    d.iter().filter(|x| x.is_error()).map(|x| x.code).collect()
}

#[test]
fn shop_intents_resolve() {
    let (m, d) = run(SHOP, &[]);
    assert!(d.is_empty(), "{d:#?}");
    let list = &m.screens[0];
    assert_eq!(list.name, "商品一覧");
    assert!(matches!(&list.intents[0].kind, IntentKind::ListData { data, .. } if data == "商品"));
    assert!(matches!(&list.intents[1].kind, IntentKind::NavigateOnSelect { data, screen } if data == "商品" && screen == "商品詳細"));
    let detail = &m.screens[1];
    assert_eq!(detail.input.as_deref(), Some("商品"));
    assert!(matches!(&detail.intents[0].kind, IntentKind::ShowFields { fields, .. } if fields == &["名前", "価格", "在庫"]));
    assert!(matches!(&detail.intents[1].kind, IntentKind::ButtonInvoke { label, action, args } if label == "購入" && action == "注文する" && args.len() == 2));
    assert_eq!(m.contracts.len(), 3);
    assert!(m
        .contracts
        .iter()
        .any(|c| matches!(&c.kind, IntentKind::OwnerRead { data, field } if data == "注文" && field == "注文者")));
    assert!(m.unresolved.is_empty());
}

#[test]
fn tc001_tc002_knowledge_display_and_count_override() {
    let (m, d) = run(RANKING, &[]);
    assert!(d.is_empty(), "{d:#?}");
    let IntentKind::ShowKnowledge { spec, limit_override, .. } = &m.screens[0].intents[0].kind else {
        panic!()
    };
    assert_eq!((spec.window_days, spec.limit, spec.metric.output.as_str()), (30, 20, "売上金額"));
    assert_eq!(*limit_override, None);
    let IntentKind::ShowKnowledge {
        spec: s5, limit_override: o5, ..
    } = &m.screens[1].intents[0].kind
    else {
        panic!()
    };
    // 件数だけが 5 になり、他の意味は変わらない
    assert_eq!(*o5, Some(5));
    assert_eq!(s5.limit, 5);
    let mut s20 = (**s5).clone();
    s20.limit = 20;
    assert_eq!(&s20, spec.as_ref());
}

#[test]
fn tc003_undefined_popular_requires_confirmation() {
    let src = format!("{RANKING}\n画面 人気:\n    人気商品を表示\n");
    let (m, d) = run(&src, &[]);
    assert_eq!(codes(&d), [Code::AMB001]);
    assert!(d[0].hints.iter().any(|h| h.contains("「人気商品」")), "{:#?}", d[0]);
    assert!(d[0].hints.iter().any(|h| h.contains("既存の知識: 売れ筋商品")));
    assert_eq!(m.unresolved.len(), 1);
    assert_eq!(m.unresolved[0].mode, Mode::Strict);
}

fn approval_for(m: &IntentModel, canonical: &str) -> ApprovedInterpretation {
    let u = &m.unresolved[0];
    ApprovedInterpretation {
        intent_id: u.intent_id.clone(),
        sentence_hash: u.sentence_hash.clone(),
        canonical: canonical.into(),
        approval_hash: "h".into(),
    }
}

#[test]
fn flexible_mode_applies_human_approval_strict_does_not() {
    let flex = format!("{RANKING}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let (m, _) = run(&flex, &[]);
    let ap = approval_for(&m, "売れ筋商品を表示");
    let (m2, d2) = run(&flex, std::slice::from_ref(&ap));
    assert!(d2.is_empty(), "{d2:#?}");
    let r = &m2.screens.last().unwrap().intents[0];
    assert!(matches!(r.resolution, Resolution::Approved { .. }));
    assert!(matches!(&r.kind, IntentKind::ShowKnowledge { knowledge, .. } if knowledge == "売れ筋商品"));
    // 厳密モードでは承認があっても適用しない
    let strict = format!("{RANKING}\n画面 人気:\n    人気商品を表示\n");
    let (_, d3) = run(&strict, &[ap]);
    assert_eq!(codes(&d3), [Code::AMB001]);
}

#[test]
fn approval_must_be_a_known_pattern() {
    let flex = format!("{RANKING}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let (m, _) = run(&flex, &[]);
    let (_, d) = run(&flex, &[approval_for(&m, "DROP TABLE 注文明細")]);
    assert_eq!(codes(&d), [Code::APP001]);
    let (_, d) = run(&flex, &[approval_for(&m, "未定義知識を表示")]);
    assert_eq!(codes(&d), [Code::APP001]);
}

#[test]
fn spec_sample_contract_is_unresolved() {
    let src = SHOP.replace("    商品は誰でも閲覧可\n", "    個人情報は本人だけ閲覧\n");
    let (_, d) = run(&src, &[]);
    assert_eq!(codes(&d), [Code::AMB001]);
}

#[test]
fn screen_reference_errors() {
    let s = SHOP.replace("商品の名前と価格と在庫を表示", "商品の名前と色を表示");
    assert_eq!(codes(&run(&s, &[]).1), [Code::REF001]);
    let s = SHOP.replace("商品を選ぶと 商品詳細 へ", "商品を選ぶと 存在しない画面 へ");
    // 遷移先が無い + 商品詳細の表示対象が決まらない
    assert_eq!(codes(&run(&s, &[]).1), [Code::REF001; 4]); // 遷移先なし×2画面 + 表示対象/ボタン引数の 商品 が決まらない
    let s = SHOP.replace("注文する(商品.ID, 1)", "注文する(商品.ID, \"1\")");
    assert_eq!(codes(&run(&s, &[]).1), [Code::TYP001]);
    let s = SHOP.replace("注文する(商品.ID, 1)", "存在しない動作(1)");
    assert_eq!(codes(&run(&s, &[]).1), [Code::REF001]);
}

#[test]
fn intent_ids_are_stable_against_reordering() {
    let (m1, _) = run(SHOP, &[]);
    let swapped = SHOP.replace(
        "    商品を一覧表示\n    商品を選ぶと 商品詳細 へ\n",
        "    商品を選ぶと 商品詳細 へ\n    商品を一覧表示\n",
    );
    let (m2, _) = run(&swapped, &[]);
    let ids1: BTreeSet<_> = m1.screens[0].intents.iter().map(|i| i.id.clone()).collect();
    let ids2: BTreeSet<_> = m2.screens[0].intents.iter().map(|i| i.id.clone()).collect();
    assert_eq!(ids1, ids2);
}

#[test]
fn owner_contract_requires_id_or_string_field() {
    let s = SHOP.replace("注文は注文者が本人のものだけ閲覧可", "注文は数量が本人のものだけ閲覧可");
    assert_eq!(codes(&run(&s, &[]).1), [Code::TYP001]);
    let s = SHOP.replace("注文は注文者が本人のものだけ閲覧可", "注文は担当者が本人のものだけ閲覧可");
    assert_eq!(codes(&run(&s, &[]).1), [Code::REF001]);
}

const IZAKAYA: &str = "アプリ「店」\n\n列挙 注文状態:\n    調理中\n    提供済み\n\nデータ 注文:\n    ID: 識別子\n    卓: 識別子\n    品名: 文字列\n    数量: 整数\n    急ぎ: 真偽\n    状態: 注文状態\n    注文日時: 日時\n\n";

fn list_of(lines: &str) -> (Vec<IntentKind>, Vec<Code>) {
    let src = format!("{IZAKAYA}画面 一覧:\n{lines}");
    let (m, d) = run(&src, &[]);
    (m.screens[0].intents.iter().map(|i| i.kind.clone()).collect(), codes(&d))
}

#[test]
fn list_order_and_filters() {
    let (k, c) = list_of("    注文を注文日時の新しい順に一覧表示\n    注文のうち状態が調理中かつ数量が2のものを数量順に一覧表示\n    注文のうち注文日時が今日のものを一覧表示\n    注文のうち状態が提供済みでないかつ卓が本人かつ急ぎが真のものを一覧表示\n");
    assert!(c.is_empty(), "{c:?}");
    let IntentKind::ListData { order, filters, .. } = &k[0] else { panic!() };
    assert_eq!(
        order,
        &Some(ListOrder {
            field: "注文日時".into(),
            desc: true,
            label: "注文日時の新しい".into()
        })
    );
    assert!(filters.is_empty());
    let IntentKind::ListData { order, filters, .. } = &k[1] else { panic!() };
    assert_eq!(
        order,
        &Some(ListOrder {
            field: "数量".into(),
            desc: false,
            label: "数量".into()
        })
    );
    assert_eq!(
        list_query(order, filters),
        vec![
            ("eq.状態".to_string(), "調理中".to_string()),
            ("eq.数量".to_string(), "2".to_string()),
            ("order".to_string(), "数量".to_string()),
        ]
    );
    let IntentKind::ListData { filters, .. } = &k[2] else { panic!() };
    assert_eq!(filters[0].op, FilterOp::Today);
    let IntentKind::ListData { filters, .. } = &k[3] else { panic!() };
    assert_eq!(filters.iter().map(|f| f.op).collect::<Vec<_>>(), vec![FilterOp::Ne, FilterOp::Me, FilterOp::Eq]);
    assert_eq!(filters[2].value.as_deref(), Some("true"));
    assert_eq!(list_caption("注文", &None, filters), "注文の一覧(状態が提供済みでない・卓が自分・急ぎが真)");
    let IntentKind::ListData { order, filters, .. } = &k[0] else { panic!() };
    assert_eq!(list_caption("注文", order, filters), "注文の一覧(注文日時の新しい順)");
}

#[test]
fn list_filter_errors_are_reported() {
    for (line, code) in [
        ("注文のうち状態が壊れたのものを一覧表示", Code::TYP001),
        ("注文のうち色が赤のものを一覧表示", Code::REF001),
        ("注文を色順に一覧表示", Code::REF001),
        ("注文のうち品名が今日のものを一覧表示", Code::TYP001),
        ("注文のうち数量が多いのものを一覧表示", Code::TYP001),
        ("注文のうち数量が本人のものを一覧表示", Code::TYP001),
        ("注文のうち調理中のものを一覧表示", Code::SYN001),
        ("注文のうち注文日時が2026-12-05のものを一覧表示", Code::TYP001),
    ] {
        let (_, c) = list_of(&format!("    {line}\n"));
        assert_eq!(c, vec![code], "{line}");
    }
    // 知らないデータは未解決(推測しない)
    let (_, c) = list_of("    会計のうち状態が調理中のものを一覧表示\n");
    assert_eq!(c, vec![Code::AMB001]);
}

#[test]
fn aggregate_patterns() {
    let (k, c) = list_of(
        "    注文を状態ごとに集計\n    注文を注文日時の月ごとに集計\n    注文のうち状態が提供済みのものの数量を品名ごとに合計\n    注文の数量を卓ごとに合計\n",
    );
    assert!(c.is_empty(), "{c:?}");
    let IntentKind::Aggregate { by, bucket, sums, filters, .. } = &k[0] else {
        panic!()
    };
    assert_eq!((by.as_str(), *bucket, sums.clone(), filters.len()), ("状態", None, vec!["数量".to_string()], 0));
    let IntentKind::Aggregate { by, bucket, .. } = &k[1] else { panic!() };
    assert_eq!((by.as_str(), *bucket), ("注文日時", Some(Bucket::Month)));
    let IntentKind::Aggregate { by, sums, filters, .. } = &k[2] else { panic!() };
    assert_eq!(
        (by.as_str(), sums.clone(), filters[0].value.as_deref()),
        ("品名", vec!["数量".to_string()], Some("提供済み"))
    );
    assert_eq!(aggregate_caption("注文", by, None, filters), "注文の品名ごとの集計(状態が提供済み)");
    let IntentKind::Aggregate { by, .. } = &k[3] else { panic!() };
    assert_eq!(by, "卓");
    for (line, code) in [
        ("注文を色ごとに集計", Code::REF001),
        ("注文を注文日時ごとに集計", Code::TYP001),
        ("注文を品名の日ごとに集計", Code::TYP001),
        ("注文の品名を卓ごとに合計", Code::TYP001),
        ("注文の重さを卓ごとに合計", Code::REF001),
    ] {
        let (_, c) = list_of(&format!("    {line}\n"));
        assert_eq!(c, vec![code], "{line}");
    }
}

fn contracts_of(lines: &str) -> (Vec<IntentKind>, Vec<Code>) {
    let src = format!("{IZAKAYA}約束:\n{lines}");
    let (m, d) = run(&src, &[]);
    (m.contracts.iter().map(|i| i.kind.clone()).collect(), codes(&d))
}

#[test]
fn unique_and_sequence_contracts() {
    let (k, c) = contracts_of("    注文は卓と品名の組で重複不可\n    注文は品名が重複不可\n    注文の状態は 調理中 → 提供済み の順\n");
    assert!(c.is_empty(), "{c:?}");
    assert_eq!(
        k[0],
        IntentKind::Unique {
            data: "注文".into(),
            fields: vec!["卓".into(), "品名".into()]
        }
    );
    assert_eq!(
        k[1],
        IntentKind::Unique {
            data: "注文".into(),
            fields: vec!["品名".into()]
        }
    );
    assert_eq!(
        k[2],
        IntentKind::Sequence {
            data: "注文".into(),
            field: "状態".into(),
            values: vec!["調理中".into(), "提供済み".into()]
        }
    );
    for (line, code) in [
        ("注文は色の組で重複不可", Code::REF001),
        ("注文はIDが重複不可", Code::TYP001),
        ("注文は卓と卓の組で重複不可", Code::AMB002),
        ("注文の状態は 調理中 → 片付け済み の順", Code::TYP001),
        ("注文の状態は 調理中 の順", Code::AMB001),
        ("注文の数量は 1 → 2 の順", Code::TYP001),
        ("注文の状態は 提供済み → 提供済み の順", Code::AMB002),
    ] {
        let (_, c) = contracts_of(&format!("    {line}\n"));
        assert_eq!(c, vec![code], "{line}");
    }
}
