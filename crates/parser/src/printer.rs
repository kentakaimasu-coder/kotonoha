//! AST → 正規形ソース。AST ラウンドトリップ(技術仕様書 §16 Property)の基準。
//!
//! 注意: Surface AST はコメントを保持しないため、このプリンタの出力でソースを上書きしてはならない。
//! `koto fmt` はテキスト正規化(NFC・改行・末尾空白)のみを行う(ADR-0107)。

use kotonoha_ast::*;

pub fn print_file(f: &SourceFile) -> String {
    let mut out = String::new();
    for (i, item) in f.items.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for a in &item.annotations {
            out.push('@');
            out.push_str(&a.name.name);
            if let Some(arg) = &a.arg {
                out.push('(');
                out.push_str(&arg.name);
                out.push(')');
            }
            out.push('\n');
        }
        print_decl(&item.decl, &mut out);
    }
    out
}

fn ind(n: usize) -> String {
    " ".repeat(n * 4)
}

fn print_decl(d: &Decl, out: &mut String) {
    match d {
        Decl::App(a) => out.push_str(&format!("アプリ「{}」\n", a.name)),
        Decl::TypeAlias(t) => out.push_str(&format!("型 {} = {}\n", t.name.name, type_str(&t.ty))),
        Decl::Enum(e) => {
            out.push_str(&format!("列挙 {}:\n", e.name.name));
            for v in &e.variants {
                out.push_str(&format!("{}{}\n", ind(1), v.name));
            }
        }
        Decl::Data(dd) => {
            out.push_str(&format!("データ {}:\n", dd.name.name));
            for f in &dd.fields {
                out.push_str(&format!("{}{}: {}\n", ind(1), f.name.name, type_str(&f.ty)));
            }
        }
        Decl::Adapter(a) => {
            out.push_str(&format!("アダプタ {}({}) -> {}:\n", a.name.name, params_str(&a.params), type_str(&a.ret)));
            out.push_str(&format!("{}操作 = {}({})\n", ind(1), a.op.word(), a.target.name));
        }
        Decl::Knowledge(k) => {
            out.push_str(&format!("知識 {}:\n", k.name.name));
            for e in &k.entries {
                out.push_str(&format!("{}{} = {}\n", ind(1), e.key.name, e.value.text));
            }
        }
        Decl::Screen(s) => {
            out.push_str(&format!("画面 {}:\n", s.name.name));
            for l in &s.lines {
                out.push_str(&format!("{}{}\n", ind(1), l.text));
            }
        }
        Decl::Action(a) => {
            out.push_str(&format!("動作 {}({})", a.name.name, params_str(&a.params)));
            if let Some(r) = &a.ret {
                out.push_str(&format!(" -> {}", type_str(r)));
            }
            out.push_str(":\n");
            for it in &a.items {
                match it {
                    ActionItem::Requires { caps, .. } => {
                        let s: Vec<String> = caps.iter().map(|c| format!("{}.{}", c.resource.name, c.verb.name)).collect();
                        out.push_str(&format!("{}必要 = [{}]\n", ind(1), s.join(", ")));
                    }
                    ActionItem::Effects { effects, .. } => {
                        let s: Vec<&str> = effects.iter().map(|e| e.path.as_str()).collect();
                        out.push_str(&format!("{}効果 = [{}]\n", ind(1), s.join(", ")));
                    }
                    ActionItem::Strict(b) => {
                        out.push_str(&format!("{}厳密:\n", ind(1)));
                        print_block(b, 2, out);
                    }
                    ActionItem::Intent(t) => out.push_str(&format!("{}{}\n", ind(1), t.text)),
                }
            }
        }
        Decl::Function(f) => {
            out.push_str(&format!("関数 {}({}) -> {}:\n", f.name.name, params_str(&f.params), type_str(&f.ret)));
            print_block(&f.body, 1, out);
        }
        Decl::External(e) => {
            out.push_str(&format!("{} {}({})", e.kind.word(), e.name.name, params_str(&e.params)));
            if let Some(r) = &e.ret {
                out.push_str(&format!(" -> {}", type_str(r)));
            }
            out.push_str(":\n");
            for kv in &e.entries {
                out.push_str(&format!("{}{} = {}\n", ind(1), kv.key.name, kv.value.text));
            }
        }
        Decl::Experiment(x) => {
            out.push_str(&format!("実験 {}:\n", x.name.name));
            for kv in &x.entries {
                out.push_str(&format!("{}{} = {}\n", ind(1), kv.key.name, kv.value.text));
            }
        }
        Decl::Role(x) => {
            out.push_str(&format!("役割 {}:\n", x.name.name));
            for kv in &x.entries {
                out.push_str(&format!("{}{} = {}\n", ind(1), kv.key.name, kv.value.text));
            }
        }
        Decl::Scenario(x) => {
            out.push_str(&format!("確かめる {}:\n", x.name.name));
            for l in &x.lines {
                out.push_str(&format!("{}{}\n", ind(1), l.text));
            }
        }
        Decl::Contract(c) => {
            out.push_str("約束:\n");
            for l in &c.lines {
                out.push_str(&format!("{}{}\n", ind(1), l.text));
            }
        }
    }
}

