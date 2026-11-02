use super::printer::{expr_str, print_file};
use super::*;

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");
const SPEC_SAMPLE: &str = include_str!("../../../tests/fixtures/spec_sample/app.koto");

fn parse_ok(src: &str) -> SourceFile {
    let (f, d) = parse_file(src, FileId(0));
    assert!(d.is_empty(), "unexpected diagnostics: {:#?}", d);
    f
}

fn parse_err(src: &str) -> Vec<Diagnostic> {
    let (_, d) = parse_file(src, FileId(0));
    assert!(!d.is_empty(), "expected diagnostics for:\n{src}");
    d
}

/// Span を除いた構造比較用 JSON。
fn strip_spans(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.remove("span");
            m.remove("op_span");
            for (_, x) in m.iter_mut() {
                strip_spans(x);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_spans),
        _ => {}
    }
}

fn shape(f: &SourceFile) -> serde_json::Value {
    let mut v = serde_json::to_value(f).unwrap();
    strip_spans(&mut v);
    v
}

#[test]
fn parses_shop_example() {
    let f = parse_ok(SHOP);
    let kinds: Vec<&str> = f
        .items
        .iter()
        .map(|i| match &i.decl {
            Decl::App(_) => "app",
            Decl::TypeAlias(_) => "type",
            Decl::Enum(_) => "enum",
            Decl::Data(_) => "data",
            Decl::Adapter(_) => "adapter",
            Decl::Knowledge(_) => "knowledge",
            Decl::Screen(_) => "screen",
            Decl::Action(_) => "action",
            Decl::Contract(_) => "contract",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "app",
            "type",
            "enum",
            "enum",
            "data",
            "data",
            "data",
            "adapter",
            "adapter",
            "screen",
            "screen",
            "action",
            "knowledge",
            "screen",
            "contract"
        ]
    );
    let Decl::Action(a) = &f.items[11].decl else { panic!() };
    assert_eq!(a.name.name, "注文する");
    assert_eq!(a.params.len(), 2);
    let (caps, _) = a.requires().unwrap();
    assert_eq!(
        caps.iter().map(|c| format!("{}.{}", c.resource.name, c.verb.name)).collect::<Vec<_>>(),
        ["注文.作成", "商品.読取", "商品.更新"]
    );
    let body = a.strict_blocks().next().unwrap();
    assert!(matches!(body.stmts[0], Stmt::Check { .. }));
    let Stmt::Transaction { body: tx, .. } = &body.stmts[1] else { panic!() };
    assert_eq!(tx.stmts.len(), 5);
    assert!(matches!(tx.stmts[4], Stmt::Return { .. }));
    let Stmt::Assign { target, .. } = &tx.stmts[2] else { panic!() };
    assert_eq!(target.root.name, "商品");
    assert_eq!(target.fields[0].name, "在庫");
}

#[test]
fn spec_sample_fails_only_on_unspaced_return() {
    // 仕様書 §2 の `注文IDを返す` は 1 語になるため SYN001(Phase 0 指摘 C-03)
    let d = parse_err(SPEC_SAMPLE);
    assert_eq!(d.len(), 1, "{d:#?}");
    assert_eq!(d[0].code, Code::SYN001);
    assert_eq!(d[0].span.line, 26);
    assert!(d[0].hints[0].contains("注文ID を返す"));
}

#[test]
fn ast_roundtrip_is_fixpoint_on_examples() {
    for src in [SHOP, include_str!("../../../examples/ranking/src/app.koto")] {
        let f1 = parse_ok(src);
        let printed = print_file(&f1);
        let f2 = parse_ok(&printed);
        assert_eq!(shape(&f1), shape(&f2), "AST が保存されない:\n{printed}");
        assert_eq!(printed, print_file(&f2), "印字が不動点でない");
    }
}

#[test]
fn spans_point_at_source() {
    let f = parse_ok(SHOP);
    let Decl::Data(d) = &f.items[4].decl else { panic!() };
    assert_eq!(d.name.name, "商品");
    let fld = &d.fields[2];
    assert_eq!(&SHOP[fld.name.span.start as usize..fld.name.span.end as usize], "価格");
    assert_eq!(fld.name.span.column, 5);
}

fn expr(s: &str) -> Expr {
    parse_expr_str(s, Span::new(FileId(0), 1, 1, 0, s.len() as u32)).unwrap()
}

