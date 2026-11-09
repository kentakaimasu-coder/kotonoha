//! 厳密層 → TypeScript(Backend 用)。型検査済みの AST だけを入力にする。
//!
//! 実行意味論(docs/architecture.md §5):
//! - 整数・金額は BigInt(`rt.add` 等で 64 ビット範囲を検査)
//! - `を確認 または 失敗(e)` は `throw new rt.Failure(e)`。取引中ならロールバックされる
//! - `取引:` は `db.tx(ctx, () => …)`。中での `を返す` は `{ v }` に包んで外へ伝える
//! - エンティティのフィールド更新は `db.markDirty` で記録し、取引の終わりに UPDATE する

use crate::appir::AppIr;
use kotonoha_ast::*;
use kotonoha_diag::{Code, Diagnostic};
use std::collections::BTreeSet;

/// 識別子を JS 識別子として安全な形にする(英字・ASCII 数字・`_` 以外は `_uXXXX`)。
pub fn mangle(prefix: &str, name: &str) -> String {
    let mut s = String::from(prefix);
    for c in name.chars() {
        if c.is_alphabetic() || c.is_ascii_digit() || c == '_' {
            s.push(c);
        } else {
            s.push_str(&format!("_u{:04X}", c as u32));
        }
    }
    s
}

pub fn js_str(s: &str) -> String {
    serde_json::to_string(s).expect("文字列は直列化できる")
}

pub struct TsGen<'a> {
    pub ir: &'a AppIr,
    out: String,
    indent: usize,
    scopes: Vec<BTreeSet<String>>,
    in_tx: bool,
    /// `関数`(同期・ctx なし)の本体を生成中
    pure: bool,
    pub errors: Vec<Diagnostic>,
}

