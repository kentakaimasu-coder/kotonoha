//! 知識レジストリ(FR-005)。`知識` 宣言を承認可能な意味定義に変換し、名前と別名を一意に解決する。
//!
//! MVP で扱える知識の形状は「集計ランキング」(技術仕様書 §8)だけ。値の書式は
//! docs/spec/knowledge.md に固定し、書式外の表現(「最近」など)は推測せず AMB001 として質問にする。

use kotonoha_ast::{Decl, Expr, ExprKind, KnowledgeDecl, KvEntry, SourceFile};
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_types::{check_expr_in, Env, Type};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Agg {
    #[serde(rename = "sum")]
    Sum,
    #[serde(rename = "count")]
    Count,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Metric {
    pub agg: Agg,
    /// 合計対象フィールド(件数のときは None)
    pub field: Option<String>,
    /// 結果列名(合計ならフィールド名、件数なら「件数」)
    pub output: String,
    #[serde(serialize_with = "ser_type")]
    pub ty: Type,
}

fn ser_type<S: serde::Serializer>(t: &Type, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&t.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrderKey {
    /// 結果列名
    pub column: String,
    pub dir: Dir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Tax {
    #[serde(rename = "included")]
    Included,
    #[serde(rename = "excluded")]
    Excluded,
}

/// 確定済みの集計ランキング定義。VAG の `knowledge` 節にそのまま入り、knowledge_hash の対象になる。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankingSpec {
    pub name: String,
    pub aliases: Vec<String>,
    pub source: String,
    /// 区間 [基準日時 - window_days 日, 基準日時)
    pub window_days: u32,
    pub time_field: String,
    pub time_zone: String,
    pub metric: Metric,
    pub unit: String,
    #[serde(serialize_with = "ser_type")]
    pub unit_ty: Type,
    pub order: Vec<OrderKey>,
    pub limit: u32,
    pub tax: Tax,
    /// 絞り込み条件(なしなら None)。型検査済み。
    #[serde(serialize_with = "ser_filter")]
    pub filter: Option<Expr>,
    #[serde(skip)]
    pub span: Span,
}

fn ser_filter<S: serde::Serializer>(f: &Option<Expr>, s: S) -> Result<S::Ok, S::Error> {
    match f {
        None => s.serialize_none(),
        Some(e) => s.serialize_str(&kotonoha_parser::printer::expr_str(e)),
    }
}

impl RankingSpec {
    /// 結果の要素型 記録{単位: T, 指標: U}
    pub fn row_type(&self) -> Type {
        Type::Record(vec![
            (self.unit.clone(), self.unit_ty.clone()),
            (self.metric.output.clone(), self.metric.ty.clone()),
        ])
    }
    pub fn result_type(&self) -> Type {
        Type::List(Box::new(self.row_type()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "shape")]
pub enum Knowledge {
    #[serde(rename = "ranking")]
    Ranking(RankingSpec),
}

impl Knowledge {
    pub fn name(&self) -> &str {
        match self {
            Knowledge::Ranking(r) => &r.name,
        }
    }
    pub fn aliases(&self) -> &[String] {
        match self {
            Knowledge::Ranking(r) => &r.aliases,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub entries: BTreeMap<String, Knowledge>,
    /// 名前・別名 → 正式名
    pub names: BTreeMap<String, String>,
}

impl Registry {
    pub fn resolve(&self, name: &str) -> Option<&Knowledge> {
        self.names.get(name).and_then(|c| self.entries.get(c))
    }
}

pub const REQUIRED_RANKING_KEYS: &[(&str, &str)] = &[
    ("対象", "集計対象のデータはどれですか(例: 対象 = 注文明細)"),
    ("期間", "集計期間を日数で確定してください(例: 期間 = 直近30日)"),
    ("指標", "何で順位を付けますか(例: 指標 = 売上金額の合計)"),
    ("単位", "何ごとに集計しますか(例: 単位 = 商品ID)"),
    ("並び", "並び順と同順位の扱いを確定してください(例: 並び = 売上金額の降順, 商品IDの昇順)"),
    ("件数", "何件表示しますか(例: 件数 = 20)"),
    ("日時列", "期間の判定に使う日時はどのフィールドですか(例: 日時列 = 確定日時)"),
    ("時刻帯", "期間の境界をどの時刻帯で数えますか(例: 時刻帯 = Asia/Tokyo)"),
    ("税区分", "金額は税込ですか税抜ですか(税区分 = 税込 / 税抜)"),
    (
        "条件",
        "集計対象に含める状態を確定してください(取消・返品の扱い。例: 条件 = 状態 == \"確定\"。無条件なら 条件 = なし)",
    ),
];
pub const OPTIONAL_KEYS: &[&str] = &["別名"];
/// コンパイラが境界計算できる時刻帯(固定オフセットのみ。tz データベース導入は ADR-0108)
pub const SUPPORTED_TIME_ZONES: &[(&str, i32)] = &[("Asia/Tokyo", 9 * 3600), ("UTC", 0)];

/// 全ファイルの知識宣言からレジストリを作る。
pub fn build_registry(files: &[&SourceFile], env: &Env) -> (Registry, Vec<Diagnostic>) {
    let mut reg = Registry::default();
    let mut diags = Vec::new();
    for f in files {
        for it in &f.items {
            if let Decl::Knowledge(k) = &it.decl {
                if reg.entries.contains_key(&k.name.name) {
                    continue; // REF002 は types::collect_env が報告済み
                }
                if let Some(kn) = parse_knowledge(k, env, &mut diags) {
                    reg.entries.insert(k.name.name.clone(), kn);
                }
            }
        }
    }
    // 名前と別名の登録(衝突は複数解釈 AMB002)
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (n, k) in &reg.entries {
        owners.entry(n.clone()).or_default().push(n.clone());
        for a in k.aliases() {
            owners.entry(a.clone()).or_default().push(n.clone());
        }
    }
    for (name, os) in owners {
        let mut uniq: Vec<String> = os.clone();
        uniq.sort();
        uniq.dedup();
        if uniq.len() > 1 || (os.len() > 1 && !reg.entries.contains_key(&name)) {
            let span = reg.entries.get(&uniq[0]).map(|k| match k {
                Knowledge::Ranking(r) => r.span,
            });
            diags.push(
                Diagnostic::error(
                    Code::AMB002,
                    span.unwrap_or_default(),
                    format!("`{name}` が複数の知識({})を指します", uniq.join(", ")),
                )
                .hint("別名はプロジェクト内で一意にしてください"),
            );
            continue;
        }
        if env.is_type_name(&name) && !reg.entries.contains_key(&name) {
            diags.push(Diagnostic::error(
                Code::AMB002,
                Span::default(),
                format!("別名 `{name}` が型名と重複しています"),
            ));
            continue;
        }
        reg.names.insert(name, uniq[0].clone());
    }
    (reg, diags)
}

fn amb(span: Span, msg: impl Into<String>, question: &str) -> Diagnostic {
    Diagnostic::error(Code::AMB001, span, msg).hint(format!("質問: {question}"))
}

fn parse_knowledge(k: &KnowledgeDecl, env: &Env, diags: &mut Vec<Diagnostic>) -> Option<Knowledge> {
    let mut map: BTreeMap<&str, &KvEntry> = BTreeMap::new();
    for e in &k.entries {
        let key = e.key.name.as_str();
        let known = REQUIRED_RANKING_KEYS.iter().any(|(k2, _)| *k2 == key) || OPTIONAL_KEYS.contains(&key);
        if !known {
            diags.push(
                Diagnostic::error(Code::REF001, e.key.span, format!("知識のキー `{key}` は定義されていません")).hint(format!(
                    "使えるキー: {}",
                    REQUIRED_RANKING_KEYS
                        .iter()
                        .map(|x| x.0)
                        .chain(OPTIONAL_KEYS.iter().copied())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )),
            );
            continue;
        }
        if map.insert(key, e).is_some() {
            diags.push(Diagnostic::error(Code::REF002, e.key.span, format!("キー `{key}` が重複しています")));
        }
    }
    if !map.contains_key("指標") {
        diags.push(amb(
            k.name.span,
            format!("知識 `{}` の形状を判定できません", k.name.name),
            "MVP で扱える知識は集計ランキング(指標 を持つ)だけです。指標 を定義してください",
        ));
        return None;
    }
    let mut ok = true;
    for (key, q) in REQUIRED_RANKING_KEYS {
        if !map.contains_key(key) {
            diags.push(amb(k.name.span, format!("知識 `{}` の業務決定 `{key}` が未確定です", k.name.name), q));
            ok = false;
        }
    }
    if !ok {
        return None;
    }
    let before = diags.len();
    let v = |key: &str| -> &KvEntry { map[key] };

    // 別名
    let aliases = match map.get("別名") {
        None => Vec::new(),
        Some(e) => parse_word_list(&e.value.text).unwrap_or_else(|| {
            diags.push(Diagnostic::error(Code::SYN001, e.value.span, "別名は `[名前, 名前]` の形で書きます"));
            Vec::new()
        }),
    };
    // 対象
    let src_e = v("対象");
    let source = src_e.value.text.clone();
    let Some(data) = env.data.get(&source) else {
        diags.push(Diagnostic::error(
            Code::REF001,
            src_e.value.span,
            format!("データ `{source}` は定義されていません"),
        ));
        return None;
    };
    // 期間
    let per = v("期間");
    let window_days = match parse_recent_days(&per.value.text) {
        Some(d) if (1..=3650).contains(&d) => d,
        Some(_) => {
            diags.push(Diagnostic::error(Code::AMB001, per.value.span, "期間は 1〜3650 日で指定してください"));
            0
        }
        None => {
            diags.push(amb(
                per.value.span,
                format!("期間 `{}` の意味が確定していません", per.value.text),
                "集計期間を日数で確定してください(例: 直近30日)",
            ));
            0
        }
    };
    // 日時列
    let tf = v("日時列");
    let time_field = tf.value.text.clone();
    match data.field(&time_field) {
        Some(Type::DateTime) => {}
        Some(t) => diags.push(Diagnostic::error(
            Code::TYP001,
            tf.value.span,
            format!("日時列 `{time_field}` の型は 日時 である必要がありますが {t} です"),
        )),
        None => diags.push(Diagnostic::error(
            Code::REF001,
            tf.value.span,
            format!("データ `{source}` にフィールド `{time_field}` はありません"),
        )),
    }
    // 時刻帯
    let tz = v("時刻帯");
    let time_zone = tz.value.text.clone();
    if !SUPPORTED_TIME_ZONES.iter().any(|(z, _)| *z == time_zone) {
        diags.push(
            Diagnostic::error(Code::TGT001, tz.value.span, format!("時刻帯 `{time_zone}` は未対応です"))
                .hint("MVP の対応時刻帯は Asia/Tokyo と UTC のみです(tz データベースの導入は ADR-0108)"),
        );
    }
    // 指標
    let me = v("指標");
    let metric = parse_metric(&me.value.text, data, me.value.span, diags);
    // 単位
    let un = v("単位");
    let unit = un.value.text.clone();
    let unit_ty = match data.field(&unit) {
        Some(Type::Option(_)) => {
            diags.push(amb(
                un.value.span,
                format!("単位 `{unit}` は 選択型のため NULL の扱いが未確定です"),
                "NULL の行を除外しますか、1 つのグループにまとめますか",
            ));
            Type::Error
        }
        Some(t) => t.clone(),
        None => {
            diags.push(Diagnostic::error(
                Code::REF001,
                un.value.span,
                format!("データ `{source}` にフィールド `{unit}` はありません"),
            ));
            Type::Error
        }
    };
    // 並び
    let ord = v("並び");
    let mut order = Vec::new();
    // 単位・指標が不正なときは連鎖エラーを避けるため並びの検査を省く
    let order_checkable = !unit_ty.is_error() && metric.is_some();
    for part in ord.value.text.split(',').map(|s| s.trim()).filter(|_| order_checkable) {
        let (col, dir) = if let Some(c) = part.strip_suffix("の降順") {
            (c, Dir::Desc)
        } else if let Some(c) = part.strip_suffix("の昇順") {
            (c, Dir::Asc)
        } else {
            diags.push(amb(
                ord.value.span,
                format!("並び `{part}` の意味が確定していません"),
                "`Xの降順` / `Xの昇順` の形で書いてください",
            ));
            continue;
        };
        let valid = metric.as_ref().is_some_and(|m| m.output == col) || col == unit;
        if !valid {
            diags.push(
                Diagnostic::error(Code::REF001, ord.value.span, format!("並びの `{col}` は集計結果の列ではありません")).hint(format!(
                    "使える列: {} / {}",
                    unit,
                    metric.as_ref().map(|m| m.output.as_str()).unwrap_or("?")
                )),
            );
            continue;
        }
        if order.iter().any(|o: &OrderKey| o.column == col) {
            diags.push(Diagnostic::error(Code::REF002, ord.value.span, format!("並びの `{col}` が重複しています")));
            continue;
        }
        order.push(OrderKey { column: col.to_string(), dir });
    }
    if order_checkable && !order.iter().any(|o| o.column == unit) {
        diags.push(amb(
            ord.value.span,
            "同順位のときの並びが確定していません",
            &format!("並びの最後に `{unit}の昇順` などを加えて順序を一意にしてください"),
        ));
    }
    // 件数
    let cnt = v("件数");
    let limit = match cnt.value.text.parse::<u32>() {
        Ok(n) if (1..=10000).contains(&n) => n,
        _ => {
            diags.push(Diagnostic::error(
                Code::AMB001,
                cnt.value.span,
                format!("件数 `{}` は 1〜10000 の整数で指定してください", cnt.value.text),
            ));
            0
        }
    };
    // 税区分
    let tx = v("税区分");
    let tax = match tx.value.text.as_str() {
        "税込" => Tax::Included,
        "税抜" => Tax::Excluded,
        other => {
            diags.push(amb(
                tx.value.span,
                format!("税区分 `{other}` の意味が確定していません"),
                "税込 / 税抜 のいずれかで確定してください",
            ));
            Tax::Included
        }
    };
    // 条件
    let co = v("条件");
    let filter = if co.value.text == "なし" {
        None
    } else {
        match kotonoha_parser::parse_expr_str(&co.value.text, co.value.span) {
            Err(d) => {
                diags.push(d);
                None
            }
            Ok(e) => {
                let vars = data.fields.clone();
                let (t, facts, ds) = check_expr_in(env, &vars, &e, Some(&Type::Bool));
                diags.extend(ds);
                if !t.same(&Type::Bool) {
                    diags.push(Diagnostic::error(
                        Code::TYP001,
                        co.value.span,
                        format!("条件は 真偽 でなければなりませんが {t} です"),
                    ));
                }
                if !facts.adapter_calls.is_empty() || !facts.action_calls.is_empty() || contains_call(&e) {
                    diags.push(Diagnostic::error(Code::TYP001, co.value.span, "条件式の中で呼出は使えません"));
                }
                Some(e)
            }
        }
    };
    if diags[before..].iter().any(|d| d.is_error()) {
        return None;
    }
    Some(Knowledge::Ranking(RankingSpec {
        name: k.name.name.clone(),
        aliases,
        source,
        window_days,
        time_field,
        time_zone,
        metric: metric?,
        unit,
        unit_ty,
        order,
        limit,
        tax,
        filter,
        span: k.span,
    }))
}

fn contains_call(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Call { .. } => true,
        ExprKind::Field(b, _) => contains_call(b),
        ExprKind::Unary { expr, .. } => contains_call(expr),
        ExprKind::Binary { lhs, rhs, .. } => contains_call(lhs) || contains_call(rhs),
        ExprKind::List(xs) => xs.iter().any(contains_call),
        _ => false,
    }
}

fn parse_metric(text: &str, data: &kotonoha_types::DataInfo, span: Span, diags: &mut Vec<Diagnostic>) -> Option<Metric> {
    if text == "件数" {
        return Some(Metric {
            agg: Agg::Count,
            field: None,
            output: "件数".into(),
            ty: Type::Int,
        });
    }
    let Some(field) = text.strip_suffix("の合計") else {
        diags.push(amb(
            span,
            format!("指標 `{text}` の意味が確定していません"),
            "MVP の指標は `Xの合計` か `件数` です(平均は端数処理が未決定)",
        ));
        return None;
    };
    match data.field(field) {
        None => {
            diags.push(Diagnostic::error(
                Code::REF001,
                span,
                format!("データ `{}` にフィールド `{field}` はありません", data.name),
            ));
            None
        }
        Some(Type::Option(_)) => {
            diags.push(amb(
                span,
                format!("指標 `{field}` は 選択型のため NULL の扱いが未確定です"),
                "NULL 金額の行を除外しますか、拒否(エラー)にしますか",
            ));
            None
        }
        Some(t @ (Type::Int | Type::Money(_))) => Some(Metric {
            agg: Agg::Sum,
            field: Some(field.to_string()),
            output: field.to_string(),
            ty: t.clone(),
        }),
        Some(Type::Decimal) => {
            diags.push(Diagnostic::error(Code::TYP001, span, "小数の合計は精度方針が未決定のため未対応です(ADR-0106)"));
            None
        }
        Some(t) => {
            diags.push(Diagnostic::error(Code::TYP001, span, format!("`{field}` の型 {t} は合計できません")));
            None
        }
    }
}

/// `直近30日` → 30
pub fn parse_recent_days(s: &str) -> Option<u32> {
    let n = s.strip_prefix("直近")?.strip_suffix("日")?;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) || (n.len() > 1 && n.starts_with('0')) {
        return None;
    }
    n.parse().ok()
}

fn parse_word_list(s: &str) -> Option<Vec<String>> {
    let inner = s.strip_prefix('[')?.strip_suffix(']')?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for w in inner.split(',').map(|x| x.trim()) {
        if w.is_empty() || !w.chars().all(kotonoha_lexer_word_char) || !seen.insert(w.to_string()) {
            return None;
        }
        out.push(w.to_string());
    }
    Some(out)
}

fn kotonoha_lexer_word_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests;
