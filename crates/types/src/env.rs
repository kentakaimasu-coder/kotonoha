//! 宣言の収集と型式の解決。名前空間は「型名」と「呼出名(アダプタ/動作)」と「知識名」に分かれる。

use crate::effects::{Capability, Effect, Verb};
use crate::ty::{is_currency_code, Type, BUILTIN_TYPE_NAMES};
use kotonoha_ast::*;
use kotonoha_diag::{Code, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct DataInfo {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub span: Span,
}

impl DataInfo {
    pub fn field(&self, name: &str) -> Option<&Type> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }
}

#[derive(Debug, Clone)]
pub struct AdapterSig {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub op: OpKind,
    pub target: String,
    pub span: Span,
}

impl AdapterSig {
    /// 操作種別から導出される Capability(ADR-0104)。
    pub fn capability(&self) -> Capability {
        let verb = match self.op {
            OpKind::Get | OpKind::List => Verb::Read,
            OpKind::Create => Verb::Create,
            OpKind::Update => Verb::Update,
            OpKind::Delete => Verb::Delete,
        };
        Capability::new(self.target.clone(), verb)
    }
    pub fn effect(&self) -> Effect {
        self.capability().verb.effect()
    }
}

#[derive(Debug, Clone)]
pub struct ActionSig {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Option<Type>,
    /// `必要 = [...]` が書かれていれば Some
    pub declared_caps: Option<BTreeSet<Capability>>,
    /// `効果 = [...]` が書かれていれば Some(空集合 = pure)
    pub declared_effects: Option<BTreeSet<Effect>>,
    pub span: Span,
}

/// 型検査用の環境。
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub app_name: Option<String>,
    pub aliases: BTreeMap<String, Type>,
    pub enums: BTreeMap<String, Vec<String>>,
    pub data: BTreeMap<String, DataInfo>,
    pub adapters: BTreeMap<String, AdapterSig>,
    pub actions: BTreeMap<String, ActionSig>,
    /// 知識名(本体は knowledge クレートが解釈する)。名前の重複検査のためだけに保持。
    pub knowledge_names: BTreeSet<String>,
    pub screens: BTreeSet<String>,
    /// `関数`(副作用なし)
    pub functions: BTreeMap<String, FnSig>,
    /// `外部関数` / `分析` / `部品`
    pub externals: BTreeMap<String, ExternalSig>,
    /// `実験`(意味は ext クレートが解釈する)
    pub experiments: Vec<ExperimentInfo>,
    /// `役割`: 役割名 → 権限(API 契約の表記: 予約.create / 動作.予約する / 全件閲覧.予約 / 実験.結果)
    pub roles: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ExperimentInfo {
    pub name: String,
    pub entries: Vec<(String, String, Span)>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ExternalSig {
    pub kind: ExternalKind,
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Option<Type>,
    /// `キー = 値`(値は生テキスト)。意味は ext クレートが解釈する
    pub entries: Vec<(String, String, Span)>,
    /// `効果 = [...]`(外部関数のみ。既定は pure)
    pub effects: BTreeSet<Effect>,
    pub span: Span,
}

impl ExternalSig {
    pub fn entry(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _, _)| k == key).map(|(_, v, _)| v.as_str())
    }
    /// 分析の `対象 = [データ, …]`
    pub fn tables(&self) -> Vec<String> {
        self.entry("対象").map(list_value).unwrap_or_default()
    }
}

