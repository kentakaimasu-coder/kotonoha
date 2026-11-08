//! 厳密層の型検査(技術仕様書 §3.2/§4)。AI を一切呼ばない決定的な検査。
//!
//! 検査と同時に、効果検査(validator)が使う「事実(Facts)」を収集する:
//! どのアダプタ・動作を呼んだか、どのエンティティのどのフィールドを更新したか。

use crate::env::Env;
use crate::ty::Type;
use kotonoha_ast::*;
use kotonoha_diag::{Code, Diagnostic, Span};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    pub adapter_calls: Vec<(String, Span)>,
    pub action_calls: Vec<(String, Span)>,
    /// (データ名, フィールド名, 位置)
    pub mutations: Vec<(String, String, Span)>,
    /// `本人` を参照したか
    pub uses_principal: bool,
    /// 呼んだ `関数`
    pub function_calls: Vec<(String, Span)>,
    /// 呼んだ `外部関数`
    pub external_calls: Vec<(String, Span)>,
}

/// 組込み関数(既存言語の基本操作。ADR-0124)。(名前, 説明)
pub const BUILTINS: &[(&str, &str)] = &[
    ("長さ", "長さ(文字列) -> 整数: 文字数(Unicode のスカラー値の数)"),
    ("件数", "件数(一覧<T>) -> 整数"),
    ("絶対値", "絶対値(整数) -> 整数"),
    ("最大", "最大(整数, 整数) -> 整数"),
    ("最小", "最小(整数, 整数) -> 整数"),
    ("文字列化", "文字列化(整数 | 金額) -> 文字列: 明示的な変換"),
    ("含む", "含む(文字列, 文字列) -> 真偽: 部分文字列を含むか"),
    ("現在日時", "現在日時() -> 日時: 動作の中だけで使える(関数では使えない)"),
];

#[derive(Debug, Clone)]
struct Local {
    ty: Type,
    mutable: bool,
    /// アダプタから取得したエンティティ(フィールド更新が永続化される)
    persisted: bool,
}

pub struct Checker<'e> {
    env: &'e Env,
    scopes: Vec<BTreeMap<String, Local>>,
    pub diags: Vec<Diagnostic>,
    pub facts: Facts,
    ret: Option<Type>,
    in_tx: u32,
    /// 組込み値 `本人`(現在の主体 ID)を使えるか。動作の中だけ(ADR-0119)
    allow_principal: bool,
    /// `関数` の本体を検査中(副作用・時刻・本人を禁止する)
    in_pure_fn: bool,
}

/// 組込み値: 現在の認証済み主体の ID(識別子)。動作の中でだけ使える。
pub const PRINCIPAL_WORD: &str = "本人";

/// 動作を検査する。
pub fn check_action(env: &Env, a: &ActionDecl) -> (Facts, Vec<Diagnostic>) {
    let sig = env.actions.get(&a.name.name);
    let mut c = Checker::new(env);
    c.allow_principal = true;
    c.ret = sig.and_then(|s| s.ret.clone());
    let mut top = BTreeMap::new();
    if let Some(sig) = sig {
        for (n, t) in &sig.params {
            top.insert(
                n.clone(),
                Local {
                    ty: t.clone(),
                    mutable: false,
                    persisted: false,
                },
            );
        }
    }
    c.scopes.push(top);
    let blocks: Vec<&Block> = a.strict_blocks().collect();
    if blocks.len() > 1 {
        c.diags
            .push(Diagnostic::error(Code::SYN001, blocks[1].span, "`厳密:` ブロックは動作ごとに 1 つです"));
    }
    match blocks.first() {
        Some(b) => {
            c.block(b);
            if c.ret.is_some() && !terminates(b) {
                c.diags.push(
                    Diagnostic::error(Code::TYP003, b.span, format!("動作 `{}` は全ての経路で値を返していません", a.name.name))
                        .hint("ブロックの最後に `値 を返す` を書いてください"),
                );
            }
        }
        None => {
            if let Some(r) = &c.ret {
                c.diags.push(Diagnostic::error(
                    Code::TYP003,
                    a.span,
                    format!("戻り値の型 {r} がありますが `厳密:` ブロックがありません"),
                ));
            }
        }
    }
    (c.facts, c.diags)
}