impl<'a> TsGen<'a> {
    pub fn new(ir: &'a AppIr) -> Self {
        TsGen {
            ir,
            out: String::new(),
            indent: 0,
            scopes: Vec::new(),
            in_tx: false,
            pure: false,
            errors: Vec::new(),
        }
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn declared(&self, n: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }

    /// 動作 1 つ分の関数を生成する。
    /// `auto_tx`: データを変える動作は、本体全体を 1 つの取引にする(`取引:` を書かなくても、失敗したら全部元に戻る)
    pub fn action(&mut self, name: &str, params: &[(String, kotonoha_types::Type)], body: Option<&Block>, auto_tx: bool) -> String {
        self.out.clear();
        let ps: Vec<String> = params.iter().map(|(n, _)| format!("{}: any", mangle("v_", n))).collect();
        self.line(&format!(
            "export async function {}(ctx: Ctx{}{}): Promise<any> {{",
            mangle("act_", name),
            if ps.is_empty() { "" } else { ", " },
            ps.join(", ")
        ));
        self.indent += 1;
        self.scopes.push(params.iter().map(|(n, _)| n.clone()).collect());
        if let Some(b) = body {
            if auto_tx {
                self.line("const __t: any = await db.tx(ctx, async () => {");
                self.indent += 1;
                self.in_tx = true;
                self.block_stmts(b);
                self.in_tx = false;
                self.line("return undefined;");
                self.indent -= 1;
                self.line("});");
                self.line("if (__t !== undefined) return __t.v;");
            } else {
                self.block_stmts(b);
            }
        }
        self.line("return undefined;");
        self.scopes.pop();
        self.indent -= 1;
        self.line("}");
        std::mem::take(&mut self.out)
    }

    /// `関数` 1 つ分(副作用なし・同期)を生成する。
    pub fn function(&mut self, name: &str, params: &[(String, kotonoha_types::Type)], body: &Block) -> String {
        self.out.clear();
        self.pure = true;
        let ps: Vec<String> = params.iter().map(|(n, _)| format!("{}: any", mangle("v_", n))).collect();
        self.line(&format!("export function {}({}): any {{", mangle("fn_", name), ps.join(", ")));
        self.indent += 1;
        self.scopes.push(params.iter().map(|(n, _)| n.clone()).collect());
        self.block_stmts(body);
        self.line("throw new Error(\"unreachable\");");
        self.scopes.pop();
        self.indent -= 1;
        self.line("}");
        self.pure = false;
        std::mem::take(&mut self.out)
    }

    fn block_stmts(&mut self, b: &Block) {
        self.scopes.push(BTreeSet::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Check { cond, fail, .. } => {
                let arg = match &fail.kind {
                    ExprKind::Call { args, .. } if args.len() == 1 => self.expr(&args[0]),
                    _ => self.expr(fail),
                };
                let c = self.expr(cond);
                self.line(&format!("if (!({c})) throw new rt.Failure({arg});"));
            }
            Stmt::Return { value, .. } => {
                let v = self.expr(value);
                if self.in_tx {
                    self.line(&format!("return {{ v: {v} }};"));
                } else {
                    self.line(&format!("return {v};"));
                }
            }
            Stmt::If { branches, else_block, .. } => {
                for (i, (c, b)) in branches.iter().enumerate() {
                    let cs = self.expr(c);
                    self.line(&format!("{}if ({cs}) {{", if i == 0 { "" } else { "} else " }));
                    self.indent += 1;
                    self.block_stmts(b);
                    self.indent -= 1;
                }
                if let Some(b) = else_block {
                    self.line("} else {");
                    self.indent += 1;
                    self.block_stmts(b);
                    self.indent -= 1;
                }
                self.line("}");
            }
            Stmt::Loop { var, iter, body, .. } => {
                let it = self.expr(iter);
                self.line(&format!("for (const {} of {it}) {{", mangle("v_", &var.name)));
                self.indent += 1;
                self.scopes.push([var.name.clone()].into_iter().collect());
                self.block_stmts(body);
                self.scopes.pop();
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Transaction { body, .. } if self.in_tx => {
                // 動作全体が既に取引(自動)なので、そのまま書く
                self.line("{");
                self.indent += 1;
                self.block_stmts(body);
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Transaction { body, .. } => {
                self.line("{");
                self.indent += 1;
                self.line("const __t: any = await db.tx(ctx, async () => {");
                self.indent += 1;
                self.in_tx = true;
                self.block_stmts(body);
                self.in_tx = false;
                self.line("return undefined;");
                self.indent -= 1;
                self.line("});");
                self.line("if (__t !== undefined) return __t.v;");
                self.indent -= 1;
                self.line("}");
            }
            Stmt::Assign { target, value, .. } => {
                let v = self.expr(value);
                let root = mangle("v_", &target.root.name);
                if target.fields.is_empty() {
                    if self.declared(&target.root.name) {
                        self.line(&format!("{root} = {v};"));
                    } else {
                        self.line(&format!("let {root}: any = {v};"));
                        self.scopes.last_mut().unwrap().insert(target.root.name.clone());
                    }
                } else {
                    let f = js_str(&target.fields[0].name);
                    self.line(&format!("{root}[{f}] = {v};"));
                    if !self.pure {
                        self.line(&format!("db.markDirty(ctx, {root}, {f});"));
                    }
                }
            }
            Stmt::Expr { expr, .. } => {
                let e = self.expr(expr);
                self.line(&format!("{e};"));
            }
        }
    }

    pub fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => format!("{v}n"),
            ExprKind::Decimal(_) => {
                self.errors.push(Diagnostic::error(
                    Code::TGT001,
                    e.span,
                    "小数は精度方針が未決定のため Backend 生成に未対応です(ADR-0106)",
                ));
                "undefined".into()
            }
            ExprKind::Str(s) => js_str(s),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::None => "null".into(),
            ExprKind::Ident(n) if n == kotonoha_types::check::PRINCIPAL_WORD && !self.declared(n) => "db.principalId(ctx)".into(),
            ExprKind::Ident(n) => mangle("v_", n),
            ExprKind::Field(b, f) => format!("{}[{}]", self.expr(b), js_str(&f.name)),
            ExprKind::Call { callee, args } => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x)).collect();
                match callee.name.as_str() {
                    "成功" => a.into_iter().next().unwrap_or_else(|| "undefined".into()),
                    "失敗" => format!("(() => {{ throw new rt.Failure({}); }})()", a.join(", ")),
                    n if self.ir.functions.iter().any(|f| f.name == n) => format!("{}({})", mangle("fn_", n), a.join(", ")),
                    n if self.ir.ext.functions.iter().any(|f| f.name == n) => {
                        format!("(await {}(ctx{}{}))", mangle("ex_", n), if a.is_empty() { "" } else { ", " }, a.join(", "))
                    }
                    "長さ" => format!("BigInt([...({})].length)", a[0]),
                    "件数" => format!("BigInt(({}).length)", a[0]),
                    "絶対値" => format!("rt.abs({})", a[0]),
                    "最大" => format!("rt.max({}, {})", a[0], a[1]),
                    "最小" => format!("rt.min({}, {})", a[0], a[1]),
                    "文字列化" => format!("String({})", a[0]),
                    "含む" => format!("({}).includes({})", a[0], a[1]),
                    "現在日時" => "BigInt(Date.now())".into(),
                    "今日" => "rt.today()".into(),
                    "日後" => format!("rt.addDays({}, {})", a[0], a[1]),
                    "日前" => format!("rt.addDays({}, -({}))", a[0], a[1]),
                    "日数差" => format!("rt.daysBetween({}, {})", a[0], a[1]),
                    n if self.ir.adapters.iter().any(|x| x.name == n) => {
                        format!("(await {}(ctx{}{}))", mangle("ad_", n), if a.is_empty() { "" } else { ", " }, a.join(", "))
                    }
                    n => format!("(await {}(ctx{}{}))", mangle("act_", n), if a.is_empty() { "" } else { ", " }, a.join(", ")),
                }
            }
            ExprKind::Unary { op, expr } => {
                let x = self.expr(expr);
                match op {
                    UnaryOp::Neg => format!("rt.neg({x})"),
                    UnaryOp::Not => format!("!({x})"),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let (l, r) = (self.expr(lhs), self.expr(rhs));
                match op {
                    BinOp::Add => format!("rt.add({l}, {r})"),
                    BinOp::Sub => format!("rt.sub({l}, {r})"),
                    BinOp::Mul => format!("rt.mul({l}, {r})"),
                    BinOp::Div => format!("rt.div({l}, {r})"),
                    BinOp::Eq => format!("rt.eq({l}, {r})"),
                    BinOp::Ne => format!("!rt.eq({l}, {r})"),
                    BinOp::Lt => format!("({l} < {r})"),
                    BinOp::Le => format!("({l} <= {r})"),
                    BinOp::Gt => format!("({l} > {r})"),
                    BinOp::Ge => format!("({l} >= {r})"),
                    BinOp::And => format!("({l} && {r})"),
                    BinOp::Or => format!("({l} || {r})"),
                }
            }
            // 名前付き引数は解析の段階で並べ替えて取り除かれる(ADR-0134)
            ExprKind::Named { value, .. } => self.expr(value),
            ExprKind::List(xs) => {
                let a: Vec<String> = xs.iter().map(|x| self.expr(x)).collect();
                format!("[{}]", a.join(", "))
            }
        }
    }
}
