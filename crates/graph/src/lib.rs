//! Verified Application Graph (VAG) v1(技術仕様書 §6、スキーマは schemas/vag.schema.json)。
//!
//! - ノード ID は宣言の安定 ID(`screen.<名前>`, `action.<名前>`, 意図は `<所有者>#<文ハッシュ12桁>`)
//! - 正規化 JSON(キーは UTF-8 バイト順・空白なし・数値は整数のみ)で SHA-256 を計算する
//! - `status = "validated"` は全検査(承認・型・効果・権限・参照整合性)を通過したときだけ付与する

use kotonoha_diag::{FileId, Span};
use kotonoha_intent::{IntentKind, IntentModel, Resolution};
use kotonoha_knowledge::{Knowledge, Registry};
use kotonoha_types::{Env, Type};
use kotonoha_validator::Validation;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Validated,
    /// 検査エラーがある状態での参考出力(ロック・ビルドには使えない)
    Draft,
}

pub struct GraphInput<'a> {
    pub env: &'a Env,
    pub registry: &'a Registry,
    pub intents: &'a IntentModel,
    pub validation: &'a Validation,
    pub files: &'a BTreeMap<FileId, String>,
}

fn span_json(files: &BTreeMap<FileId, String>, s: Span) -> Value {
    json!({"file": files.get(&s.file).cloned().unwrap_or_default(), "line": s.line, "column": s.column})
}

fn tstr(t: &Type) -> String {
    t.to_string()
}