fn params_str(ps: &[Param]) -> String {
    ps.iter()
        .map(|p| format!("{}: {}", p.name.name, type_str(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn type_str(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { name, args, .. } => {
            if args.is_empty() {
                name.name.clone()
            } else {
                format!("{}<{}>", name.name, args.iter().map(type_str).collect::<Vec<_>>().join(", "))
            }
        }
        TypeExpr::Record { fields, .. } => {
            format!(
                "記録{{{}}}",
                fields
                    .iter()
                    .map(|f| format!("{}: {}", f.name.name, type_str(&f.ty)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

fn print_block(b: &Block, level: usize, out: &mut String) {
    for s in &b.stmts {
        print_stmt(s, level, out);
    }
}

fn print_stmt(s: &Stmt, level: usize, out: &mut String) {
    let i = ind(level);
    match s {
        Stmt::Check { cond, fail, .. } => {
            let arg = match &fail.kind {
                ExprKind::Call { args, .. } if args.len() == 1 => expr_str(&args[0]),
                _ => expr_str(fail),
            };
            out.push_str(&format!("{i}{} を確認 または 失敗({arg})\n", expr_str(cond)));
        }
        Stmt::Return { value, .. } => out.push_str(&format!("{i}{} を返す\n", expr_str(value))),
        Stmt::If { branches, else_block, .. } => {
            for (k, (c, b)) in branches.iter().enumerate() {
                if k == 0 {
                    out.push_str(&format!("{i}もし {}:\n", expr_str(c)));
                } else {
                    out.push_str(&format!("{i}そうでなければ もし {}:\n", expr_str(c)));
                }
                print_block(b, level + 1, out);
            }
            if let Some(b) = else_block {
                out.push_str(&format!("{i}そうでなければ:\n"));
                print_block(b, level + 1, out);
            }
        }
        Stmt::Loop { var, iter, body, .. } => {
            out.push_str(&format!("{i}繰り返す {} から {}:\n", var.name, expr_str(iter)));
            print_block(body, level + 1, out);
        }
        Stmt::Transaction { body, .. } => {
            out.push_str(&format!("{i}取引:\n"));
            print_block(body, level + 1, out);
        }
        Stmt::Assign { target, value, .. } => {
            let mut p = target.root.name.clone();
            for f in &target.fields {
                p.push('.');
                p.push_str(&f.name);
            }
            out.push_str(&format!("{i}{p} = {}\n", expr_str(value)));
        }
        Stmt::Expr { expr, .. } => out.push_str(&format!("{i}{}\n", expr_str(expr))),
    }
}

pub fn str_lit(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if c.is_control() => o.push_str(&format!("\\u{{{:X}}}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// 優先順位に基づき最小限の括弧で出力する。
pub fn expr_str(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Decimal(v) => v.clone(),
        ExprKind::Str(s) => str_lit(s),
        ExprKind::Bool(true) => "真".into(),
        ExprKind::Bool(false) => "偽".into(),
        ExprKind::None => "なし".into(),
        ExprKind::Ident(n) => n.clone(),
        ExprKind::Field(b, f) => {
            let bs = match b.kind {
                ExprKind::Binary { .. } | ExprKind::Unary { .. } => format!("({})", expr_str(b)),
                _ => expr_str(b),
            };
            format!("{bs}.{}", f.name)
        }
        ExprKind::Call { callee, args } => {
            format!("{}({})", callee.name, args.iter().map(expr_str).collect::<Vec<_>>().join(", "))
        }
        ExprKind::Unary { op, expr } => {
            let o = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "!",
            };
            let inner = match expr.kind {
                ExprKind::Binary { .. } => format!("({})", expr_str(expr)),
                // `- -x` の誤読防止
                ExprKind::Unary { .. } => format!("({})", expr_str(expr)),
                _ => expr_str(expr),
            };
            format!("{o}{inner}")
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let p = op.precedence();
            let side = |child: &Expr, right: bool| -> String {
                if let ExprKind::Binary { op: cop, .. } = &child.kind {
                    let cp = cop.precedence();
                    if cp < p || (cp == p && (right || op.is_non_assoc())) {
                        return format!("({})", expr_str(child));
                    }
                }
                expr_str(child)
            };
            format!("{} {} {}", side(lhs, false), op.symbol(), side(rhs, true))
        }
        ExprKind::List(items) => format!("[{}]", items.iter().map(expr_str).collect::<Vec<_>>().join(", ")),
    }
}
