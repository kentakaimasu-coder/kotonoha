use super::*;
use kotonoha_ast::{Decl, SourceFile};
use kotonoha_diag::{Code, Diagnostic, FileId};

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");

fn check_src(src: &str) -> (Env, Vec<Diagnostic>) {
    let (f, pd) = kotonoha_parser::parse_file(src, FileId(0));
    assert!(pd.is_empty(), "parse: {pd:#?}");
    check_file(&f)
}

fn check_file(f: &SourceFile) -> (Env, Vec<Diagnostic>) {
    let (env, mut d) = collect_env(&[f]);
    for it in &f.items {
        if let Decl::Action(a) = &it.decl {
            d.extend(check_action(&env, a).1);
        }
    }
    (env, d)
}

fn errors(d: &[Diagnostic]) -> Vec<Code> {
    d.iter().filter(|x| x.is_error()).map(|x| x.code).collect()
}

const PRELUDE: &str = "アプリ「t」\n型 注文ID = 識別子\n列挙 注文失敗:\n    数量不正\n    在庫不足\nデータ 商品:\n    ID: 識別子\n    名前: 文字列\n    価格: 金額<JPY>\n    ドル価格: 金額<USD>\n    在庫: 整数\nアダプタ 商品をIDで取得(ID: 識別子) -> 商品:\n    操作 = 取得(商品)\n";

fn action(body: &str) -> Vec<Code> {
    let src = format!("{PRELUDE}動作 f(x: 整数, s: 文字列, id: 識別子) -> 結果<注文ID, 注文失敗>:\n    必要 = []\n    厳密:\n{body}");
    errors(&check_src(&src).1)
}

#[test]
fn shop_example_type_checks() {
    let (env, d) = check_src(SHOP);
    assert!(errors(&d).is_empty(), "{d:#?}");
    assert_eq!(env.app_name.as_deref(), Some("商店"));
    assert_eq!(env.data["商品"].field("価格"), Some(&Type::Money("JPY".into())));
    assert_eq!(env.aliases["注文ID"], Type::Id);
    let a = &env.actions["注文する"];
    assert_eq!(a.ret.as_ref().unwrap().to_string(), "結果<識別子, 注文失敗>");
}

#[test]
fn shop_action_facts() {
    let (f, _) = kotonoha_parser::parse_file(SHOP, FileId(0));
    let (env, _) = collect_env(&[&f]);
    let a = f.items.iter().find_map(|i| if let Decl::Action(a) = &i.decl { Some(a) } else { None }).unwrap();
    let (facts, d) = check_action(&env, a);
    assert!(d.is_empty(), "{d:#?}");
    let calls: Vec<&str> = facts.adapter_calls.iter().map(|c| c.0.as_str()).collect();
    assert_eq!(calls, ["商品をIDで取得", "注文を登録"]);
    assert_eq!(
        facts.mutations.iter().map(|m| (m.0.as_str(), m.1.as_str())).collect::<Vec<_>>(),
        [("商品", "在庫")]
    );
}

#[test]
fn tc005_type_mismatch_is_typ001() {
    assert_eq!(action("        y = x + s\n        id を返す\n"), [Code::TYP001]);
    assert_eq!(
        action("        x > \"a\" を確認 または 失敗(\"数量不正\")\n        id を返す\n"),
        [Code::TYP001]
    );
    assert_eq!(action("        x を返す\n"), [Code::TYP001]);
    assert_eq!(action("        x > 0 を確認 または 失敗(\"不明な理由\")\n        id を返す\n"), [Code::TYP001]);
    assert_eq!(action("        y = 1\n        y = \"a\"\n        id を返す\n"), [Code::TYP001]);
    assert_eq!(action("        y = 1.5 + 1\n        id を返す\n"), [Code::TYP001]);
}

#[test]
fn money_currency_and_division_rules() {
    let ok = action("        p = 商品をIDで取得(id)\n        q = p.価格 + p.価格\n        r = p.価格 * 2\n        q > r を確認 または 失敗(\"数量不正\")\n        id を返す\n");
    assert!(ok.is_empty(), "{ok:?}");
    assert_eq!(
        action("        p = 商品をIDで取得(id)\n        q = p.価格 + p.ドル価格\n        id を返す\n"),
        [Code::TYP001]
    );
    assert_eq!(
        action("        p = 商品をIDで取得(id)\n        q = p.価格 / 2\n        id を返す\n"),
        [Code::TYP001]
    );
}

