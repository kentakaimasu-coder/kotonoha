//! App IR: VAG(意味)から導いた、全ターゲット共通のアプリケーション表現(docs/architecture.md §3)。
//!
//! - `entities` / `enums`: データモデル(Backend の表、各クライアントのモデル型)
//! - `endpoints`: 共通 API 契約。Web/iOS/Android はすべてこの API だけを使う(業務ロジックはサーバーに 1 つ)
//! - `screens`: UI IR(Page / List / Detail / Query / Button / Form / Navigation)
//! - `actions`: 厳密層の本体(Backend だけが実行する)
//!
//! App IR はターゲット非依存。ターゲット固有の判断(フレームワーク・OS 権限)は各生成器が行う。

use kotonoha_ast::{ActionDecl, Block, Decl, Expr, SourceFile};
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_intent::{IntentKind, IntentModel};
use kotonoha_knowledge::{Knowledge, RankingSpec, Registry};
use kotonoha_types::{AdapterSig, Capability, Env, Type, Verb};
use kotonoha_validator::{AccessPolicy, Validation};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// 未認証でも可(約束 C-PUBLIC)
    Public,
    /// 認証済みで、全 Capability を保持していること
    Caps(Vec<Capability>),
}

#[derive(Debug, Clone)]
pub enum EndpointKind {
    ListData { data: String },
    GetData { data: String },
    Query { knowledge: String, limits: Vec<u32> },
    Action { action: String },
}

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub id: String,
    pub method: &'static str,
    /// パスの各セグメント(日本語はクライアント側で percent-encode する)
    pub segments: Vec<String>,
    /// `{id}` を含むか
    pub has_id: bool,
    pub kind: EndpointKind,
    pub access: Access,
}

impl Endpoint {
    /// 表示用パス(`/api/data/商品/{id}`)
    pub fn path_display(&self) -> String {
        let mut p = String::new();
        for s in &self.segments {
            p.push('/');
            p.push_str(s);
        }
        if self.has_id {
            p.push_str("/{id}");
        }
        p
    }
}

#[derive(Debug, Clone)]
pub enum Component {
    List {
        data: String,
        fields: Vec<(String, Type)>,
        link_to: Option<String>,
    },
    Detail {
        data: String,
        fields: Vec<(String, Type)>,
    },
    Query {
        knowledge: String,
        limit: u32,
        columns: Vec<(String, Type)>,
    },
    Button {
        label: String,
        action: String,
        args: Vec<Expr>,
    },
    Form {
        action: String,
        params: Vec<(String, Type)>,
    },
}

#[derive(Debug, Clone)]
pub struct ScreenIr {
    pub name: String,
    pub input: Option<String>,
    pub components: Vec<Component>,
}

#[derive(Debug, Clone)]
pub struct ActionIr {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Option<Type>,
    pub caps: Vec<Capability>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AppIr {
    pub name: String,
    pub entities: Vec<Entity>,
    pub enums: BTreeMap<String, Vec<String>>,
    pub adapters: Vec<AdapterSig>,
    pub actions: Vec<ActionIr>,
    pub queries: Vec<(RankingSpec, Vec<u32>)>,
    pub endpoints: Vec<Endpoint>,
    pub screens: Vec<ScreenIr>,
    /// 入力を持たない最初の画面
    pub entry_screen: Option<String>,
    pub policy: AccessPolicy,
}

pub struct IrInput<'a> {
    pub env: &'a Env,
    pub registry: &'a Registry,
    pub intents: &'a IntentModel,
    pub validation: &'a Validation,
    pub asts: &'a [SourceFile],
}

/// 一覧・詳細で表示する列: 宣言順の全フィールドから `ID` を除く(ADR-0116 提案)
fn display_fields(env: &Env, d: &str) -> Vec<(String, Type)> {
    env.data
        .get(d)
        .map(|x| x.fields.iter().filter(|(n, _)| n != "ID").cloned().collect())
        .unwrap_or_default()
}

fn read_access(policy: &AccessPolicy, data: &str) -> Access {
    if policy.public_read.contains(data) {
        Access::Public
    } else {
        Access::Caps(vec![Capability::new(data, Verb::Read)])
    }
}

