//! 名前付き引数(ADR-0134): `予約を登録(枠ID: 枠ID, 予約者: 本人, 人数: 人数, 予約日時: 現在日時())`。
//! 型検査の前に、呼び出し先の宣言の順へ並べ替えて普通の引数にする。省いた引数は、型が `選択<…>` なら `なし` を入れる。
//! 以降の検査・生成は位置で渡した呼び出しとして扱う(書いた順は意味を持たない)。

use crate::{Env, Type};
use kotonoha_ast::{ActionItem, Block, Decl, Expr, ExprKind, SourceFile, Stmt};
use kotonoha_diag::{Code, Diagnostic};

fn params<'e>(env: &'e Env, name: &str) -> Option<&'e [(String, Type)]> {
    env.actions
        .get(name)
        .map(|s| s.params.as_slice())
        .or_else(|| env.adapters.get(name).map(|s| s.params.as_slice()))
        .or_else(|| env.functions.get(name).map(|s| s.params.as_slice()))
        .or_else(|| env.externals.get(name).map(|s| s.params.as_slice()))
}

fn expr(env: &Env, e: &mut Expr, diags: &mut Vec<Diagnostic>) {
    match &mut e.kind {
        ExprKind::Call { callee, args } => {
            for a in args.iter_mut() {
                match &mut a.kind {
                    ExprKind::Named { value, .. } => expr(env, value, diags),
                    _ => expr(env, a, diags),
                }
            }
            if !args.iter().any(|a| matches!(a.kind, ExprKind::Named { .. })) {
                return;
            }
            let Some(ps) = params(env, &callee.name) else {
                diags.push(Diagnostic::error(
                    Code::REF001,
                    callee.span,
                    format!("`{}` は名前付きの引数(`項目名: 値`)で呼べません(動作・データの操作・関数だけ)", callee.name),
                ));
                return;
            };
            if args.iter().any(|a| !matches!(a.kind, ExprKind::Named { .. })) {
                diags.push(
                    Diagnostic::error(
                        Code::SYN001,
                        e.span,
                        format!("`{}` の引数は、全部に名前を付けるか、全部を順に並べるかのどちらかにします", callee.name),
                    )
                    .hint(format!("引数: {}", ps.iter().map(|(n, t)| format!("{n}({t})")).collect::<Vec<_>>().join(", "))),
                );
                return;
            }
            let mut given: Vec<Option<Expr>> = vec![None; ps.len()];
            let mut ok = true;
            for a in std::mem::take(args) {
                let ExprKind::Named { name, value } = a.kind else { unreachable!() };
                match ps.iter().position(|(n, _)| *n == name.name) {
                    Some(i) if given[i].is_none() => given[i] = Some(*value),
                    Some(_) => {
                        diags.push(Diagnostic::error(Code::SYN001, name.span, format!("引数 `{}` が 2 回あります", name.name)));
                        ok = false;
                    }
                    None => {
                        diags.push(
                            Diagnostic::error(Code::REF001, name.span, format!("`{}` に引数 `{}` はありません", callee.name, name.name))
                                .hint(format!("引数: {}", ps.iter().map(|(n, t)| format!("{n}({t})")).collect::<Vec<_>>().join(", "))),
                        );
                        ok = false;
                    }
                }
            }
            let mut missing = Vec::new();
            for (i, (n, t)) in ps.iter().enumerate() {
                if given[i].is_none() {
                    if matches!(t, Type::Option(_)) {
                        given[i] = Some(Expr {
                            kind: ExprKind::None,
                            span: e.span,
                        });
                    } else {
                        missing.push(format!("{n}({t})"));
                    }
                }
            }
            if !missing.is_empty() {
                diags.push(Diagnostic::error(
                    Code::TYP001,
                    e.span,
                    format!("`{}` の引数 {} がありません(省けるのは 選択<…> の項目だけ)", callee.name, missing.join(", ")),
                ));
                ok = false;
            }
            if ok {
                *args = given.into_iter().map(|x| x.unwrap()).collect();
            }
        }
        ExprKind::Named { value, .. } => expr(env, value, diags),
        ExprKind::Field(x, _) => expr(env, x, diags),
        ExprKind::Unary { expr: x, .. } => expr(env, x, diags),
        ExprKind::Binary { lhs, rhs, .. } => {
            expr(env, lhs, diags);
            expr(env, rhs, diags);
        }
        ExprKind::List(xs) => xs.iter_mut().for_each(|x| expr(env, x, diags)),
        _ => {}
    }
}

fn block(env: &Env, b: &mut Block, diags: &mut Vec<Diagnostic>) {
    for st in &mut b.stmts {
        match st {
            Stmt::Check { cond, fail, .. } => {
                expr(env, cond, diags);
                expr(env, fail, diags);
            }
            Stmt::Return { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr { expr: value, .. } => expr(env, value, diags),
            Stmt::If { branches, else_block, .. } => {
                for (c, bb) in branches {
                    expr(env, c, diags);
                    block(env, bb, diags);
                }
                if let Some(eb) = else_block {
                    block(env, eb, diags);
                }
            }
            Stmt::Loop { iter, body, .. } => {
                expr(env, iter, diags);
                block(env, body, diags);
            }
            Stmt::Transaction { body, .. } => block(env, body, diags),
        }
    }
}

/// すべての動作・関数の本体の名前付き引数を、宣言の順の引数に直す
pub fn resolve_named_args(env: &Env, files: &mut [SourceFile]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for f in files {
        for it in &mut f.items {
            match &mut it.decl {
                Decl::Action(a) => {
                    for item in &mut a.items {
                        if let ActionItem::Strict(b) = item {
                            block(env, b, &mut diags);
                        }
                    }
                }
                Decl::Function(fd) => block(env, &mut fd.body, &mut diags),
                _ => {}
            }
        }
    }
    diags
}