#[test]
fn operator_precedence() {
    let cases = [
        ("1 + 2 * 3", "1 + 2 * 3"),
        ("(1 + 2) * 3", "(1 + 2) * 3"),
        ("a || b && c", "a || b && c"),
        ("(a || b) && c", "(a || b) && c"),
        ("a - (b - c)", "a - (b - c)"),
        ("a - b - c", "a - b - c"),
        ("-x.y", "-x.y"),
        ("!(a == b)", "!(a == b)"),
        ("x + 1 >= y * 2 && z != 0", "x + 1 >= y * 2 && z != 0"),
    ];
    for (input, want) in cases {
        assert_eq!(expr_str(&expr(input)), want, "input: {input}");
    }
    // 1 + 2 * 3 の木構造
    let ExprKind::Binary { op: BinOp::Add, rhs, .. } = expr("1 + 2 * 3").kind else {
        panic!()
    };
    assert!(matches!(rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
    let ExprKind::Binary { op: BinOp::Sub, lhs, .. } = expr("a - b - c").kind else {
        panic!()
    };
    assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Sub, .. }), "左結合");
}

#[test]
fn comparison_chains_are_rejected() {
    let s = "a < b < c";
    assert_eq!(parse_expr_str(s, Span::new(FileId(0), 1, 1, 0, 9)).unwrap_err().code, Code::SYN001);
    let s = "a == b == c";
    assert!(parse_expr_str(s, Span::new(FileId(0), 1, 1, 0, 11)).is_err());
}

#[test]
fn if_elif_else_and_loop() {
    let src = "アプリ「x」\n動作 f(a: 整数) -> 整数:\n    必要 = []\n    厳密:\n        もし a > 0:\n            1 を返す\n        そうでなければ もし a == 0:\n            0 を返す\n        そうでなければ:\n            繰り返す x から [1, 2]:\n                g(x)\n            -1 を返す\n";
    let f = parse_ok(src);
    let Decl::Action(a) = &f.items[1].decl else { panic!() };
    let b = a.strict_blocks().next().unwrap();
    let Stmt::If { branches, else_block, .. } = &b.stmts[0] else { panic!() };
    assert_eq!(branches.len(), 2);
    let eb = else_block.as_ref().unwrap();
    assert!(matches!(eb.stmts[0], Stmt::Loop { .. }));
    assert_eq!(b.stmts.len(), 1);
}

#[test]
fn syntax_errors_are_reported_with_positions() {
    let cases: &[(&str, u32)] = &[
        ("データ 商品\n    名前: 文字列\n", 1),                                       // コロン欠落
        ("データ 商品:\n", 1),                                                        // 空ブロック
        ("データ 商品:\n    名前 文字列\n", 2),                                       // フィールドのコロン欠落
        ("画面 一覧:\n    a\n        b\n", 3),                                        // 深すぎるインデント
        ("    アプリ「x」\n", 1),                                                     // トップレベルのインデント
        ("動作 f():\n    厳密:\n        そうでなければ:\n            1 を返す\n", 3), // もし なし
        ("動作 f():\n    厳密:\n        1 + 2\n", 3),                                 // 呼出でない式文
        ("アダプタ a() -> 整数:\n    操作 = 盗む(x)\n", 2),                           // 未知の操作種別
        ("動作 f():\n    必要 = [商品]\n    厳密:\n        g()\n", 2),                // Capability 形式
        ("データ 型:\n    a: 整数\n", 1),                                             // 予約語を名前に
    ];
    for (src, line) in cases {
        let d = parse_err(src);
        assert!(d.iter().all(|x| x.code == Code::SYN001), "{src}: {d:#?}");
        assert_eq!(d[0].span.line, *line, "{src}: {d:#?}");
    }
}

#[test]
fn recovers_and_reports_multiple_errors() {
    let src = "データ 商品:\n    a 整数\n    b: 整数\n    c 整数\n画面 x:\n    商品を一覧表示\n";
    let (f, d) = parse_file(src, FileId(0));
    assert_eq!(d.len(), 2);
    assert_eq!(f.items.len(), 2);
}

#[test]
fn knowledge_values_are_raw_and_double_equals_kept() {
    let f = parse_ok("知識 k:\n    条件 = 状態 == \"確定\"\n    期間 = 直近30日\n");
    let Decl::Knowledge(k) = &f.items[0].decl else { panic!() };
    assert_eq!(k.entries[0].key.name, "条件");
    assert_eq!(k.entries[0].value.text, "状態 == \"確定\"");
    assert_eq!(k.entries[1].value.text, "直近30日");
    assert_eq!(k.entries[1].value.span.column, 10);
}

#[test]
fn annotations_attach_to_next_decl() {
    let f = parse_ok("@柔軟(確認)\n画面 x:\n    人気商品を表示\n");
    assert_eq!(f.items[0].annotations[0].name.name, "柔軟");
    assert_eq!(f.items[0].annotations[0].arg.as_ref().unwrap().name, "確認");
}

#[test]
fn types_with_generics_and_records() {
    let t = parse_type_str("結果<一覧<記録{a: 整数, b: 金額<JPY>}>, 文字列>", Span::default()).unwrap();
    assert_eq!(printer::type_str(&t), "結果<一覧<記録{a: 整数, b: 金額<JPY>}>, 文字列>");
}