pub fn build_ir(inp: &IrInput) -> AppIr {
    let env = inp.env;
    let policy = inp.validation.policy.clone();
    let mut screens = Vec::new();
    let mut list_data = BTreeSet::new();
    let mut get_data = BTreeSet::new();
    let mut query_limits: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for s in &inp.intents.screens {
        if let Some(d) = &s.input {
            get_data.insert(d.clone());
        }
        let nav: BTreeMap<String, String> = s
            .intents
            .iter()
            .filter_map(|i| {
                if let IntentKind::NavigateOnSelect { data, screen } = &i.kind {
                    Some((data.clone(), screen.clone()))
                } else {
                    None
                }
            })
            .collect();
        let mut comps = Vec::new();
        for i in &s.intents {
            match &i.kind {
                IntentKind::ListData { data } => {
                    list_data.insert(data.clone());
                    comps.push(Component::List {
                        data: data.clone(),
                        fields: display_fields(env, data),
                        link_to: nav.get(data).cloned(),
                    });
                }
                IntentKind::ShowFields { data, fields } => {
                    let info = &env.data[data];
                    comps.push(Component::Detail {
                        data: data.clone(),
                        fields: fields.iter().map(|f| (f.clone(), info.field(f).cloned().unwrap_or(Type::Error))).collect(),
                    });
                }
                IntentKind::ShowKnowledge { knowledge, spec, .. } => {
                    query_limits.entry(knowledge.clone()).or_default().insert(spec.limit);
                    let Type::Record(cols) = spec.row_type() else { unreachable!() };
                    comps.push(Component::Query {
                        knowledge: knowledge.clone(),
                        limit: spec.limit,
                        columns: cols,
                    });
                }
                IntentKind::ButtonInvoke { label, action, args } => comps.push(Component::Button {
                    label: label.clone(),
                    action: action.clone(),
                    args: args.clone(),
                }),
                IntentKind::Form { action } => comps.push(Component::Form {
                    action: action.clone(),
                    params: env.actions.get(action).map(|a| a.params.clone()).unwrap_or_default(),
                }),
                IntentKind::NavigateOnSelect { .. } | IntentKind::PublicRead { .. } | IntentKind::AuthRequired { .. } => {}
            }
        }
        screens.push(ScreenIr {
            name: s.name.clone(),
            input: s.input.clone(),
            components: comps,
        });
    }

    let mut endpoints = Vec::new();
    for d in &list_data {
        endpoints.push(Endpoint {
            id: format!("data.list.{d}"),
            method: "GET",
            segments: vec!["api".into(), "data".into(), d.clone()],
            has_id: false,
            kind: EndpointKind::ListData { data: d.clone() },
            access: read_access(&policy, d),
        });
    }
    for d in &get_data {
        endpoints.push(Endpoint {
            id: format!("data.get.{d}"),
            method: "GET",
            segments: vec!["api".into(), "data".into(), d.clone()],
            has_id: true,
            kind: EndpointKind::GetData { data: d.clone() },
            access: read_access(&policy, d),
        });
    }
    let mut queries = Vec::new();
    for (k, limits) in &query_limits {
        let Some(Knowledge::Ranking(spec)) = inp.registry.entries.get(k) else {
            continue;
        };
        let limits: Vec<u32> = limits.iter().copied().collect();
        endpoints.push(Endpoint {
            id: format!("query.{k}"),
            method: "GET",
            segments: vec!["api".into(), "queries".into(), k.clone()],
            has_id: false,
            kind: EndpointKind::Query {
                knowledge: k.clone(),
                limits: limits.clone(),
            },
            access: read_access(&policy, &spec.source),
        });
        queries.push((spec.clone(), limits));
    }
    // 動作は全て公開 API(呼出元が画面かどうかに関わらず、サーバーで Capability を強制する)
    let mut actions = Vec::new();
    let decls: BTreeMap<String, &ActionDecl> = inp
        .asts
        .iter()
        .flat_map(|f| f.items.iter())
        .filter_map(|i| {
            if let Decl::Action(a) = &i.decl {
                Some((a.name.name.clone(), a))
            } else {
                None
            }
        })
        .collect();
    for (name, sig) in &env.actions {
        let caps: Vec<Capability> = inp
            .validation
            .actions
            .get(name)
            .map(|s| s.capabilities.iter().cloned().collect())
            .unwrap_or_default();
        endpoints.push(Endpoint {
            id: format!("action.{name}"),
            method: "POST",
            segments: vec!["api".into(), "actions".into(), name.clone()],
            has_id: false,
            kind: EndpointKind::Action { action: name.clone() },
            access: Access::Caps(caps.clone()),
        });
        actions.push(ActionIr {
            name: name.clone(),
            params: sig.params.clone(),
            ret: sig.ret.clone(),
            caps,
            body: decls.get(name).and_then(|a| a.strict_blocks().next().cloned()),
            span: sig.span,
        });
    }
    let entry_screen = screens.iter().find(|s| s.input.is_none()).map(|s| s.name.clone());
    AppIr {
        name: env.app_name.clone().unwrap_or_default(),
        entities: env
            .data
            .values()
            .map(|d| Entity {
                name: d.name.clone(),
                fields: d.fields.clone(),
            })
            .collect(),
        enums: env.enums.clone(),
        adapters: env.adapters.values().cloned().collect(),
        actions,
        queries,
        endpoints,
        screens,
        entry_screen,
        policy,
    }
}