#[allow(clippy::type_complexity)]
pub fn build_vag(inp: &GraphInput, status: Status) -> Value {
    let env = inp.env;
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();

    // 画面と意図ノード
    for s in &inp.intents.screens {
        let sid = format!("screen.{}", s.name);
        let sum = inp.validation.screens.get(&s.name).cloned().unwrap_or_default();
        let mut outputs = Vec::new();
        let mut krefs = BTreeSet::new();
        for i in &s.intents {
            edges.insert((sid.clone(), i.id.clone(), "contains".into()));
            let resolution = match &i.resolution {
                Resolution::Pattern => json!({"via": "pattern"}),
                Resolution::Approved { canonical, approval_hash } => json!({"via": "approved", "canonical": canonical, "approval_hash": approval_hash}),
            };
            let (kind, title, inputs, outs, effects, caps, kref, detail): (&str, String, Vec<String>, Vec<String>, Vec<&str>, Vec<String>, Vec<String>, Value) =
                match &i.kind {
                    IntentKind::ListData { data, order, filters } => {
                        let o = format!("一覧<{data}>");
                        outputs.push(o.clone());
                        let mut detail = json!({"ui": "list", "data": data});
                        if let Some(o) = order {
                            detail["order"] = json!({"field": o.field, "desc": o.desc});
                        }
                        if !filters.is_empty() {
                            detail["filters"] = serde_json::to_value(filters).unwrap();
                        }
                        (
                            "display",
                            kotonoha_intent::list_caption(data, order, filters),
                            vec![],
                            vec![o],
                            vec!["data.read"],
                            vec![format!("{data}.read")],
                            vec![],
                            detail,
                        )
                    }
                    IntentKind::ShowFields { data, fields } => (
                        "display",
                        format!("{data}の{}", fields.join("と")),
                        vec![data.clone()],
                        vec![],
                        vec!["data.read"],
                        vec![format!("{data}.read")],
                        vec![],
                        json!({"ui": "detail", "data": data, "fields": fields}),
                    ),
                    IntentKind::ShowKnowledge {
                        knowledge,
                        limit_override,
                        spec,
                    } => {
                        krefs.insert(knowledge.clone());
                        let o = tstr(&spec.result_type());
                        outputs.push(o.clone());
                        (
                            "query",
                            knowledge.clone(),
                            vec![],
                            vec![o],
                            vec!["data.read"],
                            vec![format!("{}.read", spec.source)],
                            vec![knowledge.clone()],
                            json!({"ui": "list", "knowledge": knowledge, "limit": spec.limit, "limit_override": limit_override}),
                        )
                    }
                    IntentKind::NavigateOnSelect { data, screen } => {
                        edges.insert((i.id.clone(), format!("screen.{screen}"), "navigates".into()));
                        (
                            "navigation",
                            format!("{data}選択→{screen}"),
                            vec![data.clone()],
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            json!({"event": "tap", "data": data, "to": format!("screen.{screen}")}),
                        )
                    }
                    IntentKind::Component { component, args } => {
                        let args: Vec<String> = args.iter().map(kotonoha_parser::printer::expr_str).collect();
                        (
                            "display",
                            component.clone(),
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            json!({"ui": "component", "component": component, "args": args}),
                        )
                    }
                    IntentKind::Analysis { analysis } => {
                        let sig = &env.externals[analysis];
                        let o = sig.ret.as_ref().map(tstr).unwrap_or_default();
                        outputs.push(o.clone());
                        let caps: Vec<String> = sig.tables().iter().map(|t| format!("{t}.read")).collect();
                        (
                            "query",
                            analysis.clone(),
                            vec![],
                            vec![o],
                            vec!["data.read"],
                            caps,
                            vec![],
                            json!({"ui": "table", "analysis": analysis}),
                        )
                    }
                    IntentKind::Form { action } => {
                        edges.insert((i.id.clone(), format!("action.{action}"), "invokes".into()));
                        edges.insert((sid.clone(), format!("action.{action}"), "invokes".into()));
                        (
                            "display",
                            format!("{action}のフォーム"),
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            json!({"ui": "form", "event": "submit", "action": format!("action.{action}")}),
                        )
                    }
                    IntentKind::ButtonInvoke { label, action, args } => {
                        edges.insert((i.id.clone(), format!("action.{action}"), "invokes".into()));
                        edges.insert((sid.clone(), format!("action.{action}"), "invokes".into()));
                        let args: Vec<String> = args.iter().map(kotonoha_parser::printer::expr_str).collect();
                        (
                            "display",
                            label.clone(),
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            vec![],
                            json!({"ui": "button", "event": "tap", "label": label, "action": format!("action.{action}"), "args": args}),
                        )
                    }
                    IntentKind::PublicRead { .. } | IntentKind::AuthRequired { .. } | IntentKind::OwnerRead { .. } => unreachable!("約束は画面に現れない"),
                };
            nodes.push(json!({
                "id": i.id, "kind": kind, "title": title, "sentence": i.sentence,
                "inputs": inputs, "outputs": outs, "effects": effects, "capabilities": caps,
                "knowledge_refs": kref, "source_span": span_json(inp.files, i.span),
                "resolution": resolution, "detail": detail,
            }));
        }
        nodes.push(json!({
            "id": sid, "kind": "screen", "title": s.name, "mode": mode_str(s.mode),
            "inputs": s.input.iter().cloned().collect::<Vec<_>>(), "outputs": outputs,
            "effects": sum.effects.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
            "capabilities": sum.capabilities.iter().map(|c| c.vag()).collect::<Vec<_>>(),
            "knowledge_refs": krefs.into_iter().collect::<Vec<_>>(), "source_span": span_json(inp.files, s.span),
        }));
    }

    // 動作ノード
    for (name, sig) in &env.actions {
        let sum = inp.validation.actions.get(name).cloned().unwrap_or_default();
        nodes.push(json!({
            "id": format!("action.{name}"), "kind": "action", "title": name,
            "inputs": sig.params.iter().map(|(_, t)| tstr(t)).collect::<Vec<_>>(),
            "params": sig.params.iter().map(|(n, t)| json!({"name": n, "type": tstr(t)})).collect::<Vec<_>>(),
            "outputs": sig.ret.iter().map(tstr).collect::<Vec<_>>(),
            "effects": sum.effects.iter().map(|e| e.as_str()).collect::<Vec<_>>(),
            "capabilities": sum.capabilities.iter().map(|c| c.vag()).collect::<Vec<_>>(),
            "knowledge_refs": Vec::<String>::new(),
            "auth": "required",
            "source_span": span_json(inp.files, sig.span),
        }));
    }
    // 約束ノード(検証)
    for c in &inp.intents.contracts {
        let (policy, data, field) = match &c.kind {
            IntentKind::PublicRead { data } => ("public_read", data, None),
            IntentKind::AuthRequired { data } => ("auth_required", data, None),
            IntentKind::OwnerRead { data, field } => ("owner_read", data, Some(field.clone())),
            _ => continue,
        };
        nodes.push(json!({
            "id": c.id, "kind": "validation", "title": c.sentence, "sentence": c.sentence,
            "inputs": [], "outputs": [], "effects": [], "capabilities": [], "knowledge_refs": [],
            "source_span": span_json(inp.files, c.span), "detail": match field { Some(f) => json!({"policy": policy, "data": data, "field": f}), None => json!({"policy": policy, "data": data}) },
        }));
    }

    nodes.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    let edges: Vec<Value> = edges.into_iter().map(|(f, t, k)| json!({"from": f, "to": t, "kind": k})).collect();

    let schema = json!({
        "enums": env.enums.iter().map(|(n, vs)| json!({"name": n, "variants": vs})).collect::<Vec<_>>(),
        "aliases": env.aliases.iter().map(|(n, t)| json!({"name": n, "type": tstr(t)})).collect::<Vec<_>>(),
        "data": env.data.values().map(|d| json!({"name": d.name, "fields": d.fields.iter().map(|(n, t)| json!({"name": n, "type": tstr(t)})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "adapters": env.adapters.values().map(|a| json!({
            "name": a.name, "op": a.op.word(), "target": a.target,
            "params": a.params.iter().map(|(n, t)| json!({"name": n, "type": tstr(t)})).collect::<Vec<_>>(),
            "returns": tstr(&a.ret), "capability": a.capability().vag(), "effect": a.effect().as_str(),
        })).collect::<Vec<_>>(),
    });

    json!({
        "schema_version": SCHEMA_VERSION,
        "application": {"name": env.app_name.clone().unwrap_or_default()},
        "schema": schema,
        "knowledge": knowledge_json(inp.registry),
        "policy": inp.validation.policy,
        "nodes": nodes,
        "edges": edges,
        "status": match status { Status::Validated => "validated", Status::Draft => "draft" },
    })
}

fn mode_str(m: kotonoha_intent::Mode) -> &'static str {
    match m {
        kotonoha_intent::Mode::Strict => "厳密",
        kotonoha_intent::Mode::FlexConfirm => "柔軟(確認)",
        kotonoha_intent::Mode::FlexAuto => "柔軟(自動)",
    }
}

/// 知識レジストリの正規 JSON(knowledge_hash の対象)。
pub fn knowledge_json(reg: &Registry) -> Value {
    Value::Array(
        reg.entries
            .values()
            .map(|k| match k {
                Knowledge::Ranking(_) => serde_json::to_value(k).expect("知識は直列化できる"),
            })
            .collect(),
    )
}

/// 正規化 JSON(キーは UTF-8 バイト順、空白なし)。
/// serde_json の既定 Map は BTreeMap なので to_string はキー順に出力される。
pub fn canonical_json(v: &Value) -> String {
    debug_assert!(no_floats(v), "正規化 JSON は整数のみを許す");
    serde_json::to_string(v).expect("Value は常に直列化できる")
}

fn no_floats(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.is_i64() || n.is_u64(),
        Value::Array(a) => a.iter().all(no_floats),
        Value::Object(m) => m.values().all(no_floats),
        _ => true,
    }
}

pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_value(v: &Value) -> String {
    sha256_hex(&canonical_json(v))
}

/// 参照整合性(重複 ID・辺の端点の存在)。問題の説明を返す。
pub fn check_integrity(vag: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let mut ids = BTreeSet::new();
    for n in vag["nodes"].as_array().into_iter().flatten() {
        let id = n["id"].as_str().unwrap_or("");
        if !ids.insert(id.to_string()) {
            problems.push(format!("ノード ID `{id}` が重複しています"));
        }
    }
    for e in vag["edges"].as_array().into_iter().flatten() {
        for end in ["from", "to"] {
            let id = e[end].as_str().unwrap_or("");
            if !ids.contains(id) {
                problems.push(format!("辺の端点 `{id}` が存在しません"));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_keys_and_keeps_unicode() {
        let v = json!({"b": 1, "a": {"y": "商品", "x": [2, 1]}});
        assert_eq!(canonical_json(&v), r#"{"a":{"x":[2,1],"y":"商品"},"b":1}"#);
    }

    #[test]
    fn integrity_detects_dangling_edges_and_duplicates() {
        let v = json!({"nodes": [{"id": "a"}, {"id": "a"}], "edges": [{"from": "a", "to": "b"}]});
        let p = check_integrity(&v);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