#[test]
fn references_and_assignment_rules() {
    assert_eq!(action("        y = 未定義(x)\n        id を返す\n"), [Code::REF001]);
    assert_eq!(action("        x = 2\n        id を返す\n"), [Code::TYP002]);
    // 取引外の更新
    assert_eq!(
        action("        p = 商品をIDで取得(id)\n        p.在庫 = 1\n        id を返す\n"),
        [Code::TYP002]
    );
    // 取引内なら OK
    assert!(action("        取引:\n            p = 商品をIDで取得(id)\n            p.在庫 = p.在庫 - x\n        id を返す\n").is_empty());
    // ID は更新不可
    assert_eq!(
        action("        取引:\n            p = 商品をIDで取得(id)\n            p.ID = id\n        id を返す\n"),
        [Code::TYP002]
    );
    // ブロックスコープ
    assert_eq!(action("        もし x > 0:\n            y = 1\n        y を返す\n"), [Code::REF001]);
}

#[test]
fn missing_return_is_typ003() {
    assert_eq!(action("        y = 1\n"), [Code::TYP003]);
    assert!(action("        もし x > 0:\n            id を返す\n        そうでなければ:\n            id を返す\n").is_empty());
    assert_eq!(action("        もし x > 0:\n            id を返す\n"), [Code::TYP003]);
}

#[test]
fn enum_literal_comparison() {
    let src = "アプリ「t」\n列挙 状態:\n    確定\n    取消\nデータ 明細:\n    ID: 識別子\n    状態: 状態\nアダプタ 明細取得(ID: 識別子) -> 明細:\n    操作 = 取得(明細)\n動作 f(id: 識別子) -> 真偽:\n    必要 = []\n    厳密:\n        m = 明細取得(id)\n        m.状態 == \"確定\" を返す\n";
    assert!(errors(&check_src(src).1).is_empty());
    let bad = src.replace("\"確定\" を返す", "\"保留\" を返す");
    assert_eq!(errors(&check_src(&bad).1), [Code::TYP001]);
}

#[test]
fn declaration_errors() {
    let (_, d) = check_src("アプリ「t」\nデータ 整数:\n    a: 整数\n");
    assert_eq!(errors(&d), [Code::REF002]);
    let (_, d) = check_src("アプリ「t」\nデータ A:\n    a: 未知型\n");
    assert_eq!(errors(&d), [Code::REF001]);
    let (_, d) = check_src("アプリ「t」\nデータ A:\n    a: 金額<円>\n");
    assert_eq!(errors(&d), [Code::TYP001]);
    let (_, d) = check_src("アプリ「t」\n型 A = B\n型 B = A\n");
    assert!(errors(&d).contains(&Code::REF001));
    let (_, d) = check_src("データ A:\n    a: 整数\n");
    assert_eq!(errors(&d), [Code::REF001], "アプリ宣言なし");
    let (_, d) = check_src("アプリ「t」\nデータ A:\n    ID: 識別子\nアダプタ g(ID: 識別子) -> 整数:\n    操作 = 取得(A)\n");
    assert_eq!(errors(&d), [Code::TYP001]);
    let (_, d) = check_src("アプリ「t」\nデータ A:\n    ID: 識別子\n動作 f():\n    必要 = [A.盗む]\n    厳密:\n        f()\n");
    assert_eq!(errors(&d), [Code::REF001]);
}

#[test]
fn spec_sample_undefined_names_are_ref001() {
    // 仕様書 §2 サンプル(空白だけ補正)の未定義型・未定義アダプタ
    let src = include_str!("../../../tests/fixtures/spec_sample/app.koto").replace("注文IDを返す", "注文ID を返す");
    let (_, d) = check_src(&src);
    let msgs: Vec<String> = d.iter().filter(|x| x.is_error()).map(|x| format!("{} {}", x.code, x.message_ja)).collect();
    assert!(msgs.iter().any(|m| m.contains("REF001") && m.contains("注文ID")), "{msgs:#?}");
    assert!(msgs.iter().any(|m| m.contains("REF001") && m.contains("注文失敗")), "{msgs:#?}");
    assert!(msgs.iter().any(|m| m.contains("REF001") && m.contains("商品をIDで取得")), "{msgs:#?}");
    assert!(msgs.iter().any(|m| m.contains("REF001") && m.contains("資源 `注文`")), "{msgs:#?}");

    let (_, d) = check_src(
        "アプリ「t」\nデータ A:\n    ID: 識別子\n    名前: 文字列\n    備考: 選択<文字列>\nアダプタ g(備考: 選択<文字列>) -> 識別子:\n    操作 = 登録(A)\n",
    );
    assert_eq!(errors(&d), [Code::TYP001], "必須フィールド 名前 が無い 登録");
}
