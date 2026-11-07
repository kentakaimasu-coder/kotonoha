//! 拡張の意味検査: 外部関数(TypeScript / Python)・分析(読み取り専用 SQL)・拡張部品・実験(A/B テスト)。
//! ADR-0122〜0125。
//!
//! - 実装ファイルはプロジェクト内の相対パスだけ。内容の SHA-256 を VAG に記録し、ロックの対象にする
//! - 分析の SQL は構文解析し、SELECT 1 文・宣言した表だけ・許可した関数だけ・宣言した引数だけであることを確かめる
//! - 実験は画面上の対象(ボタンのラベル または 拡張部品)・変種・割合・成果を確かめる

use kotonoha_ast::ExternalKind;
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_intent::{IntentKind, IntentModel};
use kotonoha_types::{list_value, Effect, Env, Type};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ops::ControlFlow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtFile {
    pub path: String,
    pub sha256: String,
    #[serde(skip)]
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    TypeScript,
    Python,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalFn {
    pub name: String,
    #[serde(skip)]
    pub params: Vec<(String, Type)>,
    #[serde(skip)]
    pub ret: Type,
    pub effects: BTreeSet<Effect>,
    pub lang: Lang,
    pub file: ExtFile,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisDef {
    pub name: String,
    #[serde(skip)]
    pub params: Vec<(String, Type)>,
    #[serde(skip)]
    pub ret: Type,
    pub tables: Vec<String>,
    pub file: ExtFile,
    /// `:名前` を `?N`(宣言順の番号)に置き換えた SQL
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentDef {
    pub name: String,
    #[serde(skip)]
    pub params: Vec<(String, Type)>,
    pub web: Option<ExtFile>,
    pub ios: Option<ExtFile>,
    pub android: Option<ExtFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExperimentTarget {
    /// ボタンのラベルを出し分ける
    Label,
    /// 拡張部品を出し分ける
    Component,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExperimentDef {
    pub name: String,
    pub screen: String,
    pub target: ExperimentTarget,
    /// 先頭が対照群
    pub variants: Vec<String>,
    pub weights: Vec<u32>,
    /// 成果として数える動作
    pub goal: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ExtModel {
    pub functions: Vec<ExternalFn>,
    pub analyses: Vec<AnalysisDef>,
    pub components: Vec<ComponentDef>,
    pub experiments: Vec<ExperimentDef>,
}

impl ExtModel {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("直列化できる")
    }
    /// 画面上のボタン(ラベル)に掛かっている実験
    pub fn label_experiment(&self, screen: &str, label: &str) -> Option<&ExperimentDef> {
        self.experiments
            .iter()
            .find(|e| e.screen == screen && e.target == ExperimentTarget::Label && e.variants[0] == label)
    }
    /// 画面上の部品に掛かっている実験
    pub fn component_experiment(&self, screen: &str, component: &str) -> Option<&ExperimentDef> {
        self.experiments
            .iter()
            .find(|e| e.screen == screen && e.target == ExperimentTarget::Component && e.variants[0] == component)
    }
    pub fn component(&self, name: &str) -> Option<&ComponentDef> {
        self.components.iter().find(|c| c.name == name)
    }
}

pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// 実装ファイルの読み込み口(driver が与える。読んだファイルは source_hash に含める)
pub type Reader<'a> = dyn Fn(&str) -> Result<String, String> + 'a;

fn unquote(s: &str) -> Option<&str> {
    let t = s.trim();
    t.strip_prefix('"').and_then(|x| x.strip_suffix('"'))
}

fn load_file(reader: &Reader, value: &str, span: Span, exts: &[&str], diags: &mut Vec<Diagnostic>) -> Option<ExtFile> {
    let Some(path) = unquote(value) else {
        diags.push(Diagnostic::error(
            Code::SYN001,
            span,
            "ファイルの指定は \"ext/StarRating.ts\" のように二重引用符で囲みます",
        ));
        return None;
    };
    let bad = path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains("..") || path.contains(':') || path.contains('\\');
    if bad {
        diags.push(Diagnostic::error(
            Code::SYN001,
            span,
            format!("`{path}` はプロジェクト内の相対パスで指定してください(`..`・絶対パス不可)"),
        ));
        return None;
    }
    if !exts.iter().any(|e| path.ends_with(e)) {
        diags.push(Diagnostic::error(
            Code::SYN001,
            span,
            format!("`{path}` の拡張子は {} のいずれかです", exts.join(" / ")),
        ));
        return None;
    }
    match reader(path) {
        Ok(content) => Some(ExtFile {
            path: path.to_string(),
            sha256: sha256_hex(&content),
            content,
        }),
        Err(e) => {
            diags.push(Diagnostic::error(Code::REF001, span, format!("ファイル `{path}` を読めません: {e}")));
            None
        }
    }
}

fn wire_ok(t: &Type) -> bool {
    match t {
        Type::Bool | Type::Int | Type::Str | Type::Id | Type::Url | Type::Email | Type::DateTime | Type::Enum(_) => true,
        Type::Money(c) => c == "JPY",
        Type::Option(i) | Type::List(i) | Type::Array(i) => wire_ok(i),
        Type::Record(fs) => fs.iter().all(|(_, t)| wire_ok(t)),
        _ => false,
    }
}

/// SQL で使ってよい関数(移植性と安全性のための許可リスト。ADR-0125)
pub const SQL_FUNCTIONS: &[&str] = &[
    "count",
    "sum",
    "avg",
    "min",
    "max",
    "total",
    "coalesce",
    "ifnull",
    "nullif",
    "abs",
    "round",
    "lower",
    "upper",
    "length",
    "substr",
    "substring",
    "trim",
    "ltrim",
    "rtrim",
    "replace",
    "instr",
];

/// 分析の SQL を検査し、`:名前` を `?N` にした SQL を返す。
pub fn check_sql(sql: &str, tables: &[String], params: &[(String, Type)], span: Span) -> Result<String, Vec<Diagnostic>> {
    use sqlparser::ast::{visit_expressions, visit_relations, Expr, Query, SetExpr, Statement};
    use sqlparser::dialect::SQLiteDialect;
    let err = |m: String| Diagnostic::error(Code::TYP001, span, m);
    let stmts = sqlparser::parser::Parser::parse_sql(&SQLiteDialect {}, sql).map_err(|e| vec![err(format!("SQL を解析できません: {e}"))])?;
    if stmts.len() != 1 {
        return Err(vec![err(format!("分析の SQL は 1 文だけです({} 文あります)", stmts.len()))]);
    }
    let Statement::Query(q) = &stmts[0] else {
        return Err(vec![err("分析の SQL は SELECT(読み取り)だけです".into())]);
    };
    let mut ds = Vec::new();
    fn no_into(q: &Query) -> bool {
        match q.body.as_ref() {
            SetExpr::Select(s) => s.into.is_none(),
            _ => true,
        }
    }
    if !no_into(q) {
        ds.push(err("SELECT ... INTO は使えません".into()));
    }
    // CTE 名は表として参照してよい
    let ctes: BTreeSet<String> = q
        .with
        .as_ref()
        .map(|w| w.cte_tables.iter().map(|c| c.alias.name.value.clone()).collect())
        .unwrap_or_default();
    let _ = visit_relations(&stmts[0], |rel| {
        let name = rel.0.last().map(|p| p.to_string().trim_matches('"').to_string()).unwrap_or_default();
        if rel.0.len() > 1 {
            ds.push(err(format!("スキーマ付きの表名 `{rel}` は使えません")));
        } else if !tables.contains(&name) && !ctes.contains(&name) {
            ds.push(err(format!("表 `{name}` は `対象` に宣言されていません(読める表: {})", tables.join(", "))));
        }
        ControlFlow::<()>::Continue(())
    });
    let _ = visit_expressions(&stmts[0], |e| {
        if let Expr::Function(f) = e {
            let n = f.name.to_string().to_lowercase();
            if !SQL_FUNCTIONS.contains(&n.as_str()) {
                ds.push(err(format!("SQL 関数 `{n}` は使えません(使える関数: {})", SQL_FUNCTIONS.join(", "))));
            }
        }
        ControlFlow::<()>::Continue(())
    });
    // `:名前` → `?N`
    let mut out = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut used = BTreeSet::new();
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == c {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            '?' | '$' | '@' => {
                ds.push(err(format!("SQL の引数は `:名前` で書きます(`{c}` は使えません)")));
                i += 1;
            }
            ':' if chars.get(i + 1).is_some_and(|x| x.is_alphanumeric() || *x == '_') => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let name: String = chars[i + 1..j].iter().collect();
                match params.iter().position(|(p, _)| *p == name) {
                    Some(k) => {
                        used.insert(name);
                        out.push_str(&format!("?{}", k + 1));
                    }
                    None => ds.push(err(format!("SQL の引数 `:{name}` は宣言されていません"))),
                }
                i = j;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    for (p, _) in params {
        if !used.contains(p) {
            ds.push(err(format!("引数 `{p}` が SQL で使われていません")));
        }
    }
    if ds.is_empty() {
        Ok(out.trim().trim_end_matches(';').trim().to_string())
    } else {
        Err(ds)
    }
}

pub fn resolve(env: &Env, intents: &IntentModel, reader: &Reader) -> (ExtModel, Vec<Diagnostic>) {
    let mut m = ExtModel::default();
    let mut diags = Vec::new();
    for (name, x) in &env.externals {
        let entry = |k: &str| x.entries.iter().find(|(kk, _, _)| kk == k).map(|(_, v, s)| (v.as_str(), *s));
        let allowed: &[&str] = match x.kind {
            ExternalKind::Function => &["実装", "効果"],
            ExternalKind::Analysis => &["対象", "SQL"],
            ExternalKind::Component => &["web", "ios", "android"],
        };
        for (k, _, s) in &x.entries {
            if !allowed.contains(&k.as_str()) {
                diags.push(Diagnostic::error(
                    Code::REF001,
                    *s,
                    format!("{} のキー `{k}` は定義されていません(使えるキー: {})", x.kind.word(), allowed.join(" / ")),
                ));
            }
        }
        for (p, t) in &x.params {
            if !wire_ok(t) {
                diags.push(Diagnostic::error(
                    Code::TGT001,
                    x.span,
                    format!("{} `{name}` の引数 {p}: 型 {t} は受け渡しできません", x.kind.word()),
                ));
            }
        }
        match x.kind {
            ExternalKind::Function => {
                let Some((v, s)) = entry("実装") else {
                    diags.push(Diagnostic::error(
                        Code::AMB001,
                        x.span,
                        format!("外部関数 `{name}` に `実装 = \"ext/….ts\"` または `\"ml/….py\"` がありません"),
                    ));
                    continue;
                };
                let ret = x.ret.clone().unwrap_or(Type::Error);
                if !wire_ok(&ret) {
                    diags.push(Diagnostic::error(
                        Code::TGT001,
                        x.span,
                        format!("外部関数 `{name}` の戻り値 {ret} は受け渡しできません"),
                    ));
                }
                if let Some(file) = load_file(reader, v, s, &[".ts", ".py"], &mut diags) {
                    let lang = if file.path.ends_with(".py") { Lang::Python } else { Lang::TypeScript };
                    m.functions.push(ExternalFn {
                        name: name.clone(),
                        params: x.params.clone(),
                        ret,
                        effects: x.effects.clone(),
                        lang,
                        file,
                    });
                }
            }
            ExternalKind::Analysis => {
                let tables = x.tables();
                for t in &tables {
                    if !env.data.contains_key(t) {
                        diags.push(Diagnostic::error(
                            Code::REF001,
                            x.span,
                            format!("分析 `{name}` の対象 `{t}` はデータとして定義されていません"),
                        ));
                    }
                }
                if tables.is_empty() {
                    diags.push(Diagnostic::error(
                        Code::AMB001,
                        x.span,
                        format!("分析 `{name}` に `対象 = [データ, …]` がありません(読める表を宣言します)"),
                    ));
                    continue;
                }
                let ret = x.ret.clone().unwrap_or(Type::Error);
                let row_ok = matches!(&ret, Type::List(r) if matches!(r.as_ref(), Type::Record(fs) if fs.iter().all(|(_, t)| wire_ok(t) && !matches!(t, Type::List(_) | Type::Record(_)))));
                if !row_ok {
                    diags.push(Diagnostic::error(
                        Code::TYP001,
                        x.span,
                        format!("分析 `{name}` の戻り値は 一覧<記録{{列: 型, …}}> です({ret})"),
                    ));
                }
                let Some((v, s)) = entry("SQL") else {
                    diags.push(Diagnostic::error(
                        Code::AMB001,
                        x.span,
                        format!("分析 `{name}` に `SQL = \"analytics/….sql\"` がありません"),
                    ));
                    continue;
                };
                if let Some(file) = load_file(reader, v, s, &[".sql"], &mut diags) {
                    match check_sql(&file.content, &tables, &x.params, s) {
                        Ok(sql) => m.analyses.push(AnalysisDef {
                            name: name.clone(),
                            params: x.params.clone(),
                            ret,
                            tables,
                            file,
                            sql,
                        }),
                        Err(ds) => diags.extend(ds),
                    }
                }
            }
            ExternalKind::Component => {
                let mut c = ComponentDef {
                    name: name.clone(),
                    params: x.params.clone(),
                    web: None,
                    ios: None,
                    android: None,
                };
                if let Some((v, s)) = entry("web") {
                    c.web = load_file(reader, v, s, &[".ts"], &mut diags);
                }
                if let Some((v, s)) = entry("ios") {
                    c.ios = load_file(reader, v, s, &[".swift"], &mut diags);
                }
                if let Some((v, s)) = entry("android") {
                    c.android = load_file(reader, v, s, &[".kt"], &mut diags);
                }
                if c.web.is_none() && c.ios.is_none() && c.android.is_none() && !diags.iter().any(|d| d.span.line == x.span.line) {
                    diags.push(Diagnostic::error(
                        Code::AMB001,
                        x.span,
                        format!("部品 `{name}` に実装(web / ios / android)が 1 つもありません"),
                    ));
                }
                m.components.push(c);
            }
        }
    }
    resolve_experiments(env, intents, &mut m, &mut diags);
    (m, diags)
}

fn resolve_experiments(env: &Env, intents: &IntentModel, m: &mut ExtModel, diags: &mut Vec<Diagnostic>) {
    let mut taken: BTreeSet<(String, String)> = BTreeSet::new();
    for x in &env.experiments {
        let get = |k: &str| x.entries.iter().find(|(kk, _, _)| kk == k).map(|(_, v, s)| (v.clone(), *s));
        for (k, _, s) in &x.entries {
            if !["画面", "変種", "割合", "成果"].contains(&k.as_str()) {
                diags.push(Diagnostic::error(
                    Code::REF001,
                    *s,
                    format!("実験のキー `{k}` は定義されていません(画面 / 変種 / 割合 / 成果)"),
                ));
            }
        }
        let mut missing = Vec::new();
        for k in ["画面", "変種", "割合", "成果"] {
            if get(k).is_none() {
                missing.push(k);
            }
        }
        if !missing.is_empty() {
            diags.push(Diagnostic::error(
                Code::AMB001,
                x.span,
                format!("実験 `{}` に {} がありません", x.name, missing.join("・")),
            ));
            continue;
        }
        let (screen, ss) = get("画面").unwrap();
        let (variants_raw, vs) = get("変種").unwrap();
        let (weights_raw, ws) = get("割合").unwrap();
        let (goal, gs) = get("成果").unwrap();
        let variants = list_value(&variants_raw);
        let weights: Vec<u32> = list_value(&weights_raw).iter().filter_map(|w| w.parse().ok()).collect();
        let Some(sm) = intents.screens.iter().find(|s| s.name == screen) else {
            diags.push(Diagnostic::error(Code::REF001, ss, format!("画面 `{screen}` は定義されていません")));
            continue;
        };
        if !(2..=10).contains(&variants.len()) {
            diags.push(Diagnostic::error(Code::AMB001, vs, "変種は 2〜10 個です(先頭が対照群)"));
            continue;
        }
        if weights.len() != variants.len() || weights.contains(&0) || weights.iter().sum::<u32>() != 100 {
            diags.push(Diagnostic::error(
                Code::AMB001,
                ws,
                "割合は変種と同じ数の正の整数で、合計 100 にします(例: [50, 50])",
            ));
            continue;
        }
        if !env.actions.contains_key(&goal) {
            diags.push(Diagnostic::error(Code::REF001, gs, format!("成果の動作 `{goal}` は定義されていません")));
            continue;
        }
        let label = sm
            .intents
            .iter()
            .any(|i| matches!(&i.kind, IntentKind::ButtonInvoke { label, .. } if *label == variants[0]));
        let comp = sm
            .intents
            .iter()
            .any(|i| matches!(&i.kind, IntentKind::Component { component, .. } if *component == variants[0]));
        let target = if label {
            ExperimentTarget::Label
        } else if comp {
            let base = env.externals.get(&variants[0]).map(|s| s.params.clone()).unwrap_or_default();
            let mut ok = true;
            for v in &variants[1..] {
                match env.externals.get(v) {
                    Some(s) if s.kind == ExternalKind::Component && s.params == base => {}
                    Some(s) if s.kind == ExternalKind::Component => {
                        diags.push(Diagnostic::error(
                            Code::TYP001,
                            vs,
                            format!("部品 `{v}` の引数が対照群 `{}` と違います", variants[0]),
                        ));
                        ok = false;
                    }
                    _ => {
                        diags.push(Diagnostic::error(Code::REF001, vs, format!("変種の部品 `{v}` は定義されていません")));
                        ok = false;
                    }
                }
            }
            if !ok {
                continue;
            }
            ExperimentTarget::Component
        } else {
            diags.push(
                Diagnostic::error(
                    Code::REF001,
                    vs,
                    format!("画面 `{screen}` にラベル「{}」のボタンも部品 `{}` もありません", variants[0], variants[0]),
                )
                .hint("変種の先頭(対照群)は、画面にある現在のボタンのラベル または 部品名です"),
            );
            continue;
        };
        if !taken.insert((screen.clone(), variants[0].clone())) {
            diags.push(Diagnostic::error(
                Code::AMB002,
                x.span,
                format!("画面 `{screen}` の「{}」に実験が 2 つあります", variants[0]),
            ));
            continue;
        }
        if variants.iter().collect::<BTreeSet<_>>().len() != variants.len() {
            diags.push(Diagnostic::error(Code::AMB002, vs, "変種が重複しています"));
            continue;
        }
        m.experiments.push(ExperimentDef {
            name: x.name.clone(),
            screen,
            target,
            variants,
            weights,
            goal,
        });
    }
}

/// 割付(ADR-0122): SHA-256(実験名 ":" 利用者 ID) の先頭 8 バイトを 100 で割った余りで変種を選ぶ。
/// backend の runtime(TypeScript)と同じ計算(テストで一致を確認する)。
pub fn assign(experiment: &str, subject: &str, weights: &[u32]) -> usize {
    let h = Sha256::digest(format!("{experiment}:{subject}").as_bytes());
    let n = u64::from_be_bytes(h[..8].try_into().unwrap()) % 100;
    let mut acc = 0u64;
    for (i, w) in weights.iter().enumerate() {
        acc += *w as u64;
        if n < acc {
            return i;
        }
    }
    weights.len() - 1
}

/// VAG の `extensions` 節
pub fn vag_json(m: &ExtModel) -> Value {
    json!({
        "functions": m.functions.iter().map(|f| json!({"name": f.name, "lang": f.lang, "file": f.file, "effects": f.effects})).collect::<Vec<_>>(),
        "analyses": m.analyses.iter().map(|a| json!({"name": a.name, "tables": a.tables, "file": a.file})).collect::<Vec<_>>(),
        "components": m.components.iter().map(|c| json!({"name": c.name, "web": c.web, "ios": c.ios, "android": c.android})).collect::<Vec<_>>(),
        "experiments": m.experiments,
    })
}

#[cfg(test)]
mod tests;