/// `関数` を検査する(副作用・`本人`・`現在日時`・取引を禁止)。
pub fn check_function(env: &Env, f: &FunctionDecl) -> (Facts, Vec<Diagnostic>) {
    let mut c = Checker::new(env);
    c.in_pure_fn = true;
    let sig = env.functions.get(&f.name.name);
    c.ret = sig.map(|s| s.ret.clone());
    let mut top = BTreeMap::new();
    if let Some(sig) = sig {
        for (n, t) in &sig.params {
            top.insert(
                n.clone(),
                Local {
                    ty: t.clone(),
                    mutable: false,
                    persisted: false,
                },
            );
        }
    }
    c.scopes.push(top);
    c.block(&f.body);
    if !terminates(&f.body) {
        c.diags.push(Diagnostic::error(
            Code::TYP003,
            f.body.span,
            format!("関数 `{}` は全ての経路で値を返していません", f.name.name),
        ));
    }
    if contains_tx(&f.body) {
        c.diags.push(Diagnostic::error(Code::SEC001, f.span, "関数の中で `取引:` は使えません"));
    }
    (c.facts, c.diags)
}

fn contains_tx(b: &Block) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Transaction { .. } => true,
        Stmt::If { branches, else_block, .. } => branches.iter().any(|(_, b)| contains_tx(b)) || else_block.as_ref().is_some_and(contains_tx),
        Stmt::Loop { body, .. } => contains_tx(body),
        _ => false,
    })
}

/// 任意のスコープで式を検査する(画面意図のボタン引数・知識の条件式で使う)。
pub fn check_expr_in(env: &Env, vars: &[(String, Type)], e: &Expr, expected: Option<&Type>) -> (Type, Facts, Vec<Diagnostic>) {
    let mut c = Checker::new(env);
    let mut top = BTreeMap::new();
    for (n, t) in vars {
        top.insert(
            n.clone(),
            Local {
                ty: t.clone(),
                mutable: false,
                persisted: false,
            },
        );
    }
    c.scopes.push(top);
    let t = c.expr(e, expected);
    (t, c.facts, c.diags)
}

fn terminates(b: &Block) -> bool {
    match b.stmts.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::Transaction { body, .. }) => terminates(body),
        Some(Stmt::If {
            branches,
            else_block: Some(eb),
            ..
        }) => branches.iter().all(|(_, b)| terminates(b)) && terminates(eb),
        _ => false,
    }
}

impl<'e> Checker<'e> {
    pub fn new(env: &'e Env) -> Self {
        Checker {
            env,
            scopes: Vec::new(),
            diags: Vec::new(),
            facts: Facts::default(),
            ret: None,
            in_tx: 0,
            allow_principal: false,
            in_pure_fn: false,
        }
    }

    fn err(&mut self, code: Code, span: Span, msg: impl Into<String>) -> Diagnostic {
        Diagnostic::error(code, span, msg)
    }

