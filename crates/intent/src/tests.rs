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
    assert!(matches!(&list.intents[0].kind, IntentKind::ListData { data } if data == "商品"));
    assert!(matches!(&list.intents[1].kind, IntentKind::NavigateOnSelect { data, screen } if data == "商品" && screen == "商品詳細"));
    let detail = &m.screens[1];
    assert_eq!(detail.input.as_deref(), Some("商品"));
    assert!(matches!(&detail.intents[0].kind, IntentKind::ShowFields { fields, .. } if fields == &["名前", "価格", "在庫"]));
    assert!(matches!(&detail.intents[1].kind, IntentKind::ButtonInvoke { label, action, args } if label == "購入" && action == "注文する" && args.len() == 2));
    assert_eq!(m.contracts.len(), 2);
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
