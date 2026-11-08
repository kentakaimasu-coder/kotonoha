//! 意図層の解決(FR-002/FR-004/FR-006)。
//!
//! 画面・約束・動作本体の日本語文を、docs/spec/intent_patterns.md に列挙した**有限の文型**で照合する。
//! 照合できない文は推測せず AMB001(未解決意図)にして質問を提示する。
//! `@柔軟(確認|自動)` の宣言に限り、人が承認した解釈(正規文への書き換え)を適用できる。
//! 承認済み解釈も同じ文型照合を通るため、AI 出力が直接意味を確定することはない(SEC-08)。

use kotonoha_ast::{Decl, Expr, ExprKind, Item, RawText, SourceFile};
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_knowledge::{Knowledge, RankingSpec, Registry};
use kotonoha_types::{check_expr_in, Env, Type};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// 曖昧性モード(技術仕様書 §7)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[serde(rename = "厳密")]
    Strict,
    #[serde(rename = "柔軟(確認)")]
    FlexConfirm,
    #[serde(rename = "柔軟(自動)")]
    FlexAuto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "pattern")]
pub enum IntentKind {
    /// P-LIST `{データ}を一覧表示`。並べ替え `{データ}を{フィールド}(の新しい)順に一覧表示` と
    /// 絞り込み `{データ}のうち{条件}(かつ{条件})*のものを…一覧表示` を付けられる(ADR-0128)
    #[serde(rename = "list")]
    ListData {
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        order: Option<ListOrder>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        filters: Vec<ListFilter>,
    },
    /// P-FIELDS `{データ}の{フィールド}(と{フィールド})*を表示`
    #[serde(rename = "show_fields")]
    ShowFields { data: String, fields: Vec<String> },
    /// P-KNOWLEDGE `{知識}を表示` / `{知識}を{N}件表示`
    #[serde(rename = "show_knowledge")]
    ShowKnowledge {
        knowledge: String,
        limit_override: Option<u32>,
        #[serde(skip)]
        spec: Box<RankingSpec>,
    },
    /// P-NAV `{データ}を選ぶと {画面} へ`
    #[serde(rename = "navigate_on_select")]
    NavigateOnSelect { data: String, screen: String },
    /// P-BUTTON `「ラベル」を押すと {動作}(引数, ...)`
    #[serde(rename = "button_invoke")]
    ButtonInvoke {
        label: String,
        action: String,
        #[serde(serialize_with = "ser_args")]
        args: Vec<Expr>,
    },
    /// P-COMPONENT `{部品}(引数, …)を表示`(ADR-0123)
    #[serde(rename = "component")]
    Component {
        component: String,
        #[serde(serialize_with = "ser_args")]
        args: Vec<Expr>,
    },
    /// P-ANALYSIS `{分析}を表示`(引数なしの分析。ADR-0125)
    #[serde(rename = "analysis")]
    Analysis { analysis: String },
    /// P-AGGREGATE `{データ}を{フィールド}ごとに集計` / `{データ}の{F}と{F}を{フィールド}(の日|の月)ごとに合計`
    /// (件数と合計。SQL を書かずに集計する。ADR-0128)
    #[serde(rename = "aggregate")]
    Aggregate {
        data: String,
        by: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bucket: Option<Bucket>,
        sums: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        filters: Vec<ListFilter>,
    },
    /// P-FORM `{動作}のフォームを表示`(ADR-0115 提案)
    #[serde(rename = "form")]
    Form {
        action: String,
        /// 送信ボタンの文言(`{動作}のフォームを「申し込む」ボタンで表示`。無ければ「送信」。ADR-0129)
        #[serde(skip_serializing_if = "Option::is_none")]
        submit: Option<String>,
    },
    /// C-PUBLIC `{データ}は誰でも閲覧可`
    #[serde(rename = "public_read")]
    PublicRead { data: String },
    /// C-OWNER `{データ}は{フィールド}が本人のものだけ閲覧可`(行レベル認可。ADR-0118)
    #[serde(rename = "owner_read")]
    OwnerRead { data: String, field: String },
    /// C-UNIQUE `{データ}は{フィールド}(と{フィールド})*の組で重複不可` / `{データ}は{フィールド}が重複不可`(ADR-0129)
    /// 末尾に `(状態が取消のものを除く)` を付けると、その行は重複の判定から外す(部分一意索引)
    #[serde(rename = "unique")]
    Unique {
        data: String,
        fields: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        except: Vec<ListFilter>,
    },
    /// C-SEQUENCE `{データ}の{フィールド}は {値} → {値} → … の順`(列挙の値を決めた順にしか進めない。ADR-0129)
    /// 末尾に `(取消はいつでも)` を付けると、その値にはどの値からでも移れる(移ったら終わり)
    #[serde(rename = "sequence")]
    Sequence {
        data: String,
        field: String,
        values: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        anytime: Vec<String>,
    },
    /// C-AUTH `未認証なら{データ}不可`
    #[serde(rename = "auth_required")]
    AuthRequired { data: String },
}

/// 一覧の並べ替え(ADR-0128)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListOrder {
    pub field: String,
    pub desc: bool,
    /// 書かれた言い方(「注文日時の新しい」)。見出しに使う
    #[serde(skip)]
    pub label: String,
}

/// 日時の集計の単位(アプリの時刻帯での暦日・暦月)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Day,
    Month,
}

/// 集計の見出し(「注文の品名ごとの集計(状態が提供済み)」)
pub fn aggregate_caption(data: &str, by: &str, bucket: Option<Bucket>, filters: &[ListFilter]) -> String {
    let unit = match bucket {
        None => by.to_string(),
        Some(Bucket::Day) => format!("{by}の日"),
        Some(Bucket::Month) => format!("{by}の月"),
    };
    let base = format!("{data}の{unit}ごとの集計");
    let cond = list_caption(data, &None, filters);
    match cond.strip_prefix(&format!("{data}の一覧")) {
        Some(rest) if !rest.is_empty() => format!("{base}{rest}"),
        _ => base,
    }
}

/// 一覧の絞り込み条件の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// `{フィールド}が{値}`
    Eq,
    /// `{フィールド}が{値}でない`
    Ne,
    /// `{フィールド}が今日`(日時・日付。アプリの時刻帯の暦日)
    Today,
    /// `{フィールド}が本人`(識別子・文字列。ログイン中の人)
    Me,
    /// `{フィールド}が今日より前`(延滞など)
    BeforeToday,
    /// `{フィールド}が今日より後`
    AfterToday,
    /// `{フィールド}が今日以降`
    FromToday,
    /// `{フィールド}が今日まで`
    UntilToday,
}