/// `[a, b]` 形式の値を語の一覧にする(括弧が無ければ 1 語)
pub fn list_value(s: &str) -> Vec<String> {
    let t = s.trim();
    let inner = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(t);
    inner.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

impl Env {
    pub fn is_type_name(&self, n: &str) -> bool {
        BUILTIN_TYPE_NAMES.contains(&n) || self.aliases.contains_key(n) || self.enums.contains_key(n) || self.data.contains_key(n)
    }
    pub fn enum_has(&self, e: &str, variant: &str) -> bool {
        self.enums.get(e).is_some_and(|vs| vs.iter().any(|v| v == variant))
    }
}

/// 全ファイルの宣言から環境を作る。
/// 役割の宣言(名前, `キー = 値` の行, 位置)。検証は全宣言を集めた後
type RoleRaw = (String, Vec<(String, String, Span)>, Span);

pub fn collect_env(files: &[&SourceFile]) -> (Env, Vec<Diagnostic>) {
    let mut env = Env::default();
    let mut diags = Vec::new();
    let mut type_names: BTreeMap<String, Span> = BTreeMap::new();
    let mut call_names: BTreeMap<String, Span> = BTreeMap::new();
    let mut app_span: Option<Span> = None;
    let mut roles_raw: Vec<RoleRaw> = Vec::new();

    let decls: Vec<&Decl> = files.iter().flat_map(|f| f.items.iter().map(|i| &i.decl)).collect();

    let dup = |names: &mut BTreeMap<String, Span>, id: &Ident, diags: &mut Vec<Diagnostic>| -> bool {
        if let Some(prev) = names.get(&id.name) {
            diags.push(
                Diagnostic::error(Code::REF002, id.span, format!("`{}` は既に定義されています", id.name)).hint(format!("最初の定義は {}行目です", prev.line)),
            );
            true
        } else {
            names.insert(id.name.clone(), id.span);
            false
        }
    };

    // 1 周目: 型名の登録
    for d in &decls {
        let id = match d {
            Decl::TypeAlias(t) => &t.name,
            Decl::Enum(e) => &e.name,
            Decl::Data(dd) => &dd.name,
            _ => continue,
        };
        if BUILTIN_TYPE_NAMES.contains(&id.name.as_str()) {
            diags.push(Diagnostic::error(Code::REF002, id.span, format!("`{}` は組込み型の名前です", id.name)));
            continue;
        }
        if dup(&mut type_names, id, &mut diags) {
            continue;
        }
        if let Decl::Enum(e) = d {
            let mut seen = BTreeSet::new();
            let mut vs = Vec::new();
            for v in &e.variants {
                if !seen.insert(v.name.clone()) {
                    diags.push(Diagnostic::error(Code::REF002, v.span, format!("列挙値 `{}` が重複しています", v.name)));
                } else {
                    vs.push(v.name.clone());
                }
            }
            env.enums.insert(e.name.name.clone(), vs);
        }
    }
    // 型名のプレースホルダ(データは名目型なので先に名前だけ入れる)
    let data_names: BTreeSet<String> = decls
        .iter()
        .filter_map(|d| if let Decl::Data(dd) = d { Some(dd.name.name.clone()) } else { None })
        .filter(|n| type_names.contains_key(n))
        .collect();

    // 別名の解決(循環検出付き)
    let alias_decls: BTreeMap<String, &TypeAliasDecl> = decls
        .iter()
        .filter_map(|d| if let Decl::TypeAlias(t) = d { Some((t.name.name.clone(), t)) } else { None })
        .collect();
    for name in alias_decls.keys() {
        let mut stack = Vec::new();
        resolve_alias(name, &alias_decls, &data_names, &mut env, &mut stack, &mut diags);
    }

    // 2 周目: データ・アダプタ・動作・その他
    for d in &decls {
        match d {
            Decl::App(a) => {
                if let Some(prev) = app_span {
                    diags.push(Diagnostic::error(Code::REF002, a.span, "`アプリ` 宣言は 1 つだけです").hint(format!("最初の宣言は {}行目です", prev.line)));
                } else {
                    app_span = Some(a.span);
                    env.app_name = Some(a.name.clone());
                }
            }
            Decl::Data(dd) => {
                if !data_names.contains(&dd.name.name) || env.data.contains_key(&dd.name.name) {
                    continue;
                }
                let mut fields = Vec::new();
                let mut seen = BTreeSet::new();
                for f in &dd.fields {
                    if !seen.insert(f.name.name.clone()) {
                        diags.push(Diagnostic::error(
                            Code::REF002,
                            f.name.span,
                            format!("フィールド `{}` が重複しています", f.name.name),
                        ));
                        continue;
                    }
                    let t = resolve_type(&f.ty, &env, &data_names, &mut diags);
                    fields.push((f.name.name.clone(), t));
                }
                env.data.insert(
                    dd.name.name.clone(),
                    DataInfo {
                        name: dd.name.name.clone(),
                        fields,
                        span: dd.span,
                    },
                );
            }
            Decl::Knowledge(k) => {
                if type_names.contains_key(&k.name.name) {
                    diags.push(Diagnostic::error(
                        Code::REF002,
                        k.name.span,
                        format!("`{}` は型名と重複しています", k.name.name),
                    ));
                } else if !env.knowledge_names.insert(k.name.name.clone()) {
                    diags.push(Diagnostic::error(
                        Code::REF002,
                        k.name.span,
                        format!("知識 `{}` は既に定義されています", k.name.name),
                    ));
                }
            }
            Decl::Screen(s) if !env.screens.insert(s.name.name.clone()) => {
                diags.push(Diagnostic::error(
                    Code::REF002,
                    s.name.span,
                    format!("画面 `{}` は既に定義されています", s.name.name),
                ));
            }
            _ => {}
        }
    }
    for d in &decls {
        match d {
            Decl::Adapter(a) => {
                if dup(&mut call_names, &a.name, &mut diags) {
                    continue;
                }
                let params = resolve_params(&a.params, &env, &data_names, &mut diags);
                let ret = resolve_type(&a.ret, &env, &data_names, &mut diags);
                let sig = AdapterSig {
                    name: a.name.name.clone(),
                    params,
                    ret,
                    op: a.op,
                    target: a.target.name.clone(),
                    span: a.span,
                };
                check_adapter(&sig, a, &env, &mut diags);
                env.adapters.insert(a.name.name.clone(), sig);
            }
            Decl::Role(x) => {
                if roles_raw
                    .iter()
                    .any(|(n, _, _): &(String, Vec<(String, String, Span)>, Span)| *n == x.name.name)
                {
                    diags.push(Diagnostic::error(
                        Code::REF002,
                        x.name.span,
                        format!("役割 `{}` は既に定義されています", x.name.name),
                    ));
                } else {
                    roles_raw.push((
                        x.name.name.clone(),
                        x.entries.iter().map(|kv| (kv.key.name.clone(), kv.value.text.clone(), kv.value.span)).collect(),
                        x.span,
                    ));
                }
            }
            Decl::Experiment(x) => {
                if env.experiments.iter().any(|e| e.name == x.name.name) {
                    diags.push(Diagnostic::error(
                        Code::REF002,
                        x.name.span,
                        format!("実験 `{}` は既に定義されています", x.name.name),
                    ));
                } else {
                    env.experiments.push(ExperimentInfo {
                        name: x.name.name.clone(),
                        entries: x.entries.iter().map(|kv| (kv.key.name.clone(), kv.value.text.clone(), kv.value.span)).collect(),
                        span: x.span,
                    });
                }
            }
            Decl::Function(f) => {
                if dup(&mut call_names, &f.name, &mut diags) {
                    continue;
                }
                let params = resolve_params(&f.params, &env, &data_names, &mut diags);
                let ret = resolve_type(&f.ret, &env, &data_names, &mut diags);
                env.functions.insert(
                    f.name.name.clone(),
                    FnSig {
                        name: f.name.name.clone(),
                        params,
                        ret,
                        span: f.span,
                    },
                );
            }
            Decl::External(x) => {
                if dup(&mut call_names, &x.name, &mut diags) {
                    continue;
                }
                let params = resolve_params(&x.params, &env, &data_names, &mut diags);
                let ret = x.ret.as_ref().map(|r| resolve_type(r, &env, &data_names, &mut diags));
                let mut effects = BTreeSet::new();
                for kv in &x.entries {
                    if kv.key.name == "効果" {
                        let inner = kv.value.text.trim().trim_start_matches('[').trim_end_matches(']');
                        for e in inner.split(',').map(str::trim).filter(|e| !e.is_empty() && *e != "pure") {
                            match Effect::parse(e) {
                                Some(ef) => {
                                    effects.insert(ef);
                                }
                                None => diags.push(Diagnostic::error(Code::REF001, kv.value.span, format!("未知の効果 `{e}` です"))),
                            }
                        }
                    }
                }
                env.externals.insert(
                    x.name.name.clone(),
                    ExternalSig {
                        kind: x.kind,
                        name: x.name.name.clone(),
                        params,
                        ret,
                        entries: x.entries.iter().map(|kv| (kv.key.name.clone(), kv.value.text.clone(), kv.value.span)).collect(),
                        effects,
                        span: x.span,
                    },
                );
            }
            Decl::Action(a) => {
                if dup(&mut call_names, &a.name, &mut diags) {
                    continue;
                }
                let params = resolve_params(&a.params, &env, &data_names, &mut diags);
                let ret = a.ret.as_ref().map(|r| resolve_type(r, &env, &data_names, &mut diags));
                let declared_caps = a.requires().map(|(caps, _)| {
                    let mut set = BTreeSet::new();
                    for c in caps {
                        match Verb::from_ja(&c.verb.name) {
                            None => diags.push(
                                Diagnostic::error(Code::REF001, c.verb.span, format!("未知の Capability 動詞 `{}` です", c.verb.name))
                                    .hint("読取 / 作成 / 更新 / 削除 のいずれかです"),
                            ),
                            Some(v) => {
                                if !data_names.contains(&c.resource.name) {
                                    diags.push(Diagnostic::error(
                                        Code::REF001,
                                        c.resource.span,
                                        format!("資源 `{}` はデータとして定義されていません", c.resource.name),
                                    ));
                                }
                                set.insert(Capability::new(c.resource.name.clone(), v));
                            }
                        }
                    }
                    set
                });
                let declared_effects = a.effects().map(|(effs, _)| {
                    let mut set = BTreeSet::new();
                    for e in effs {
                        if e.path == "pure" {
                            continue;
                        }
                        match Effect::parse(&e.path) {
                            Some(x) => {
                                set.insert(x);
                            }
                            None => diags.push(Diagnostic::error(Code::REF001, e.span, format!("未知の効果 `{}` です", e.path)).hint(
                                "pure / data.read / data.write / network.send / filesystem.read / filesystem.write / device.camera / device.location / device.notifications",
                            )),
                        }
                    }
                    set
                });
                env.actions.insert(
                    a.name.name.clone(),
                    ActionSig {
                        name: a.name.name.clone(),
                        params,
                        ret,
                        declared_caps,
                        declared_effects,
                        span: a.span,
                    },
                );
            }
            _ => {}
        }
    }
    if app_span.is_none() {
        diags.push(Diagnostic::error(Code::REF001, Span::default(), "`アプリ「名前」` 宣言がありません"));
    }
    // 役割(全宣言を集めた後で、参照先のデータ・動作を確かめる)
    for (name, entries, span) in roles_raw {
        let mut caps = Vec::new();
        let mut found = false;
        for (k, v, sp) in entries {
            if k != "権限" {
                diags.push(Diagnostic::error(
                    Code::REF001,
                    sp,
                    format!("役割のキー `{k}` は定義されていません(使えるキー: 権限)"),
                ));
                continue;
            }
            found = true;
            for item in list_value(&v) {
                match role_cap(&env, &item) {
                    Ok(c) => caps.push(c),
                    Err(m) => diags.push(
                        Diagnostic::error(Code::REF001, sp, m)
                            .hint("書き方: データ.読取 / データ.作成 / データ.更新 / データ.削除 / 動作.動作名 / 全件閲覧.データ / 実験.結果"),
                    ),
                }
            }
        }
        if !found {
            diags.push(Diagnostic::error(Code::AMB001, span, format!("役割 `{name}` に `権限 = [...]` がありません")));
        }
        caps.sort();
        caps.dedup();
        env.roles.insert(name, caps);
    }
    (env, diags)
}

fn resolve_alias(
    name: &str,
    decls: &BTreeMap<String, &TypeAliasDecl>,
    data_names: &BTreeSet<String>,
    env: &mut Env,
    stack: &mut Vec<String>,
    diags: &mut Vec<Diagnostic>,
) -> Type {
    if let Some(t) = env.aliases.get(name) {
        return t.clone();
    }
    let decl = decls[name];
    if stack.iter().any(|s| s == name) {
        diags.push(Diagnostic::error(Code::REF001, decl.name.span, format!("型の別名 `{name}` が循環しています")));
        return Type::Error;
    }
    stack.push(name.to_string());
    // 依存する別名を先に解決
    let mut deps = Vec::new();
    collect_named(&decl.ty, &mut deps);
    for d in deps {
        if decls.contains_key(&d) && !env.aliases.contains_key(&d) {
            resolve_alias(&d, decls, data_names, env, stack, diags);
        }
    }
    stack.pop();
    let t = resolve_type(&decl.ty, env, data_names, diags);
    env.aliases.insert(name.to_string(), t.clone());
    t
}

fn collect_named(t: &TypeExpr, out: &mut Vec<String>) {
    match t {
        TypeExpr::Named { name, args, .. } => {
            out.push(name.name.clone());
            args.iter().for_each(|a| collect_named(a, out));
        }
        TypeExpr::Record { fields, .. } => fields.iter().for_each(|f| collect_named(&f.ty, out)),
    }
}

fn resolve_params(ps: &[Param], env: &Env, data_names: &BTreeSet<String>, diags: &mut Vec<Diagnostic>) -> Vec<(String, Type)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in ps {
        if !seen.insert(p.name.name.clone()) {
            diags.push(Diagnostic::error(Code::REF002, p.name.span, format!("引数 `{}` が重複しています", p.name.name)));
        }
        out.push((p.name.name.clone(), resolve_type(&p.ty, env, data_names, diags)));
    }
    out
}