    fn lookup(&self, n: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|s| s.get(n))
    }

    fn mismatch(&mut self, span: Span, want: &Type, got: &Type) {
        if want.is_error() || got.is_error() {
            return;
        }
        let d = self.err(Code::TYP001, span, format!("型が一致しません: {want} が必要ですが {got} です"));
        let d = match (want, got) {
            (Type::Str, Type::Int | Type::Decimal | Type::Money(_)) => d.hint("数値から文字列への暗黙変換はしません"),
            (Type::Decimal, Type::Int) | (Type::Int, Type::Decimal) => d.hint("整数と小数の暗黙変換はしません"),
            (Type::Money(a), Type::Money(b)) if a != b => d.hint("通貨の異なる金額は混在できません"),
            _ => d,
        };
        self.diags.push(d);
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(BTreeMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Check { cond, fail, span } => {
                self.expect(cond, &Type::Bool);
                match self.ret.clone() {
                    Some(Type::Result(_, e)) => {
                        if let ExprKind::Call { args, .. } = &fail.kind {
                            if let Some(a) = args.first() {
                                self.expect(a, &e);
                            }
                        }
                    }
                    _ => {
                        let d = self.err(Code::TYP001, *span, "`を確認 または 失敗(...)` は戻り値が 結果<T, E> の動作でのみ使えます");
                        self.diags.push(d);
                    }
                }
            }
            Stmt::Return { value, span } => match self.ret.clone() {
                None => {
                    let d = self.err(Code::TYP001, *span, "戻り値の型が宣言されていない動作では値を返せません");
                    self.diags.push(d);
                }
                Some(Type::Result(t, e)) => {
                    // T は 成功(T) に自動で包む(ADR-0102)
                    let got = self.expr(value, Some(&t));
                    let full = Type::Result(t.clone(), e);
                    if !got.same(&t) && !got.same(&full) {
                        self.mismatch(value.span, &t, &got);
                    }
                }
                Some(r) => self.expect(value, &r),
            },
            Stmt::If { branches, else_block, .. } => {
                for (c, b) in branches {
                    self.expect(c, &Type::Bool);
                    self.block(b);
                }
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            Stmt::Loop { var, iter, body, .. } => {
                let it = self.expr(iter, None);
                let elem = match it {
                    Type::List(t) | Type::Array(t) => *t,
                    Type::Error => Type::Error,
                    other => {
                        let d = self.err(Code::TYP001, iter.span, format!("繰り返しの対象は 一覧 か 配列 ですが {other} です"));
                        self.diags.push(d);
                        Type::Error
                    }
                };
                let mut sc = BTreeMap::new();
                sc.insert(
                    var.name.clone(),
                    Local {
                        ty: elem,
                        mutable: false,
                        persisted: false,
                    },
                );
                self.scopes.push(sc);
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Transaction { body, span } => {
                if self.in_tx > 0 {
                    let d = self.err(Code::TYP002, *span, "入れ子の `取引:` は未対応です(ADR-0102)");
                    self.diags.push(d);
                }
                self.in_tx += 1;
                self.block(body);
                self.in_tx -= 1;
            }
            Stmt::Assign { target, value, span } => self.assign(target, value, *span),
            Stmt::Expr { expr, span } => {
                let t = self.expr(expr, None);
                if matches!(t, Type::Result(..)) {
                    self.diags.push(Diagnostic::warning(Code::TYP001, *span, "結果<T, E> を無視しています"));
                }
            }
        }
    }

    fn assign(&mut self, target: &Place, value: &Expr, span: Span) {
        let name = &target.root.name;
        if target.fields.is_empty() {
            if name == PRINCIPAL_WORD {
                let d = self.err(Code::TYP002, target.root.span, "`本人` は組込みの値なので代入できません");
                self.diags.push(d);
                return;
            }
            if self.env.adapters.contains_key(name) || self.env.actions.contains_key(name) {
                let d = self.err(
                    Code::REF002,
                    target.root.span,
                    format!("`{name}` はアダプタ/動作の名前なので変数名に使えません"),
                );
                self.diags.push(d);
                return;
            }
            match self.lookup(name).cloned() {
                Some(l) => {
                    if !l.mutable {
                        let d = self.err(
                            Code::TYP002,
                            target.root.span,
                            format!("`{name}` は引数または繰り返し変数なので再代入できません"),
                        );
                        self.diags.push(d);
                        return;
                    }
                    self.expect(value, &l.ty);
                }
                None => {
                    let t = self.expr(value, None);
                    if matches!(t, Type::Unit) {
                        let d = self.err(Code::TYP001, value.span, "値を返さない呼出は代入できません");
                        self.diags.push(d);
                    }
                    let persisted = matches!(&value.kind, ExprKind::Call { callee, .. }
                        if self.env.adapters.get(&callee.name).is_some_and(|a| a.op == OpKind::Get));
                    self.scopes.last_mut().unwrap().insert(
                        name.clone(),
                        Local {
                            ty: t,
                            mutable: true,
                            persisted,
                        },
                    );
                }
            }
            return;
        }
        // フィールド更新
        let Some(l) = self.lookup(name).cloned() else {
            let d = self.err(Code::REF001, target.root.span, format!("`{name}` は定義されていません"));
            self.diags.push(d);
            return;
        };
        if target.fields.len() > 1 {
            let d = self.err(Code::TYP002, span, "入れ子のフィールド更新は未対応です");
            self.diags.push(d);
            return;
        }
        let field = &target.fields[0];
        let Type::Data(dname) = &l.ty else {
            if !l.ty.is_error() {
                let d = self.err(
                    Code::TYP002,
                    span,
                    format!("{} のフィールドは更新できません(更新できるのはデータのフィールドだけです)", l.ty),
                );
                self.diags.push(d);
            }
            return;
        };
        let Some(ft) = self.env.data.get(dname).and_then(|d| d.field(&field.name)).cloned() else {
            let d = self.err(Code::REF001, field.span, format!("データ `{dname}` にフィールド `{}` はありません", field.name));
            self.diags.push(d);
            return;
        };
        if field.name == "ID" {
            let d = self.err(Code::TYP002, field.span, "ID フィールドは更新できません");
            self.diags.push(d);
        }
        if !l.persisted {
            let d = self
                .err(Code::TYP002, span, format!("`{name}` は保存元が分からないため更新できません"))
                .hint("アダプタの 取得 で得たエンティティだけを更新できます");
            self.diags.push(d);
        }
        if self.in_tx == 0 {
            let d = self.err(Code::TYP002, span, "エンティティの更新は `取引:` の中でだけ行えます");
            self.diags.push(d);
        }
        self.expect(value, &ft);
        self.facts.mutations.push((dname.clone(), field.name.clone(), span));
    }

    fn expect(&mut self, e: &Expr, want: &Type) {
        let got = self.expr(e, Some(want));
        if !got.same(want) {
            self.mismatch(e.span, want, &got);
        }
    }

    /// 式の型を求める。`expected` はリテラルの型付け(文字列→列挙、なし→選択)にだけ使う。
    pub fn expr(&mut self, e: &Expr, expected: Option<&Type>) -> Type {
        match &e.kind {
            // 金額が期待される所の整数リテラルは、その通貨の金額(例: `価格 > 0`、`3000 を返す`)
            ExprKind::Int(_) => match expected {
                Some(Type::Money(c)) => Type::Money(c.clone()),
                Some(Type::Option(t)) if matches!(t.as_ref(), Type::Money(_)) => expected.unwrap().clone(),
                _ => self.lift(Type::Int, expected),
            },
            ExprKind::Decimal(_) => self.lift(Type::Decimal, expected),
            ExprKind::Bool(_) => self.lift(Type::Bool, expected),
            ExprKind::Str(s) => {
                let base = match expected {
                    Some(Type::Option(t)) => Some(t.as_ref()),
                    other => other,
                };
                if let Some(Type::Enum(en)) = base {
                    if self.env.enum_has(en, s) {
                        return expected.unwrap().clone();
                    }
                    let vs = self.env.enums.get(en).map(|v| v.join(" / ")).unwrap_or_default();
                    let d = self
                        .err(Code::TYP001, e.span, format!("\"{s}\" は列挙 {en} の値ではありません"))
                        .hint(format!("使える値: {vs}"));
                    self.diags.push(d);
                    return Type::Error;
                }
                self.lift(Type::Str, expected)
            }
            ExprKind::None => match expected {
                Some(t @ Type::Option(_)) => t.clone(),
                _ => {
                    let d = self.err(Code::TYP001, e.span, "`なし` は 選択<T> 型が期待される位置でだけ使えます");
                    self.diags.push(d);
                    Type::Error
                }
            },
            ExprKind::Ident(n) => match self.lookup(n) {
                Some(l) => {
                    let t = l.ty.clone();
                    self.lift(t, expected)
                }
                None if n == PRINCIPAL_WORD && self.allow_principal => {
                    self.facts.uses_principal = true;
                    self.lift(Type::Id, expected)
                }
                None => {
                    let mut d = self.err(Code::REF001, e.span, format!("`{n}` は定義されていません"));
                    if let Some(kw) = kotonoha_lexer_suffix(n) {
                        d = d.hint(format!("`{} {kw}` のように空白で区切る必要があるかもしれません", &n[..n.len() - kw.len()]));
                    } else if let Some((en, _)) = self.env.enums.iter().find(|(_, vs)| vs.iter().any(|v| v == n)) {
                        d = d.hint(format!("列挙 {en} の値は文字列で書きます: \"{n}\""));
                    } else if let Some(vs) = self.env.enums.get(n) {
                        // `返事.参加` のように書いた場合(ペルソナ評価で多かった間違い)
                        d = d.hint(format!(
                            "`{n}` は列挙の型名です。列挙の値は文字列で書きます(例: \"{}\")。使える値: {}",
                            vs.first().cloned().unwrap_or_default(),
                            vs.join(" / ")
                        ));
                    } else if self.env.is_type_name(n) {
                        d = d.hint(format!("`{n}` は型名です。値として使うには変数を定義してください"));
                    }
                    self.diags.push(d);
                    Type::Error
                }
            },
            ExprKind::Field(base, f) => {
                let bt = self.expr(base, None);
                let t = match &bt {
                    Type::Data(dn) => match self.env.data.get(dn).and_then(|d| d.field(&f.name)) {
                        Some(t) => t.clone(),
                        None => {
                            let d = self.err(Code::REF001, f.span, format!("データ `{dn}` にフィールド `{}` はありません", f.name));
                            self.diags.push(d);
                            Type::Error
                        }
                    },
                    Type::Record(fs) => match fs.iter().find(|(n, _)| n == &f.name) {
                        Some((_, t)) => t.clone(),
                        None => {
                            let d = self.err(Code::REF001, f.span, format!("記録にフィールド `{}` はありません", f.name));
                            self.diags.push(d);
                            Type::Error
                        }
                    },
                    Type::Error => Type::Error,
                    Type::Option(_) => {
                        let d = self.err(Code::TYP001, base.span, format!("{bt} の中身は存在確認なしに参照できません"));
                        self.diags.push(d);
                        Type::Error
                    }
                    other => {
                        let d = self.err(Code::TYP001, base.span, format!("{other} にはフィールドがありません"));
                        self.diags.push(d);
                        Type::Error
                    }
                };
                self.lift(t, expected)
            }
            ExprKind::Call { callee, args } => self.call(callee, args, e.span, expected),
            ExprKind::Unary { op, expr } => {
                let t = self.expr(expr, None);
                match op {
                    UnaryOp::Not => {
                        if !t.same(&Type::Bool) {
                            self.mismatch(expr.span, &Type::Bool, &t);
                        }
                        Type::Bool
                    }
                    UnaryOp::Neg => {
                        if !t.is_numeric() && !t.is_error() {
                            let d = self.err(Code::TYP001, expr.span, format!("{t} に符号反転は使えません"));
                            self.diags.push(d);
                            return Type::Error;
                        }
                        t
                    }
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs, e.span),
            ExprKind::List(items) => {
                let want_elem = match expected {
                    Some(Type::List(t)) | Some(Type::Array(t)) => Some(t.as_ref().clone()),
                    _ => None,
                };
                let mut elem: Option<Type> = want_elem.clone();
                for it in items {
                    match &elem {
                        Some(t) => {
                            let t = t.clone();
                            self.expect(it, &t)
                        }
                        None => elem = Some(self.expr(it, None)),
                    }
                }
                match (elem, expected) {
                    (Some(t), Some(Type::Array(_))) => Type::Array(Box::new(t)),
                    (Some(t), _) => Type::List(Box::new(t)),
                    (None, _) => {
                        let d = self.err(Code::TYP001, e.span, "空の一覧の要素型が決まりません");
                        self.diags.push(d);
                        Type::Error
                    }
                }
            }
        }
    }

    /// T を 選択<T> が期待される位置で受け入れる。
    fn lift(&self, t: Type, expected: Option<&Type>) -> Type {
        if let Some(Type::Option(inner)) = expected {
            if t.same(inner) {
                return Type::Option(inner.clone());
            }
        }
        t
    }

    fn call(&mut self, callee: &Ident, args: &[Expr], span: Span, expected: Option<&Type>) -> Type {
        let name = callee.name.as_str();
        if name == "失敗" || name == "成功" {
            if name == "失敗" {
                let d = self.err(Code::TYP001, span, "`失敗(...)` は `を確認 または 失敗(...)` の中でだけ使えます");
                self.diags.push(d);
                return Type::Error;
            }
            // 成功(x): 期待型が 結果<T,E> のときだけ型が決まる
            if let Some(Type::Result(t, _)) = expected {
                if args.len() != 1 {
                    let d = self.err(Code::TYP001, span, "`成功(...)` の引数は 1 つです");
                    self.diags.push(d);
                    return Type::Error;
                }
                self.expect(&args[0], t);
                return expected.unwrap().clone();
            }
            let d = self.err(Code::TYP001, span, "`成功(...)` は 結果<T, E> 型が期待される位置でだけ使えます");
            self.diags.push(d);
            return Type::Error;
        }
        if let Some(t) = self.builtin(name, args, span) {
            return t;
        }
        if let Some(f) = self.env.functions.get(name).cloned() {
            self.facts.function_calls.push((name.to_string(), callee.span));
            return self.check_args(name, &f.params, args, span, f.ret);
        }
        if let Some(x) = self.env.externals.get(name).cloned() {
            if x.kind != ExternalKind::Function {
                let d = self.err(
                    Code::TYP001,
                    callee.span,
                    format!("{} `{name}` は呼び出せません(画面で `{name}を表示` として使います)", x.kind.word()),
                );
                self.diags.push(d);
                return Type::Error;
            }
            if self.in_pure_fn {
                let d = self.err(
                    Code::SEC001,
                    callee.span,
                    format!("関数の中で外部関数 `{name}` は呼べません(関数は副作用を持てません)"),
                );
                self.diags.push(d);
            }
            self.facts.external_calls.push((name.to_string(), callee.span));
            return self.check_args(name, &x.params, args, span, x.ret.unwrap_or(Type::Error));
        }
        if self.in_pure_fn && (self.env.adapters.contains_key(name) || self.env.actions.contains_key(name)) {
            let d = self.err(
                Code::SEC001,
                callee.span,
                format!("関数の中で `{name}` は呼べません(関数は副作用を持てません。動作で呼んでください)"),
            );
            self.diags.push(d);
        }
        let (params, ret, kind) = if let Some(a) = self.env.adapters.get(name) {
            (a.params.clone(), a.ret.clone(), 0)
        } else if let Some(a) = self.env.actions.get(name) {
            (a.params.clone(), a.ret.clone().unwrap_or(Type::Unit), 1)
        } else {
            let mut d = self.err(Code::REF001, callee.span, format!("呼出先 `{name}` は定義されていません"));
            d = d.hint("データ操作は `アダプタ` 宣言で定義します(標準提供の操作はありません)");
            self.diags.push(d);
            for a in args {
                self.expr(a, None);
            }
            return Type::Error;
        };
        if kind == 0 {
            self.facts.adapter_calls.push((name.to_string(), callee.span));
        } else {
            self.facts.action_calls.push((name.to_string(), callee.span));
        }
        if params.len() != args.len() {
            let d = self.err(
                Code::TYP001,
                span,
                format!("`{name}` の引数は {} 個ですが {} 個渡されています", params.len(), args.len()),
            );
            self.diags.push(d);
            for a in args {
                self.expr(a, None);
            }
            return ret;
        }
        for (a, (_, pt)) in args.iter().zip(params.iter()) {
            self.expect(a, pt);
        }
        ret
    }

    fn check_args(&mut self, name: &str, params: &[(String, Type)], args: &[Expr], span: Span, ret: Type) -> Type {
        if params.len() != args.len() {
            let d = self.err(
                Code::TYP001,
                span,
                format!("`{name}` の引数は {} 個ですが {} 個渡されています", params.len(), args.len()),
            );
            self.diags.push(d);
            for a in args {
                self.expr(a, None);
            }
            return ret;
        }
        for (a, (_, pt)) in args.iter().zip(params) {
            self.expect(a, pt);
        }
        ret
    }

    /// 組込み関数。該当しなければ None。
    fn builtin(&mut self, name: &str, args: &[Expr], span: Span) -> Option<Type> {
        if !BUILTINS.iter().any(|(n, _)| *n == name) || self.env.functions.contains_key(name) || self.env.externals.contains_key(name) {
            return None;
        }
        let arity = |n: usize, s: &mut Self| -> bool {
            if args.len() != n {
                let d = s.err(Code::TYP001, span, format!("`{name}` の引数は {n} 個です"));
                s.diags.push(d);
                false
            } else {
                true
            }
        };
        Some(match name {
            "長さ" => {
                if arity(1, self) {
                    self.expect(&args[0], &Type::Str);
                }
                Type::Int
            }
            "件数" => {
                if arity(1, self) {
                    let t = self.expr(&args[0], None);
                    if !matches!(t, Type::List(_) | Type::Array(_) | Type::Error) {
                        self.mismatch(args[0].span, &Type::List(Box::new(Type::Error)), &t);
                    }
                }
                Type::Int
            }
            "絶対値" => {
                if arity(1, self) {
                    self.expect(&args[0], &Type::Int);
                }
                Type::Int
            }
            "最大" | "最小" => {
                if arity(2, self) {
                    self.expect(&args[0], &Type::Int);
                    self.expect(&args[1], &Type::Int);
                }
                Type::Int
            }
            "文字列化" => {
                if arity(1, self) {
                    let t = self.expr(&args[0], None);
                    if !matches!(t, Type::Int | Type::Money(_) | Type::Error) {
                        let d = self.err(Code::TYP001, args[0].span, format!("文字列化できるのは 整数 と 金額 です({t})"));
                        self.diags.push(d);
                    }
                }
                Type::Str
            }
            "含む" => {
                if arity(2, self) {
                    self.expect(&args[0], &Type::Str);
                    self.expect(&args[1], &Type::Str);
                }
                Type::Bool
            }
            "現在日時" => {
                arity(0, self);
                if self.in_pure_fn {
                    let d = self.err(Code::SEC001, span, "関数の中で `現在日時()` は使えません(結果が呼ぶたびに変わるため)");
                    self.diags.push(d);
                }
                Type::DateTime
            }
            _ => return None,
        })
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) -> Type {
        use BinOp::*;
        match op {
            And | Or => {
                self.expect(lhs, &Type::Bool);
                self.expect(rhs, &Type::Bool);
                Type::Bool
            }
            Eq | Ne => {
                // 列挙と文字列リテラルの比較を許すため、リテラルでない側を先に型付けする
                let (first, second) = if matches!(lhs.kind, ExprKind::Str(_) | ExprKind::None) {
                    (rhs, lhs)
                } else {
                    (lhs, rhs)
                };
                let t1 = self.expr(first, None);
                let t2 = self.expr(second, Some(&t1));
                if !t1.same(&t2) {
                    self.mismatch(second.span, &t1, &t2);
                }
                Type::Bool
            }
            Lt | Le | Gt | Ge => {
                let t1 = self.expr(lhs, None);
                let t2 = self.expr(rhs, Some(&t1));
                let orderable = matches!(
                    t1,
                    Type::Int | Type::Decimal | Type::Money(_) | Type::DateTime | Type::Date | Type::Duration | Type::Error
                );
                if !orderable {
                    let mut d = self.err(Code::TYP001, lhs.span, format!("{t1} は大小比較できません"));
                    if t1 == Type::Str {
                        d = d.hint("文字列の照合順序は未決定のため大小比較は未対応です");
                    }
                    self.diags.push(d);
                } else if !t1.same(&t2) {
                    self.mismatch(rhs.span, &t1, &t2);
                }
                Type::Bool
            }
            Add | Sub | Mul | Div => {
                let t1 = self.expr(lhs, None);
                let t2 = self.expr(rhs, None);
                if t1.is_error() || t2.is_error() {
                    return Type::Error;
                }
                let r = match (op, &t1, &t2) {
                    (_, Type::Int, Type::Int) => Some(Type::Int),
                    (Add | Sub | Mul, Type::Decimal, Type::Decimal) => Some(Type::Decimal),
                    (Div, Type::Decimal, Type::Decimal) => {
                        let d = self.err(Code::TYP001, span, "小数の除算は丸め規則が未決定のため未対応です(ADR-0106)");
                        self.diags.push(d);
                        return Type::Error;
                    }
                    (Add | Sub, Type::Money(a), Type::Money(b)) if a == b => Some(t1.clone()),
                    (Mul, Type::Money(_), Type::Int) => Some(t1.clone()),
                    (Mul, Type::Int, Type::Money(_)) => Some(t2.clone()),
                    (Div, Type::Money(_), _) => {
                        let d = self.err(Code::TYP001, span, "金額の除算は端数処理が未決定のため未対応です(ADR-0106)");
                        self.diags.push(d);
                        return Type::Error;
                    }
                    (Add, Type::Str, Type::Str) => Some(Type::Str),
                    _ => None,
                };
                match r {
                    Some(t) => t,
                    None => {
                        let mut d = self.err(Code::TYP001, span, format!("{t1} {} {t2} は定義されていません", op.symbol()));
                        if let (Type::Money(a), Type::Money(b)) = (&t1, &t2) {
                            if a != b {
                                d = d.hint("通貨の異なる金額は演算できません");
                            }
                        }
                        if matches!((&t1, &t2), (Type::Str, Type::Int | Type::Decimal) | (Type::Int | Type::Decimal, Type::Str)) {
                            d = d.hint("数値から文字列への暗黙変換はしません");
                        }
                        self.diags.push(d);
                        Type::Error
                    }
                }
            }
        }
    }
}

fn kotonoha_lexer_suffix(n: &str) -> Option<&'static str> {
    ["を返す", "を確認"].into_iter().find(|s| n.ends_with(s) && n.len() > s.len())
}