/// 一覧の絞り込み(ADR-0128)。値は JSON の表記(数値・真偽・文字列)で持つ
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListFilter {
    pub field: String,
    pub op: FilterOp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl ListFilter {
    /// 一覧 API のクエリ(`eq.状態=調理中` など)
    pub fn query(&self) -> (String, String) {
        let op = match self.op {
            FilterOp::Eq => "eq",
            FilterOp::Ne => "ne",
            FilterOp::Today => "today",
            FilterOp::Me => "me",
            FilterOp::BeforeToday => "lt_today",
            FilterOp::AfterToday => "gt_today",
            FilterOp::FromToday => "ge_today",
            FilterOp::UntilToday => "le_today",
        };
        (format!("{op}.{}", self.field), self.value.clone().unwrap_or_else(|| "1".into()))
    }
}

/// 一覧 API に付けるクエリ(順序固定)
pub fn list_query(order: &Option<ListOrder>, filters: &[ListFilter]) -> Vec<(String, String)> {
    let mut q: Vec<(String, String)> = filters.iter().map(|f| f.query()).collect();
    if let Some(o) = order {
        q.push(("order".into(), o.field.clone()));
        if o.desc {
            q.push(("desc".into(), "1".into()));
        }
    }
    q
}

/// 条件の終わりの「のもの」(「調理中のもの」)と「もの」(「提供済みでないもの」)を外す
fn strip_mono(x: &str) -> Option<&str> {
    x.strip_suffix("のもの").or_else(|| x.strip_suffix("もの"))
}

fn shown(v: Option<&str>) -> &str {
    match v {
        Some("true") => "真",
        Some("false") => "偽",
        Some(x) => x,
        None => "",
    }
}

/// 一覧の説明(「注文(状態が調理中・注文日時の新しい順)」)
pub fn list_caption(data: &str, order: &Option<ListOrder>, filters: &[ListFilter]) -> String {
    let mut parts: Vec<String> = filters
        .iter()
        .map(|f| match f.op {
            FilterOp::Eq => format!("{}が{}", f.field, shown(f.value.as_deref())),
            FilterOp::Ne => format!("{}が{}でない", f.field, shown(f.value.as_deref())),
            FilterOp::Today => format!("{}が今日", f.field),
            FilterOp::Me => format!("{}が自分", f.field),
            FilterOp::BeforeToday => format!("{}が今日より前", f.field),
            FilterOp::AfterToday => format!("{}が今日より後", f.field),
            FilterOp::FromToday => format!("{}が今日以降", f.field),
            FilterOp::UntilToday => format!("{}が今日まで", f.field),
        })
        .collect();
    if let Some(o) = order {
        parts.push(format!("{}順", o.label));
    }
    if parts.is_empty() {
        format!("{data}の一覧")
    } else {
        format!("{data}の一覧({})", parts.join("・"))
    }
}

fn ser_args<S: serde::Serializer>(a: &[Expr], s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(a.len()))?;
    for e in a {
        seq.serialize_element(&kotonoha_parser::printer::expr_str(e))?;
    }
    seq.end()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "via", rename_all = "snake_case")]