/// 型式 → 型。
pub fn resolve_type(t: &TypeExpr, env: &Env, data_names: &BTreeSet<String>, diags: &mut Vec<Diagnostic>) -> Type {
    match t {
        TypeExpr::Record { fields, .. } => {
            let mut seen = BTreeSet::new();
            let mut out = Vec::new();
            for f in fields {
                if !seen.insert(f.name.name.clone()) {
                    diags.push(Diagnostic::error(
                        Code::REF002,
                        f.name.span,
                        format!("フィールド `{}` が重複しています", f.name.name),
                    ));
                }
                out.push((f.name.name.clone(), resolve_type(&f.ty, env, data_names, diags)));
            }
            Type::Record(out)
        }
        TypeExpr::Named { name, args, span } => {
            let n = name.name.as_str();
            let arity = |k: usize, diags: &mut Vec<Diagnostic>| -> bool {
                if args.len() != k {
                    diags.push(Diagnostic::error(
                        Code::TYP001,
                        *span,
                        format!("型 `{n}` の型引数は {k} 個ですが {} 個あります", args.len()),
                    ));
                    false
                } else {
                    true
                }
            };
            let arg = |i: usize, diags: &mut Vec<Diagnostic>| resolve_type(&args[i], env, data_names, diags);
            match n {
                "真偽" | "整数" | "小数" | "文字列" | "日時" | "日付" | "識別子" | "バイト列" | "期間" | "URL" | "メールアドレス" =>
                {
                    if !arity(0, diags) {
                        return Type::Error;
                    }
                    match n {
                        "真偽" => Type::Bool,
                        "整数" => Type::Int,
                        "小数" => Type::Decimal,
                        "文字列" => Type::Str,
                        "日時" => Type::DateTime,
                        "日付" => Type::Date,
                        "識別子" => Type::Id,
                        "バイト列" => Type::Bytes,
                        "期間" => Type::Duration,
                        "URL" => Type::Url,
                        _ => Type::Email,
                    }
                }
                "金額" => {
                    if !arity(1, diags) {
                        return Type::Error;
                    }
                    match &args[0] {
                        TypeExpr::Named { name: c, args: a2, .. } if a2.is_empty() && is_currency_code(&c.name) => Type::Money(c.name.clone()),
                        other => {
                            diags.push(Diagnostic::error(
                                Code::TYP001,
                                other.span(),
                                "金額の型引数は通貨コード(英大文字 3 文字、例 JPY)です",
                            ));
                            Type::Error
                        }
                    }
                }
                "配列" | "一覧" | "選択" => {
                    if !arity(1, diags) {
                        return Type::Error;
                    }
                    let a = arg(0, diags);
                    match n {
                        "配列" => Type::Array(Box::new(a)),
                        "一覧" => Type::List(Box::new(a)),
                        _ => {
                            if matches!(a, Type::Option(_)) {
                                diags.push(Diagnostic::error(Code::TYP001, *span, "選択<選択<T>> は使えません"));
                            }
                            Type::Option(Box::new(a))
                        }
                    }
                }
                "結果" => {
                    if !arity(2, diags) {
                        return Type::Error;
                    }
                    let a = arg(0, diags);
                    let b = arg(1, diags);
                    Type::Result(Box::new(a), Box::new(b))
                }
                _ => {
                    if !args.is_empty() {
                        diags.push(Diagnostic::error(Code::TYP001, *span, format!("型 `{n}` は型引数を取りません")));
                    }
                    if let Some(t) = env.aliases.get(n) {
                        t.clone()
                    } else if env.enums.contains_key(n) {
                        Type::Enum(n.to_string())
                    } else if data_names.contains(n) {
                        Type::Data(n.to_string())
                    } else {
                        diags.push(
                            Diagnostic::error(Code::REF001, name.span, format!("型 `{n}` は定義されていません"))
                                .hint("データ / 列挙 / 型 宣言で定義してください"),
                        );
                        Type::Error
                    }
                }
            }
        }
    }
}