/// ワイヤ形式(JSON)での型。全クライアントが同じ規則で符号化・復号する(docs/architecture.md §4)。
pub fn wire_type(t: &Type) -> Result<Value, String> {
    Ok(match t {
        Type::Bool => json!({"kind": "bool"}),
        Type::Int => json!({"kind": "int"}),
        Type::Str => json!({"kind": "string"}),
        Type::Id => json!({"kind": "id"}),
        Type::Url => json!({"kind": "url"}),
        Type::Email => json!({"kind": "email"}),
        Type::Money(c) if c == "JPY" => json!({"kind": "money", "currency": "JPY", "minor_units": 0}),
        Type::DateTime => json!({"kind": "datetime"}),
        Type::Enum(e) => json!({"kind": "enum", "name": e}),
        Type::Data(d) => json!({"kind": "entity", "name": d}),
        Type::Option(inner) => json!({"kind": "optional", "of": wire_type(inner)?}),
        Type::List(inner) | Type::Array(inner) => json!({"kind": "list", "of": wire_type(inner)?}),
        Type::Record(fs) => {
            json!({"kind": "record", "fields": fs.iter().map(|(n, t)| Ok(json!({"name": n, "type": wire_type(t)?}))).collect::<Result<Vec<_>, String>>()?})
        }
        Type::Result(t, e) => json!({"kind": "result", "ok": wire_type(t)?, "err": wire_type(e)?}),
        other => return Err(format!("型 {other} のワイヤ形式は未定義です(ADR-0106)")),
    })
}

/// 全ターゲット共通の制約を検査する(ワイヤ形式が定義できない型など)。TGT001 を返す。
pub fn check_ir(ir: &AppIr) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let check = |what: String, t: &Type, d: &mut Vec<Diagnostic>| {
        if let Err(e) = wire_type(t) {
            d.push(Diagnostic::error(Code::TGT001, Span::default(), format!("{what}: {e}")));
        }
    };
    for e in &ir.entities {
        for (n, t) in &e.fields {
            if matches!(t, Type::Data(_) | Type::List(_) | Type::Array(_) | Type::Record(_) | Type::Result(..)) {
                d.push(Diagnostic::error(
                    Code::TGT001,
                    Span::default(),
                    format!("データ {} のフィールド {n}: 型 {t} の永続化は未対応です", e.name),
                ));
                continue;
            }
            check(format!("データ {} のフィールド {n}", e.name), t, &mut d);
        }
    }
    for a in &ir.actions {
        for (n, t) in &a.params {
            if matches!(t, Type::Data(_) | Type::List(_) | Type::Array(_) | Type::Record(_) | Type::Result(..)) {
                d.push(Diagnostic::error(
                    Code::TGT001,
                    a.span,
                    format!("動作 {} の引数 {n}: 型 {t} の API 入力は未対応です", a.name),
                ));
                continue;
            }
            check(format!("動作 {} の引数 {n}", a.name), t, &mut d);
        }
        if let Some(r) = &a.ret {
            check(format!("動作 {} の戻り値", a.name), r, &mut d);
        }
    }
    d
}

/// 共通 API 契約(api.json)。全ターゲットの生成物に同梱し、クライアント/サーバーの整合を検査できるようにする。
pub fn api_contract(ir: &AppIr) -> Value {
    let access = |a: &Access| match a {
        Access::Public => json!({"public": true, "capabilities": []}),
        Access::Caps(c) => json!({"public": false, "capabilities": c.iter().map(|x| x.vag()).collect::<Vec<_>>()}),
    };
    json!({
        "contract_version": "1",
        "application": ir.name,
        "entities": ir.entities.iter().map(|e| json!({
            "name": e.name,
            "fields": e.fields.iter().map(|(n, t)| json!({"name": n, "type": wire_type(t).unwrap_or(Value::Null)})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "enums": ir.enums.iter().map(|(n, v)| json!({"name": n, "variants": v})).collect::<Vec<_>>(),
        "endpoints": ir.endpoints.iter().map(|e| {
            let mut v = json!({"id": e.id, "method": e.method, "path": e.path_display(), "segments": e.segments, "has_id": e.has_id, "access": access(&e.access)});
            match &e.kind {
                EndpointKind::ListData { data } => { v["response"] = json!({"kind": "list", "of": {"kind": "entity", "name": data}}); }
                EndpointKind::GetData { data } => { v["response"] = json!({"kind": "entity", "name": data}); }
                EndpointKind::Query { knowledge, limits } => {
                    let spec = &ir.queries.iter().find(|q| &q.0.name == knowledge).unwrap().0;
                    v["query"] = json!({"limit": {"allowed": limits}, "base": {"kind": "datetime", "optional": true}});
                    v["response"] = wire_type(&spec.result_type()).unwrap_or(Value::Null);
                }
                EndpointKind::Action { action } => {
                    let a = ir.actions.iter().find(|x| &x.name == action).unwrap();
                    v["request"] = json!({"kind": "record", "fields": a.params.iter().map(|(n, t)| json!({"name": n, "type": wire_type(t).unwrap_or(Value::Null)})).collect::<Vec<_>>()});
                    v["response"] = match &a.ret { Some(t) => wire_type(t).unwrap_or(Value::Null), None => json!({"kind": "unit"}) };
                }
            }
            v
        }).collect::<Vec<_>>(),
        "screens": ir.screens.iter().map(|s| json!({"name": s.name, "input": s.input})).collect::<Vec<_>>(),
        "entry_screen": ir.entry_screen,
    })
}