pub enum Resolution {
    /// 文型に直接一致
    Pattern,
    /// 承認済み解釈(正規文)を適用
    Approved { canonical: String, approval_hash: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedIntent {
    pub id: String,
    pub owner: String,
    pub sentence: String,
    pub kind: IntentKind,
    pub resolution: Resolution,
    #[serde(skip)]
    pub span: Span,
}

/// 未解決の意図。resolver が候補生成の入力にする。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Unresolved {
    pub intent_id: String,
    pub owner: String,
    pub sentence: String,
    pub sentence_hash: String,
    pub mode: Mode,
    pub questions: Vec<String>,
    #[serde(skip)]
    pub span: Span,
}

/// 承認済み解釈(lock クレートの承認記録から渡される)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedInterpretation {
    pub intent_id: String,
    pub sentence_hash: String,
    pub canonical: String,
    pub approval_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScreenModel {
    pub name: String,
    pub mode: Mode,
    /// 遷移元から渡される「現在のエンティティ」のデータ名
    pub input: Option<String>,
    pub intents: Vec<ResolvedIntent>,
    #[serde(skip)]
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IntentModel {
    pub screens: Vec<ScreenModel>,
    pub contracts: Vec<ResolvedIntent>,
    pub unresolved: Vec<Unresolved>,
    /// 使われた承認の intent_id
    pub used_approvals: BTreeSet<String>,
}

pub fn sentence_hash(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn intent_id(owner: &str, sentence: &str, seen: &mut BTreeMap<String, u32>) -> String {
    let base = format!("{owner}#{}", &sentence_hash(sentence)[..12]);
    let n = seen.entry(base.clone()).or_insert(0);
    *n += 1;
    if *n == 1 {
        base
    } else {
        format!("{base}~{n}")
    }
}

fn mode_of(item: &Item, diags: &mut Vec<Diagnostic>) -> Mode {
    let mut mode = Mode::Strict;
    for a in &item.annotations {
        let m = match (a.name.name.as_str(), a.arg.as_ref().map(|x| x.name.as_str())) {
            ("厳密", None) => Mode::Strict,
            ("柔軟", Some("確認")) => Mode::FlexConfirm,
            ("柔軟", Some("自動")) => Mode::FlexAuto,
            _ => {
                diags.push(Diagnostic::error(Code::SYN001, a.span, "未知の注釈です").hint("@厳密 / @柔軟(確認) / @柔軟(自動) のいずれかです"));
                continue;
            }
        };
        mode = m;
    }
    let applicable = matches!(item.decl, Decl::Screen(_) | Decl::Contract(_) | Decl::Action(_));
    if !item.annotations.is_empty() && !applicable {
        diags.push(Diagnostic::error(
            Code::SYN001,
            item.annotations[0].span,
            "曖昧性モードの注釈は 画面 / 動作 / 約束 にだけ付けられます",
        ));
    }
    mode
}

struct Ctx<'a> {
    env: &'a Env,
    reg: &'a Registry,
    approvals: &'a [ApprovedInterpretation],
    diags: Vec<Diagnostic>,
    model: IntentModel,
}

/// 意図を解決する。
pub fn resolve_intents(files: &[&SourceFile], env: &Env, reg: &Registry, approvals: &[ApprovedInterpretation]) -> (IntentModel, Vec<Diagnostic>) {
    let mut cx = Ctx {
        env,
        reg,
        approvals,
        diags: Vec::new(),
        model: IntentModel::default(),
    };
    let mut seen = BTreeMap::new();
    for f in files {
        for item in &f.items {
            let mode = mode_of(item, &mut cx.diags);
            match &item.decl {
                Decl::Screen(s) => {
                    let owner = format!("screen.{}", s.name.name);
                    let mut intents = Vec::new();
                    for line in &s.lines {
                        let id = intent_id(&owner, &line.text, &mut seen);
                        if let Some(ri) = cx.resolve_line(&owner, id, line, mode, LineKind::Screen) {
                            intents.push(ri);
                        }
                    }
                    cx.model.screens.push(ScreenModel {
                        name: s.name.name.clone(),
                        mode,
                        input: None,
                        intents,
                        span: s.span,
                    });
                }
                Decl::Contract(c) => {
                    for line in &c.lines {
                        let id = intent_id("contract", &line.text, &mut seen);
                        if let Some(ri) = cx.resolve_line("contract", id, line, mode, LineKind::Contract) {
                            cx.model.contracts.push(ri);
                        }
                    }
                }
                Decl::Action(a) => {
                    let owner = format!("action.{}", a.name.name);
                    for it in &a.items {
                        if let kotonoha_ast::ActionItem::Intent(line) = it {
                            let id = intent_id(&owner, &line.text, &mut seen);
                            // 動作本体の意図文型は MVP では定義しない(常に未解決)
                            cx.resolve_line(&owner, id, line, mode, LineKind::Action);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    cx.link_screens();
    cx.check_button_args();
    let Ctx { diags, model, .. } = cx;
    (model, diags)
}

#[derive(Clone, Copy, PartialEq)]
enum LineKind {
    Screen,
    Contract,
    Action,
}

#[allow(clippy::enum_variant_names)]
enum Match {
    Ok(IntentKind),
    /// 文型に一致したが参照が不正(REF001 等)。未解決扱いにはしない。
    Invalid(Diagnostic),
    /// どの文型にも一致しない。質問の一覧を付ける。
    NoMatch(Vec<String>),
}

impl Ctx<'_> {
    fn resolve_line(&mut self, owner: &str, id: String, line: &RawText, mode: Mode, kind: LineKind) -> Option<ResolvedIntent> {
        let m = self.match_sentence(&line.text, line.span, kind);
        match m {
            Match::Ok(k) => Some(ResolvedIntent {
                id,
                owner: owner.into(),
                sentence: line.text.clone(),
                kind: k,
                resolution: Resolution::Pattern,
                span: line.span,
            }),
            Match::Invalid(d) => {
                self.diags.push(d);
                None
            }
            Match::NoMatch(questions) => {
                let shash = sentence_hash(&line.text);
                if mode != Mode::Strict {
                    let found = self
                        .approvals
                        .iter()
                        .find(|a| a.sentence_hash == shash && (a.intent_id == id || mode == Mode::FlexAuto));
                    if let Some(ap) = found {
                        self.model.used_approvals.insert(ap.intent_id.clone());
                        return match self.match_sentence(&ap.canonical, line.span, kind) {
                            Match::Ok(k) => Some(ResolvedIntent {
                                id,
                                owner: owner.into(),
                                sentence: line.text.clone(),
                                kind: k,
                                resolution: Resolution::Approved {
                                    canonical: ap.canonical.clone(),
                                    approval_hash: ap.approval_hash.clone(),
                                },
                                span: line.span,
                            }),
                            Match::Invalid(d) => {
                                self.diags.push(
                                    Diagnostic::error(
                                        Code::APP001,
                                        line.span,
                                        format!("承認済み解釈「{}」が現在のソースでは成立しません", ap.canonical),
                                    )
                                    .hint(d.message_ja),
                                );
                                None
                            }
                            Match::NoMatch(_) => {
                                self.diags.push(
                                    Diagnostic::error(Code::APP001, line.span, format!("承認済み解釈「{}」が既知の文型に一致しません", ap.canonical))
                                        .hint("承認記録は既知の文型の正規文でなければなりません"),
                                );
                                None
                            }
                        };
                    }
                }
                let mut d = Diagnostic::error(Code::AMB001, line.span, format!("「{}」を解釈できません(UNRESOLVED_INTENT)", line.text));
                // 宣言された名前から作った正しい文のうち、書いた文に近いものを示す(書き方を例から探さなくて済むように)
                let close = closest(&line.text, &example_sentences(self.env, self.reg, matches!(kind, LineKind::Contract)), 3);
                if !close.is_empty() {
                    d = d.hint(format!(
                        "近い書き方: {}",
                        close.iter().map(|c| format!("「{c}」")).collect::<Vec<_>>().join(" / ")
                    ));
                }
                for q in &questions {
                    d = d.hint(format!("質問: {q}"));
                }
                d = match mode {
                    Mode::Strict => d.hint("既知の文型(docs/spec/intent_patterns.md)で書き直すか、宣言に @柔軟(確認) を付けて解釈を承認してください"),
                    _ => d.hint(format!("koto resolve で候補を作成し、koto approve {id} <候補番号> で承認してください")),
                };
                self.diags.push(d);
                self.model.unresolved.push(Unresolved {
                    intent_id: id,
                    owner: owner.into(),
                    sentence: line.text.clone(),
                    sentence_hash: shash,
                    mode,
                    questions,
                    span: line.span,
                });
                None
            }
        }
    }

    fn match_sentence(&self, s: &str, span: Span, kind: LineKind) -> Match {
        match kind {
            LineKind::Action => Match::NoMatch(vec!["動作本体の日本語文は MVP では解釈しません。`厳密:` ブロックで処理を書いてください".into()]),
            LineKind::Contract => self.match_contract(s, span),
            LineKind::Screen => self.match_screen(s, span),
        }
    }

    fn match_contract(&self, s: &str, span: Span) -> Match {
        if let Some(rest) = s.strip_suffix("が本人のものだけ閲覧可") {
            if let Some((d, f)) = rest.split_once('は') {
                let Some(info) = self.env.data.get(d) else {
                    return Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータを指しますか")]);
                };
                return match info.field(f) {
                    Some(kotonoha_types::Type::Id | kotonoha_types::Type::Str) => Match::Ok(IntentKind::OwnerRead {
                        data: d.into(),
                        field: f.into(),
                    }),
                    Some(t) => Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("本人の判定に使うフィールド `{f}` は 識別子 か 文字列 である必要がありますが {t} です"),
                    )),
                    None => Match::Invalid(Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません"))),
                };
            }
        }
        // C-SEQUENCE(末尾の「(取消はいつでも)」は、どの値からでも移れる値)
        let (seq_s, anytime_text) = match s.strip_suffix("はいつでも)").and_then(|x| x.rsplit_once('(')) {
            Some((head, vals)) => (head, Some(vals)),
            None => (s, None),
        };
        if let Some(rest) = seq_s
            .strip_suffix("の順に進む")
            .or_else(|| seq_s.strip_suffix("の順に変わる"))
            .or_else(|| seq_s.strip_suffix("の順"))
        {
            if let Some((left, vals)) = rest.split_once('は') {
                if vals.contains('→') || vals.contains("->") {
                    let Some((d, f)) = self.split_data_prefix(left.trim()) else {
                        return Match::NoMatch(vec![format!(
                            "「{left}」は「データのフィールド」の形ではありません。どのデータのどの項目の順番ですか"
                        )]);
                    };
                    let info = &self.env.data[d];
                    let Some(t) = info.field(f) else {
                        return Match::Invalid(Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません")));
                    };
                    let Type::Enum(e) = t else {
                        return Match::Invalid(Diagnostic::error(
                            Code::TYP001,
                            span,
                            format!("順番を決められるのは列挙のフィールドです(`{f}` は {t})"),
                        ));
                    };
                    let values: Vec<String> = vals.replace("->", "→").split('→').map(|x| x.trim().to_string()).collect();
                    let all = &self.env.enums[e];
                    for v in &values {
                        if !all.contains(v) {
                            return Match::Invalid(
                                Diagnostic::error(Code::TYP001, span, format!("「{v}」は列挙 `{e}` の値ではありません"))
                                    .hint(format!("値: {}", all.join(" / "))),
                            );
                        }
                    }
                    let uniq: BTreeSet<&String> = values.iter().collect();
                    if uniq.len() != values.len() || values.len() < 2 {
                        return Match::Invalid(Diagnostic::error(Code::AMB002, span, "順番には 2 つ以上の異なる値を書きます"));
                    }
                    let anytime: Vec<String> = anytime_text
                        .map(|t| t.split(['・', '、']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                        .unwrap_or_default();
                    for v in &anytime {
                        if !all.contains(v) {
                            return Match::Invalid(
                                Diagnostic::error(Code::TYP001, span, format!("「{v}」は列挙 `{e}` の値ではありません"))
                                    .hint(format!("値: {}", all.join(" / "))),
                            );
                        }
                        if values.contains(v) {
                            return Match::Invalid(Diagnostic::error(Code::AMB002, span, format!("「{v}」は順番といつでもの両方に書かれています")));
                        }
                    }
                    let missing: Vec<&String> = all.iter().filter(|v| !values.contains(v) && !anytime.contains(v)).collect();
                    if !missing.is_empty() {
                        return Match::Invalid(
                            Diagnostic::error(
                                Code::AMB002,
                                span,
                                format!(
                                    "列挙 `{e}` の値 {} が順番に入っていません",
                                    missing.iter().map(|x| format!("「{x}」")).collect::<Vec<_>>().join("")
                                ),
                            )
                            .hint(format!(
                                "順番の途中に入らない値(取消など)は、末尾に「({}はいつでも)」と書くと、どの値からでも移れます",
                                missing[0]
                            )),
                        );
                    }
                    return Match::Ok(IntentKind::Sequence {
                        data: d.into(),
                        field: f.into(),
                        values,
                        anytime,
                    });
                }
            }
        }
        // C-UNIQUE(末尾の「(状態が取消のものを除く)」は判定から外す行)
        let (uniq_s, except_text) = match s.strip_suffix("を除く)").and_then(|x| x.rsplit_once("重複不可(")) {
            Some((head, cond)) => (format!("{head}重複不可"), Some(cond.to_string())),
            None => (s.to_string(), None),
        };
        if let Some(rest) = uniq_s.strip_suffix("重複不可") {
            if let Some((d, fs)) = rest.split_once('は') {
                let fs = fs
                    .strip_suffix("の組で")
                    .or_else(|| fs.strip_suffix("が"))
                    .or_else(|| fs.strip_suffix("で"))
                    .or_else(|| fs.strip_suffix("は"));
                if let Some(fs) = fs {
                    let Some(info) = self.env.data.get(d) else {
                        return Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータの重複を防ぎますか")]);
                    };
                    let mut fields: Vec<String> = Vec::new();
                    for f in fs.split('と') {
                        match info.field(f) {
                            None => return Match::Invalid(Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません"))),
                            Some(
                                Type::Bool
                                | Type::Int
                                | Type::Money(_)
                                | Type::Date
                                | Type::DateTime
                                | Type::Str
                                | Type::Id
                                | Type::Enum(_)
                                | Type::Email
                                | Type::Url,
                            ) => {}
                            Some(Type::Option(_)) => {
                                return Match::Invalid(Diagnostic::error(
                                    Code::TYP001,
                                    span,
                                    format!("選択<…> のフィールド `{f}` は重複の判定に使えません(空の値どうしは重複とみなされないため)"),
                                ))
                            }
                            Some(t) => {
                                return Match::Invalid(Diagnostic::error(
                                    Code::TYP001,
                                    span,
                                    format!("{t} のフィールド `{f}` は重複の判定に使えません"),
                                ))
                            }
                        }
                        if f == "ID" {
                            return Match::Invalid(Diagnostic::error(Code::TYP001, span, "ID は自動で重複しないため、書く必要はありません"));
                        }
                        if fields.iter().any(|x| x == f) {
                            return Match::Invalid(Diagnostic::error(Code::AMB002, span, format!("フィールド `{f}` が 2 回書かれています")));
                        }
                        fields.push(f.into());
                    }
                    let except = match &except_text {
                        None => vec![],
                        Some(t) => {
                            let Some(cond) = strip_mono(t) else {
                                return Match::Invalid(Diagnostic::error(Code::SYN001, span, "除く条件は「(状態が取消のものを除く)」の形で書きます"));
                            };
                            match self.parse_filters(d, info, Some(cond), span) {
                                Err(m) => return m,
                                Ok(f) if f.iter().all(|x| matches!(x.op, FilterOp::Eq | FilterOp::Ne)) => f,
                                Ok(_) => {
                                    return Match::Invalid(Diagnostic::error(
                                        Code::TYP001,
                                        span,
                                        "除く条件には「フィールドが値」「フィールドが値でない」だけを使えます",
                                    ))
                                }
                            }
                        }
                    };
                    return Match::Ok(IntentKind::Unique {
                        data: d.into(),
                        fields,
                        except,
                    });
                }
            }
        }
        if let Some(d) = s.strip_suffix("は誰でも閲覧可") {
            // 分析(集計)も、作者の判断で誰にでも公開できる(例: 待合室の呼び出し番号。本人限定データの集計でも可)
            let is_analysis = self.env.externals.get(d).is_some_and(|x| x.kind == kotonoha_ast::ExternalKind::Analysis);
            return if self.env.data.contains_key(d) || is_analysis {
                Match::Ok(IntentKind::PublicRead { data: d.into() })
            } else {
                Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータを指しますか")])
            };
        }
        // 「未認証なら{D}不可」と同じ意味の、普段の言い回し(ペルソナ評価で求められた)
        if let Some(d) = s.strip_suffix("はログインした人だけ閲覧可").or_else(|| s.strip_suffix("はログインした人だけ")) {
            return if self.env.data.contains_key(d) {
                Match::Ok(IntentKind::AuthRequired { data: d.into() })
            } else {
                Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータを指しますか")])
            };
        }
        if let Some(d) = s.strip_prefix("未認証なら").and_then(|x| x.strip_suffix("不可")) {
            return if self.env.data.contains_key(d) {
                Match::Ok(IntentKind::AuthRequired { data: d.into() })
            } else {
                Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータの操作を禁止しますか")])
            };
        }
        let _ = span;
        Match::NoMatch(vec![
            "約束の文型は「{データ}は誰でも閲覧可」「{データ}はログインした人だけ閲覧可」(=「未認証なら{データ}不可」)「{データ}は{フィールド}が本人のものだけ閲覧可」「{データ}は{フィールド}と{フィールド}の組で重複不可」です".into(),
            "この約束が守るべきデータ・操作・主体(本人/組織/役割)を具体的に指定してください".into(),
        ])
    }

    fn match_screen(&self, s: &str, span: Span) -> Match {
        // P-BUTTON
        if let Some(rest) = s.strip_prefix('「') {
            if let Some((label, after)) = rest.split_once("」を押すと ") {
                let call_text = after.trim();
                let offset = s.len() - call_text.len();
                let base = Span::new(
                    span.file,
                    span.line,
                    span.column + s[..offset].chars().count() as u32,
                    span.start + offset as u32,
                    span.end,
                );
                return match kotonoha_parser::parse_expr_str(call_text, base) {
                    Ok(Expr {
                        kind: ExprKind::Call { callee, args },
                        ..
                    }) => {
                        if self.env.actions.contains_key(&callee.name) {
                            Match::Ok(IntentKind::ButtonInvoke {
                                label: label.into(),
                                action: callee.name.clone(),
                                args,
                            })
                        } else {
                            Match::Invalid(Diagnostic::error(
                                Code::REF001,
                                callee.span,
                                format!("動作 `{}` は定義されていません", callee.name),
                            ))
                        }
                    }
                    Ok(e) => Match::Invalid(Diagnostic::error(
                        Code::SYN001,
                        e.span,
                        "「…」を押すと の後には動作の呼出 `動作名(引数)` を書きます",
                    )),
                    Err(d) => Match::Invalid(d),
                };
            }
        }
        // P-NAV
        if let Some((d, rest)) = s.split_once("を選ぶと ") {
            if let Some(scr) = rest.strip_suffix(" へ") {
                let scr = scr.trim();
                if !self.env.data.contains_key(d) {
                    return Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません")]);
                }
                if !self.env.screens.contains(scr) {
                    return Match::Invalid(Diagnostic::error(Code::REF001, span, format!("画面 `{scr}` は定義されていません")));
                }
                return Match::Ok(IntentKind::NavigateOnSelect {
                    data: d.into(),
                    screen: scr.into(),
                });
            }
        }
        // P-FORM(送信ボタンの文言付き)
        if let Some(x) = s.strip_suffix("ボタンで表示") {
            if let Some((a, label)) = x.strip_suffix('」').and_then(|y| y.split_once("のフォームを「")) {
                if !self.env.actions.contains_key(a) {
                    return Match::Invalid(Diagnostic::error(Code::REF001, span, format!("動作 `{a}` は定義されていません")));
                }
                if label.trim().is_empty() || label.chars().count() > 30 {
                    return Match::Invalid(Diagnostic::error(Code::SYN001, span, "ボタンの文言は 1〜30 文字で書きます"));
                }
                return Match::Ok(IntentKind::Form {
                    action: a.into(),
                    submit: Some(label.into()),
                });
            }
        }
        // P-AGGREGATE
        if let Some(rest) = s.strip_suffix("ごとに集計") {
            if let Some(m) = self.match_aggregate(rest, false, span) {
                return m;
            }
        }
        if let Some(rest) = s.strip_suffix("ごとに合計") {
            if let Some(m) = self.match_aggregate(rest, true, span) {
                return m;
            }
        }
        // P-LIST(並べ替え・絞り込み付きを含む)
        if let Some(rest) = s.strip_suffix("一覧表示") {
            if let Some(m) = self.match_list(rest, span) {
                return m;
            }
        }
        // P-KNOWLEDGE(件数上書き)
        if let Some(stem) = s.strip_suffix("件表示") {
            let digits: String = stem
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !digits.is_empty() {
                let before = &stem[..stem.len() - digits.len()];
                if let Some(k) = before.strip_suffix('を') {
                    return self.knowledge_match(k, Some(&digits), span);
                }
            }
        }
        if let Some(x) = s.strip_suffix("を表示") {
            // P-COMPONENT
            if let Some(open) = x.find('(') {
                let name = &x[..open];
                if let Some(sig) = self.env.externals.get(name) {
                    if sig.kind == kotonoha_ast::ExternalKind::Component && x.ends_with(')') {
                        let base = Span::new(span.file, span.line, span.column, span.start, span.end);
                        return match kotonoha_parser::parse_expr_str(x, base) {
                            Ok(Expr {
                                kind: ExprKind::Call { args, .. },
                                ..
                            }) => Match::Ok(IntentKind::Component { component: name.into(), args }),
                            Ok(_) => Match::Invalid(Diagnostic::error(Code::SYN001, span, "部品は `部品名(引数)を表示` の形で書きます")),
                            Err(d) => Match::Invalid(d),
                        };
                    }
                }
            }
            // P-ANALYSIS
            if let Some(sig) = self.env.externals.get(x) {
                return match sig.kind {
                    // 引数のある分析は、画面に条件の入力欄を付けて表示する(Web。ADR-0129)
                    kotonoha_ast::ExternalKind::Analysis => Match::Ok(IntentKind::Analysis { analysis: x.into() }),
                    kotonoha_ast::ExternalKind::Component => {
                        Match::Invalid(Diagnostic::error(Code::SYN001, span, format!("部品は `{x}(引数)を表示` の形で書きます")))
                    }
                    kotonoha_ast::ExternalKind::Function => Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("外部関数 `{x}` は画面に表示できません(動作の中で呼びます)"),
                    )),
                };
            }
            // P-FORM
            if let Some(a) = x.strip_suffix("のフォーム") {
                if self.env.actions.contains_key(a) {
                    return Match::Ok(IntentKind::Form {
                        action: a.into(),
                        submit: None,
                    });
                }
                return Match::Invalid(Diagnostic::error(Code::REF001, span, format!("動作 `{a}` は定義されていません")));
            }
            if self.reg.resolve(x).is_some() {
                return self.knowledge_match(x, None, span);
            }
            // P-FIELDS
            if let Some((d, fields_text)) = self.split_data_prefix(x) {
                let info = &self.env.data[d];
                let parts: Vec<&str> = fields_text.split('と').collect();
                let fields: Vec<String> = if parts.iter().all(|p| info.field(p).is_some()) {
                    parts.iter().map(|p| p.to_string()).collect()
                } else if info.field(fields_text).is_some() {
                    vec![fields_text.to_string()]
                } else {
                    let unknown: Vec<&str> = parts.iter().copied().filter(|p| info.field(p).is_none()).collect();
                    return Match::Invalid(
                        Diagnostic::error(
                            Code::REF001,
                            span,
                            format!(
                                "データ `{d}` にフィールド {} はありません",
                                unknown.iter().map(|u| format!("`{u}`")).collect::<Vec<_>>().join(", ")
                            ),
                        )
                        .hint(format!(
                            "フィールド: {}",
                            info.fields.iter().map(|f| f.0.as_str()).collect::<Vec<_>>().join(" / ")
                        )),
                    );
                };
                return Match::Ok(IntentKind::ShowFields { data: d.into(), fields });
            }
            let mut q = vec![format!("「{x}」は知識・データとして定義されていません。何を表示しますか")];
            if !self.reg.entries.is_empty() {
                q.push(format!("既存の知識: {}", self.reg.names.keys().cloned().collect::<Vec<_>>().join(" / ")));
            }
            q.push(format!(
                "新しい意味なら `知識 {x}:` を定義してください(期間・指標・件数などの業務決定が必要です)"
            ));
            return Match::NoMatch(q);
        }
        Match::NoMatch(vec![
            "画面の文型は docs/spec/intent_patterns.md の P-LIST / P-FIELDS / P-KNOWLEDGE / P-NAV / P-BUTTON です".into(),
        ])
    }

    /// `{データ}の…` のデータ名を最長一致で取り出す。
    /// `…一覧表示` の前半を照合する。一覧の文でなければ None。
    fn match_list(&self, rest: &str, span: Span) -> Option<Match> {
        let (target, order_text) = if let Some(x) = rest.strip_suffix("順に") {
            let (t, o) = x.rsplit_once('を')?;
            (t, Some(o))
        } else {
            (rest.strip_suffix('を')?, None)
        };
        let (d, conds) = match strip_mono(target).and_then(|x| x.split_once("のうち")) {
            Some((d, c)) => (d, Some(c)),
            None => (target, None),
        };
        let Some(info) = self.env.data.get(d) else {
            return Some(Match::NoMatch(vec![format!("「{d}」はデータとして定義されていません。どのデータの一覧ですか")]));
        };
        let field = |f: &str| -> Result<Type, Match> {
            info.field(f).cloned().ok_or_else(|| {
                Match::Invalid(
                    Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません")).hint(format!(
                        "フィールド: {}",
                        info.fields.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" / ")
                    )),
                )
            })
        };
        let mut order = None;
        if let Some(o) = order_text {
            const DESC: &[&str] = &["の新しい", "の大きい", "の多い", "の高い", "の遅い", "の降"];
            const ASC: &[&str] = &["の古い", "の小さい", "の少ない", "の安い", "の低い", "の早い", "の昇"];
            let (f, desc) = if let Some(f) = DESC.iter().find_map(|x| o.strip_suffix(x)) {
                (f, true)
            } else if let Some(f) = ASC.iter().find_map(|x| o.strip_suffix(x)) {
                (f, false)
            } else {
                (o, false)
            };
            match field(f) {
                Err(m) => return Some(m),
                Ok(Type::Bool | Type::Int | Type::Money(_) | Type::DateTime | Type::Date | Type::Str | Type::Id | Type::Enum(_) | Type::Email) => {}
                Ok(t) => {
                    return Some(Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("{t} のフィールド `{f}` では並べ替えできません"),
                    )))
                }
            }
            order = Some(ListOrder {
                field: f.into(),
                desc,
                label: o.into(),
            });
        }
        let filters = match self.parse_filters(d, info, conds, span) {
            Ok(f) => f,
            Err(m) => return Some(m),
        };
        Some(Match::Ok(IntentKind::ListData {
            data: d.into(),
            order,
            filters,
        }))
    }

    /// 絞り込み条件(`{F}が{値}` を `かつ` でつないだもの)
    fn parse_filters(&self, d: &str, info: &kotonoha_types::DataInfo, conds: Option<&str>, span: Span) -> Result<Vec<ListFilter>, Match> {
        let field = |f: &str| -> Result<Type, Match> {
            info.field(f).cloned().ok_or_else(|| {
                Match::Invalid(
                    Diagnostic::error(Code::REF001, span, format!("データ `{d}` にフィールド `{f}` はありません")).hint(format!(
                        "フィールド: {}",
                        info.fields.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" / ")
                    )),
                )
            })
        };
        let mut filters = Vec::new();
        for c in conds.map(|c| c.split("かつ").map(str::trim).collect::<Vec<_>>()).unwrap_or_default() {
            let Some((f, v)) = c.split_once('が').map(|(f, v)| (f.trim(), v.trim())) else {
                return Err(Match::Invalid(
                    Diagnostic::error(Code::SYN001, span, format!("条件「{c}」は「フィールドが値」の形で書きます"))
                        .hint(format!("例: 「{d}のうち状態が受付のものを一覧表示」「{d}のうち日時が今日のものを一覧表示」")),
                ));
            };
            let t = field(f)?;
            let base = match &t {
                Type::Option(x) => x.as_ref().clone(),
                x => x.clone(),
            };
            let rel = match v {
                "今日" => Some(FilterOp::Today),
                "今日より前" | "今日より以前" => Some(FilterOp::BeforeToday),
                "今日より後" | "今日より先" => Some(FilterOp::AfterToday),
                "今日以降" => Some(FilterOp::FromToday),
                "今日まで" | "今日以前" => Some(FilterOp::UntilToday),
                _ => None,
            };
            if let Some(op) = rel.filter(|o| *o != FilterOp::Today) {
                if !matches!(base, Type::DateTime | Type::Date) {
                    return Err(Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("「{f}が{v}」は日時か日付のフィールドに使います(`{f}` は {t})"),
                    )));
                }
                filters.push(ListFilter {
                    field: f.into(),
                    op,
                    value: None,
                });
                continue;
            }
            if v == "今日" {
                if !matches!(base, Type::DateTime | Type::Date) {
                    return Err(Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("「{f}が今日」は日時か日付のフィールドに使います(`{f}` は {t})"),
                    )));
                }
                filters.push(ListFilter {
                    field: f.into(),
                    op: FilterOp::Today,
                    value: None,
                });
                continue;
            }
            if v == "本人" {
                if !matches!(base, Type::Id | Type::Str) {
                    return Err(Match::Invalid(Diagnostic::error(
                        Code::TYP001,
                        span,
                        format!("「{f}が本人」は識別子か文字列のフィールドに使います(`{f}` は {t})"),
                    )));
                }
                filters.push(ListFilter {
                    field: f.into(),
                    op: FilterOp::Me,
                    value: None,
                });
                continue;
            }
            let (v, op) = match v.strip_suffix("でない").or_else(|| v.strip_suffix("ではない")) {
                Some(x) => (x, FilterOp::Ne),
                None => (v, FilterOp::Eq),
            };
            let raw = v.strip_prefix('「').and_then(|x| x.strip_suffix('」')).unwrap_or(v);
            let value = match &base {
                Type::Enum(e) => {
                    if !self.env.enum_has(e, raw) {
                        return Err(Match::Invalid(
                            Diagnostic::error(Code::TYP001, span, format!("「{raw}」は列挙 `{e}` の値ではありません"))
                                .hint(format!("値: {}", self.env.enums[e].join(" / "))),
                        ));
                    }
                    raw.to_string()
                }
                Type::Bool => match raw {
                    "真" | "はい" => "true".into(),
                    "偽" | "いいえ" => "false".into(),
                    _ => {
                        return Err(Match::Invalid(Diagnostic::error(
                            Code::TYP001,
                            span,
                            format!("真偽のフィールド `{f}` の値は 真 / 偽 です"),
                        )))
                    }
                },
                Type::Int | Type::Money(_) => {
                    let n: String = raw
                        .chars()
                        .map(|c| {
                            if ('０'..='９').contains(&c) {
                                char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap()
                            } else {
                                c
                            }
                        })
                        .filter(|c| *c != ',')
                        .collect();
                    match n.parse::<i64>() {
                        Ok(x) if x.unsigned_abs() <= (1u64 << 53) => x.to_string(),
                        _ => {
                            return Err(Match::Invalid(Diagnostic::error(
                                Code::TYP001,
                                span,
                                format!("`{f}` は整数のフィールドです(「{raw}」は整数ではありません)"),
                            )))
                        }
                    }
                }
                Type::Str | Type::Id | Type::Email | Type::Url => raw.to_string(),
                Type::Date => {
                    let ok = raw.len() == 10 && raw.as_bytes()[4] == b'-' && raw.as_bytes()[7] == b'-';
                    if !ok {
                        return Err(Match::Invalid(Diagnostic::error(
                            Code::TYP001,
                            span,
                            format!("日付の値は 2026-12-05 の形で書きます(「{raw}」)"),
                        )));
                    }
                    raw.to_string()
                }
                _ => {
                    return Err(Match::Invalid(
                        Diagnostic::error(Code::TYP001, span, format!("{t} のフィールド `{f}` は値で絞り込めません")).hint("日時は「…が今日」で絞り込めます"),
                    ))
                }
            };
            filters.push(ListFilter {
                field: f.into(),
                op,
                value: Some(value),
            });
        }
        Ok(filters)
    }

    /// `…ごとに集計` / `…ごとに合計` の前半(`{対象}を{単位}`)を照合する
    fn match_aggregate(&self, rest: &str, explicit: bool, span: Span) -> Option<Match> {
        let (target, by_text) = rest.rsplit_once('を')?;
        // 対象: {D} / {D}のうち{条件}のもの(集計)、{D}の{F}と{F} / {D}のうち{条件}のものの{F}と{F}(合計)
        let (d, conds, sums_text) = if explicit {
            if let Some((d, r)) = target.split_once("のうち") {
                let (c, f) = r.rsplit_once("のものの").or_else(|| r.rsplit_once("ものの"))?;
                (d, Some(c), Some(f))
            } else {
                let (d, f) = self.split_data_prefix(target)?;
                (d, None, Some(f))
            }
        } else {
            match strip_mono(target).and_then(|x| x.split_once("のうち")) {
                Some((d, c)) => (d, Some(c), None),
                None => (target, None, None),
            }
        };
        let Some(info) = self.env.data.get(d) else {
            return Some(Match::NoMatch(vec![format!(
                "「{d}」はデータとして定義されていません。どのデータを集計しますか"
            )]));
        };
        let (by, bucket) = if let Some(f) = by_text.strip_suffix("の日") {
            (f, Some(Bucket::Day))
        } else if let Some(f) = by_text.strip_suffix("の月") {
            (f, Some(Bucket::Month))
        } else {
            (by_text, None)
        };
        let base = |t: &Type| match t {
            Type::Option(x) => x.as_ref().clone(),
            x => x.clone(),
        };
        let Some(bt) = info.field(by).map(base) else {
            return Some(Match::Invalid(Diagnostic::error(
                Code::REF001,
                span,
                format!("データ `{d}` にフィールド `{by}` はありません"),
            )));
        };
        if bucket.is_some() && !matches!(bt, Type::DateTime | Type::Date) {
            return Some(Match::Invalid(Diagnostic::error(
                Code::TYP001,
                span,
                format!("「{by_text}ごと」は日時か日付のフィールドに使います(`{by}` は {bt})"),
            )));
        }
        if bucket.is_none() && matches!(bt, Type::DateTime) {
            return Some(Match::Invalid(
                Diagnostic::error(Code::TYP001, span, format!("日時 `{by}` はそのままでは集計できません"))
                    .hint(format!("「{by}の日ごとに集計」「{by}の月ごとに集計」と書きます")),
            ));
        }
        if !matches!(
            bt,
            Type::Bool | Type::Int | Type::Money(_) | Type::DateTime | Type::Date | Type::Str | Type::Id | Type::Enum(_) | Type::Email
        ) {
            return Some(Match::Invalid(Diagnostic::error(
                Code::TYP001,
                span,
                format!("{bt} のフィールド `{by}` ごとには集計できません"),
            )));
        }
        let numeric = |t: &Type| matches!(base(t), Type::Int | Type::Money(_));
        let sums: Vec<String> = match sums_text {
            None => info.fields.iter().filter(|(n, t)| n != by && numeric(t)).map(|(n, _)| n.clone()).collect(),
            Some(text) => {
                let mut v = Vec::new();
                for f in text.split('と') {
                    match info.field(f) {
                        None => {
                            return Some(Match::Invalid(Diagnostic::error(
                                Code::REF001,
                                span,
                                format!("データ `{d}` にフィールド `{f}` はありません"),
                            )))
                        }
                        Some(t) if !numeric(t) => {
                            return Some(Match::Invalid(Diagnostic::error(
                                Code::TYP001,
                                span,
                                format!("合計できるのは整数か金額のフィールドです(`{f}` は {t})"),
                            )))
                        }
                        Some(_) => v.push(f.to_string()),
                    }
                }
                v
            }
        };
        let filters = match self.parse_filters(d, info, conds, span) {
            Ok(f) => f,
            Err(m) => return Some(m),
        };
        Some(Match::Ok(IntentKind::Aggregate {
            data: d.into(),
            by: by.into(),
            bucket,
            sums,
            filters,
        }))
    }

    fn split_data_prefix<'s>(&self, x: &'s str) -> Option<(&'s str, &'s str)> {
        let mut best: Option<(&str, &str)> = None;
        for (i, _) in x.match_indices('の') {
            let (d, rest) = (&x[..i], &x[i + 'の'.len_utf8()..]);
            if self.env.data.contains_key(d) && !rest.is_empty() && best.is_none_or(|(b, _)| d.len() > b.len()) {
                best = Some((d, rest));
            }
        }
        best
    }

    fn knowledge_match(&self, name: &str, digits: Option<&str>, span: Span) -> Match {
        let Some(k) = self.reg.resolve(name) else {
            let mut q = vec![format!("「{name}」は知識として定義されていません。何を指しますか")];
            if !self.reg.entries.is_empty() {
                q.push(format!("既存の知識: {}", self.reg.names.keys().cloned().collect::<Vec<_>>().join(" / ")));
            }
            return Match::NoMatch(q);
        };
        let Knowledge::Ranking(r) = k;
        let mut spec = r.clone();
        let limit_override = match digits {
            None => None,
            Some(ds) => match ds.parse::<u32>() {
                Ok(n) if (1..=10000).contains(&n) && !(ds.len() > 1 && ds.starts_with('0')) => {
                    spec.limit = n;
                    Some(n)
                }
                _ => {
                    return Match::Invalid(Diagnostic::error(
                        Code::AMB001,
                        span,
                        format!("件数 `{ds}` は 1〜10000 の整数で指定してください"),
                    ))
                }
            },
        };
        Match::Ok(IntentKind::ShowKnowledge {
            knowledge: r.name.clone(),
            limit_override,
            spec: Box::new(spec),
        })
    }

    /// 画面間の遷移から各画面の入力(現在のエンティティ)を決め、必要条件を検査する。
    fn link_screens(&mut self) {
        let mut inputs: BTreeMap<String, Vec<(String, Span)>> = BTreeMap::new();
        for s in &self.model.screens {
            let listed: BTreeSet<&str> = s
                .intents
                .iter()
                .filter_map(|i| {
                    if let IntentKind::ListData { data, .. } = &i.kind {
                        Some(data.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            for i in &s.intents {
                if let IntentKind::NavigateOnSelect { data, screen } = &i.kind {
                    if !listed.contains(data.as_str()) {
                        self.diags.push(
                            Diagnostic::error(Code::REF001, i.span, format!("画面 `{}` に {data} の一覧がないため選択できません", s.name))
                                .hint(format!("同じ画面に `{data}を一覧表示` が必要です")),
                        );
                    }
                    inputs.entry(screen.clone()).or_default().push((data.clone(), i.span));
                }
            }
        }
        for s in &mut self.model.screens {
            let provided = inputs.get(&s.name).cloned().unwrap_or_default();
            let kinds: BTreeSet<&String> = provided.iter().map(|p| &p.0).collect();
            if kinds.len() > 1 {
                self.diags.push(Diagnostic::error(
                    Code::TYP001,
                    s.span,
                    format!(
                        "画面 `{}` への遷移が異なるデータ({})を渡しています",
                        s.name,
                        kinds.into_iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                ));
                continue;
            }
            s.input = provided.first().map(|p| p.0.clone());
            for i in &s.intents {
                if let IntentKind::ShowFields { data, .. } = &i.kind {
                    if s.input.as_deref() != Some(data.as_str()) {
                        self.diags.push(
                            Diagnostic::error(Code::REF001, i.span, format!("画面 `{}` で表示する {data} がどれか決まりません", s.name))
                                .hint(format!("別の画面から `{data}を選ぶと {} へ` で遷移させてください", s.name)),
                        );
                    }
                }
            }
        }
    }

    fn check_button_args(&mut self) {
        for s in &self.model.screens {
            let vars: Vec<(String, Type)> = s.input.iter().map(|d| (d.clone(), Type::Data(d.clone()))).collect();
            for i in &s.intents {
                let (action, args, params) = match &i.kind {
                    IntentKind::ButtonInvoke { action, args, .. } => match self.env.actions.get(action) {
                        Some(sig) => (action, args, sig.params.clone()),
                        None => continue,
                    },
                    IntentKind::Component { component, args } => match self.env.externals.get(component) {
                        Some(sig) => (component, args, sig.params.clone()),
                        None => continue,
                    },
                    _ => continue,
                };
                {
                    struct Sig {
                        params: Vec<(String, Type)>,
                    }
                    let sig = Sig { params };
                    if sig.params.len() != args.len() {
                        self.diags.push(Diagnostic::error(
                            Code::TYP001,
                            i.span,
                            format!("動作 `{action}` の引数は {} 個ですが {} 個渡されています", sig.params.len(), args.len()),
                        ));
                        continue;
                    }
                    for (a, (_, pt)) in args.iter().zip(&sig.params) {
                        let (t, facts, ds) = check_expr_in(self.env, &vars, a, Some(pt));
                        self.diags.extend(ds);
                        if !facts.adapter_calls.is_empty() || !facts.action_calls.is_empty() {
                            self.diags.push(Diagnostic::error(Code::TYP001, a.span, "画面の引数式の中で呼出は使えません"));
                        } else if !t.same(pt) {
                            self.diags
                                .push(Diagnostic::error(Code::TYP001, a.span, format!("型が一致しません: {pt} が必要ですが {t} です")));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// 宣言された名前で書ける正しい文の例(`koto patterns` と「近い書き方」の提案に使う)。
/// contract=true なら約束の文、false なら画面の文。
pub fn example_sentences(env: &Env, reg: &Registry, contract: bool) -> Vec<String> {
    let mut out = Vec::new();
    if contract {
        for (d, info) in &env.data {
            out.push(format!("{d}は誰でも閲覧可"));
            out.push(format!("{d}はログインした人だけ閲覧可"));
            let keys: Vec<&str> = info
                .fields
                .iter()
                .filter(|(f, t)| f != "ID" && matches!(t, kotonoha_types::Type::Id | kotonoha_types::Type::Str))
                .map(|(f, _)| f.as_str())
                .take(2)
                .collect();
            if !keys.is_empty() {
                out.push(format!("{d}は{}の組で重複不可", keys.join("と")));
            }
            for (f, t) in &info.fields {
                if matches!(t, kotonoha_types::Type::Id) && f != "ID" {
                    out.push(format!("{d}は{f}が本人のものだけ閲覧可"));
                }
            }
        }
        return out;
    }
    for (d, info) in &env.data {
        out.push(format!("{d}を一覧表示"));
        for (f, t) in &info.fields {
            match t {
                kotonoha_types::Type::DateTime | kotonoha_types::Type::Date => {
                    out.push(format!("{d}を{f}の新しい順に一覧表示"));
                    out.push(format!("{d}のうち{f}が今日のものを一覧表示"));
                    out.push(format!("{d}を{f}の日ごとに集計"));
                    out.push(format!("{d}のうち{f}が今日より前のものを一覧表示"));
                    if let Some((ef, ev)) = info.fields.iter().find_map(|(n, t)| match t {
                        kotonoha_types::Type::Enum(e) => env.enums.get(e).and_then(|vs| vs.first()).map(|v| (n, v)),
                        _ => None,
                    }) {
                        out.push(format!("{d}のうち{ef}が{ev}でないかつ{f}が今日以降のものを{f}順に一覧表示"));
                    }
                }
                kotonoha_types::Type::Enum(e) => {
                    if let Some(v) = env.enums.get(e).and_then(|vs| vs.first()) {
                        out.push(format!("{d}のうち{f}が{v}のものを一覧表示"));
                    }
                    out.push(format!("{d}を{f}ごとに集計"));
                }
                _ => {}
            }
        }
        let fs: Vec<&str> = info.fields.iter().map(|(f, _)| f.as_str()).filter(|f| *f != "ID").collect();
        if !fs.is_empty() {
            out.push(format!("{d}の{}を表示", fs.join("と")));
        }
        for s in &env.screens {
            out.push(format!("{d}を選ぶと {s} へ"));
        }
    }
    for k in reg.entries.keys() {
        out.push(format!("{k}を表示"));
        out.push(format!("{k}を5件表示"));
    }
    for (a, sig) in &env.actions {
        out.push(format!("{a}のフォームを表示"));
        let args: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
        out.push(format!("「{a}」を押すと {a}({})", args.join(", ")));
    }
    for (n, x) in &env.externals {
        match x.kind {
            kotonoha_ast::ExternalKind::Analysis if x.params.is_empty() => out.push(format!("{n}を表示")),
            kotonoha_ast::ExternalKind::Component => out.push(format!(
                "{n}({})を表示",
                x.params.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ")
            )),
            _ => {}
        }
    }
    out
}

/// 文字の 2-gram の重なり(Dice 係数)で近い順に n 件。似ていないもの(0.25 未満)は出さない。
pub fn closest(text: &str, candidates: &[String], n: usize) -> Vec<String> {
    fn grams(s: &str) -> Vec<(char, char)> {
        let cs: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
        cs.windows(2).map(|w| (w[0], w[1])).collect()
    }
    let a = grams(text);
    if a.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(u32, &String)> = candidates
        .iter()
        .map(|c| {
            let b = grams(c);
            let mut used = vec![false; b.len()];
            let mut hit = 0;
            for g in &a {
                if let Some(i) = b.iter().enumerate().position(|(i, x)| !used[i] && x == g) {
                    used[i] = true;
                    hit += 1;
                }
            }
            let score = (2 * hit * 1000 / (a.len() + b.len()).max(1)) as u32;
            (score, c)
        })
        .filter(|(s, _)| *s >= 250)
        .collect();
    scored.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(y.1)));
    scored.into_iter().take(n).map(|(_, c)| c.clone()).collect()
}