fn check_adapter(sig: &AdapterSig, a: &AdapterDecl, env: &Env, diags: &mut Vec<Diagnostic>) {
    let Some(data) = env.data.get(&sig.target) else {
        diags.push(Diagnostic::error(
            Code::REF001,
            a.target.span,
            format!("データ `{}` は定義されていません", sig.target),
        ));
        return;
    };
    let d = Type::Data(sig.target.clone());
    let id_ty = data.field("ID").cloned();
    let need_id = matches!(sig.op, OpKind::Get | OpKind::Update | OpKind::Delete);
    if need_id && id_ty.is_none() {
        diags.push(Diagnostic::error(
            Code::REF001,
            a.target.span,
            format!("操作 `{}` には データ `{}` の `ID` フィールドが必要です", sig.op.word(), sig.target),
        ));
        return;
    }
    let bad = |msg: String, diags: &mut Vec<Diagnostic>| diags.push(Diagnostic::error(Code::TYP001, a.span, msg));
    match sig.op {
        OpKind::Get => {
            if sig.params.len() != 1 || !sig.params[0].1.same(id_ty.as_ref().unwrap()) {
                bad(format!("操作 取得 の引数は ID の 1 つ(型 {})です", id_ty.unwrap()), diags);
            }
            if !(sig.ret.same(&d) || sig.ret.same(&Type::Option(Box::new(d.clone())))) {
                bad(format!("操作 取得 の戻り値は {d} または 選択<{d}> です"), diags);
            }
        }
        OpKind::List => {
            if !sig.params.is_empty() {
                bad("操作 一覧 の絞り込み引数は MVP では未対応です(ADR-0104)".into(), diags);
            }
            if !sig.ret.same(&Type::List(Box::new(d.clone()))) {
                bad(format!("操作 一覧 の戻り値は 一覧<{d}> です"), diags);
            }
        }
        OpKind::Create | OpKind::Update => {
            for (pn, pt) in &sig.params {
                match data.field(pn) {
                    None => diags.push(Diagnostic::error(
                        Code::REF001,
                        a.span,
                        format!("引数 `{pn}` は データ `{}` のフィールドではありません", sig.target),
                    )),
                    Some(ft) if !ft.same(pt) => bad(format!("引数 `{pn}` の型 {pt} がフィールドの型 {ft} と一致しません"), diags),
                    _ => {}
                }
            }
            if sig.op == OpKind::Create {
                let missing: Vec<&str> = data
                    .fields
                    .iter()
                    .filter(|(n, t)| n != "ID" && !matches!(t, Type::Option(_)) && !sig.params.iter().any(|(pn, _)| pn == n))
                    .map(|(n, _)| n.as_str())
                    .collect();
                if !missing.is_empty() {
                    bad(format!("操作 登録 の引数に必須フィールド {} がありません", missing.join(", ")), diags);
                }
                if sig.params.iter().any(|(n, _)| n == "ID") {
                    bad("操作 登録 では ID は生成されるため引数にできません(ADR-0104)".into(), diags);
                }
                let ok = sig.ret.same(&d) || id_ty.as_ref().is_some_and(|t| sig.ret.same(t));
                if !ok {
                    bad(format!("操作 登録 の戻り値は {d} またはその ID 型です"), diags);
                }
            } else {
                if !sig.params.iter().any(|(n, _)| n == "ID") {
                    bad("操作 更新 には引数 ID が必要です".into(), diags);
                }
                if !sig.ret.same(&d) {
                    bad(format!("操作 更新 の戻り値は {d} です"), diags);
                }
            }
        }
        OpKind::Delete => {
            if sig.params.len() != 1 || sig.params[0].0 != "ID" {
                bad("操作 削除 の引数は ID の 1 つです".into(), diags);
            }
            if !sig.ret.same(&Type::Bool) {
                bad("操作 削除 の戻り値は 真偽 です".into(), diags);
            }
        }
    }
}

/// 役割の権限 1 つを、API 契約の表記(予約.create など)に直す
fn role_cap(env: &Env, item: &str) -> Result<String, String> {
    let Some((head, tail)) = item.split_once('.') else {
        return Err(format!("権限 `{item}` は `データ.読取` のように `.` で区切って書きます"));
    };
    match head {
        "動作" => {
            if env.actions.contains_key(tail) {
                Ok(item.to_string())
            } else {
                Err(format!("動作 `{tail}` は定義されていません"))
            }
        }
        "全件閲覧" => {
            if env.data.contains_key(tail) {
                Ok(item.to_string())
            } else {
                Err(format!("データ `{tail}` は定義されていません"))
            }
        }
        "実験" if tail == "結果" => Ok(item.to_string()),
        d => {
            if !env.data.contains_key(d) {
                return Err(format!("データ `{d}` は定義されていません"));
            }
            let verb = match tail {
                "読取" | "read" => "read",
                "作成" | "create" => "create",
                "更新" | "update" => "update",
                "削除" | "delete" => "delete",
                other => return Err(format!("`{other}` は権限の動詞ではありません(読取 / 作成 / 更新 / 削除)")),
            };
            Ok(format!("{d}.{verb}"))
        }
    }
}
