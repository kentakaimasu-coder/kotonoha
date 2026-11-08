//! 意図解釈候補の生成アダプタ(技術仕様書 §7、CLAUDE.md 原則 3「AI 解釈は resolver アダプタに隔離」)。
//!
//! - アダプタの出力は**未信頼データ**(SEC-08)。[`validate_candidate`] で厳格なスキーマ検査を行い、
//!   さらに driver が各解釈を既知の文型で再照合してから承認対象にする。出力をコードとして評価することはない。
//! - 外部の AI へソースを送る方針は ADR-006 で未決定のため、外部送信するアダプタは持たない。
//!   `offline`(質問だけを返す)、`fixture`(人が書いた/テスト用の候補 JSON を読む)に加え、
//!   ADR-0127 の `local`(この機械の中で動くオープンモデル)と `memory`(過去の承認だけ)を提供する。

pub mod learn;
pub mod local;

use kotonoha_intent::Unresolved;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const CANDIDATES_FILE: &str = "kotonoha.candidates.json";
pub const KNOWN_EFFECTS: &[&str] = &[
    "data.read",
    "data.write",
    "network.send",
    "filesystem.read",
    "filesystem.write",
    "device.camera",
    "device.location",
    "device.notifications",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interpretation {
    /// 既知の文型に一致する正規文(例「売れ筋商品を表示」)
    pub canonical: String,
    pub rationale: String,
}

/// 検証済みの候補(intent-candidate.schema.json)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub intent_id: String,
    pub sentence_hash: String,
    pub interpretations: Vec<Interpretation>,
    pub assumptions: Vec<String>,
    pub unresolved: Vec<String>,
    pub required_effects: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatesFile {
    pub candidates_version: String,
    pub adapter: String,
    pub candidates: Vec<Candidate>,
}

/// 候補生成アダプタ。戻り値は未検証の JSON。
pub trait ResolverAdapter {
    fn name(&self) -> &'static str;
    /// ネットワーク送信を伴うか(NFR-010: 伴う場合は明示同意が必要)
    fn sends_source(&self) -> bool;
    fn propose(&self, u: &Unresolved) -> Result<Value, String>;
}

/// AI を使わない既定アダプタ。解釈は出さず、質問を unresolved として返す。
pub struct OfflineAdapter;

impl ResolverAdapter for OfflineAdapter {
    fn name(&self) -> &'static str {
        "offline"
    }
    fn sends_source(&self) -> bool {
        false
    }
    fn propose(&self, u: &Unresolved) -> Result<Value, String> {
        Ok(serde_json::json!({
            "intent_id": u.intent_id,
            "sentence_hash": u.sentence_hash,
            "interpretations": [],
            "assumptions": [],
            "unresolved": u.questions,
            "required_effects": [],
            "evidence_refs": [],
        }))
    }
}

/// 人が書いた(またはテスト用の)候補 JSON を読むアダプタ。キーは intent_id または原文。
pub struct FixtureAdapter {
    pub entries: BTreeMap<String, Value>,
}

impl FixtureAdapter {
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("候補フィクスチャを読めません: {e}"))?;
        let obj = v.as_object().ok_or("候補フィクスチャは { キー: 候補 } のオブジェクトです")?;
        Ok(FixtureAdapter {
            entries: obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        })
    }
}

impl ResolverAdapter for FixtureAdapter {
    fn name(&self) -> &'static str {
        "fixture"
    }
    fn sends_source(&self) -> bool {
        false
    }
    fn propose(&self, u: &Unresolved) -> Result<Value, String> {
        let mut v = self
            .entries
            .get(&u.intent_id)
            .or_else(|| self.entries.get(&u.sentence))
            .cloned()
            .ok_or_else(|| format!("`{}` の候補がフィクスチャにありません", u.sentence))?;
        // intent_id / sentence_hash はフィクスチャ側で省略できる(現在の値を補う)
        if let Some(o) = v.as_object_mut() {
            o.entry("intent_id").or_insert(Value::String(u.intent_id.clone()));
            o.entry("sentence_hash").or_insert(Value::String(u.sentence_hash.clone()));
        }
        Ok(v)
    }
}

/// 未信頼の候補 JSON の厳格なスキーマ検査。余分なキー・型違い・ID 不一致はすべて拒否する。
pub fn validate_candidate(raw: &Value, u: &Unresolved) -> Result<Candidate, Vec<String>> {
    let mut errs = Vec::new();
    let Some(o) = raw.as_object() else {
        return Err(vec!["候補はオブジェクトでなければなりません".into()]);
    };
    const KEYS: &[&str] = &[
        "intent_id",
        "sentence_hash",
        "interpretations",
        "assumptions",
        "unresolved",
        "required_effects",
        "evidence_refs",
    ];
    for k in o.keys() {
        if !KEYS.contains(&k.as_str()) {
            errs.push(format!("未知のキー `{k}`"));
        }
    }
    for k in KEYS {
        if !o.contains_key(*k) {
            errs.push(format!("必須キー `{k}` がありません"));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let s = |k: &str, errs: &mut Vec<String>| -> String {
        o[k].as_str().map(str::to_string).unwrap_or_else(|| {
            errs.push(format!("`{k}` は文字列です"));
            String::new()
        })
    };
    let strs = |k: &str, errs: &mut Vec<String>| -> Vec<String> {
        match o[k].as_array() {
            Some(a) if a.iter().all(|x| x.is_string()) => a.iter().map(|x| x.as_str().unwrap().to_string()).collect(),
            _ => {
                errs.push(format!("`{k}` は文字列の配列です"));
                Vec::new()
            }
        }
    };
    let intent_id = s("intent_id", &mut errs);
    let sentence_hash = s("sentence_hash", &mut errs);
    if intent_id != u.intent_id {
        errs.push(format!("intent_id `{intent_id}` が対象 `{}` と一致しません", u.intent_id));
    }
    if sentence_hash != u.sentence_hash {
        errs.push("sentence_hash が現在の文と一致しません".into());
    }
    let mut interpretations = Vec::new();
    match o["interpretations"].as_array() {
        None => errs.push("`interpretations` は配列です".into()),
        Some(a) => {
            for (i, it) in a.iter().enumerate() {
                let ok = it
                    .as_object()
                    .is_some_and(|m| m.len() == 2 && m.get("canonical").is_some_and(|x| x.is_string()) && m.get("rationale").is_some_and(|x| x.is_string()));
                if !ok {
                    errs.push(format!("interpretations[{i}] は {{canonical: 文字列, rationale: 文字列}} です"));
                    continue;
                }
                let c = it["canonical"].as_str().unwrap();
                if c.is_empty() || c.contains('\n') || c.chars().count() > 200 {
                    errs.push(format!("interpretations[{i}].canonical は 1 行 200 文字以内です"));
                    continue;
                }
                interpretations.push(Interpretation {
                    canonical: c.into(),
                    rationale: it["rationale"].as_str().unwrap().into(),
                });
            }
        }
    }
    let assumptions = strs("assumptions", &mut errs);
    let unresolved = strs("unresolved", &mut errs);
    let required_effects = strs("required_effects", &mut errs);
    let evidence_refs = strs("evidence_refs", &mut errs);
    for e in &required_effects {
        if !KNOWN_EFFECTS.contains(&e.as_str()) {
            errs.push(format!("未知の効果 `{e}`"));
        }
    }
    if errs.is_empty() {
        Ok(Candidate {
            intent_id,
            sentence_hash,
            interpretations,
            assumptions,
            unresolved,
            required_effects,
            evidence_refs,
        })
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotonoha_diag::Span;
    use kotonoha_intent::Mode;

    fn u() -> Unresolved {
        Unresolved {
            intent_id: "screen.人気#000000000000".into(),
            owner: "screen.人気".into(),
            sentence: "人気商品を表示".into(),
            sentence_hash: "abc".into(),
            mode: Mode::FlexConfirm,
            questions: vec!["「人気商品」は何を指しますか".into()],
            span: Span::default(),
        }
    }

    #[test]
    fn offline_adapter_returns_questions_only() {
        let v = OfflineAdapter.propose(&u()).unwrap();
        let c = validate_candidate(&v, &u()).unwrap();
        assert!(c.interpretations.is_empty());
        assert_eq!(c.unresolved.len(), 1);
        assert!(!OfflineAdapter.sends_source());
    }

    #[test]
    fn fixture_adapter_fills_ids() {
        let fx = FixtureAdapter::from_json(
            r#"{"人気商品を表示": {"interpretations":[{"canonical":"売れ筋商品を表示","rationale":"人気=売れ筋と解釈"}],"assumptions":["人気は売上金額基準"],"unresolved":[],"required_effects":["data.read"],"evidence_refs":["知識 売れ筋商品"]}}"#,
        )
        .unwrap();
        let c = validate_candidate(&fx.propose(&u()).unwrap(), &u()).unwrap();
        assert_eq!(c.interpretations[0].canonical, "売れ筋商品を表示");
    }

    #[test]
    fn rejects_malformed_or_injected_candidates() {
        let base = OfflineAdapter.propose(&u()).unwrap();
        let mut extra = base.clone();
        extra["code"] = Value::String("rm -rf /".into());
        assert!(validate_candidate(&extra, &u()).is_err());
        let mut wrong_id = base.clone();
        wrong_id["intent_id"] = Value::String("screen.他#1".into());
        assert!(validate_candidate(&wrong_id, &u()).is_err());
        let mut bad_eff = base.clone();
        bad_eff["required_effects"] = serde_json::json!(["shell.exec"]);
        assert!(validate_candidate(&bad_eff, &u()).is_err());
        let mut bad_interp = base.clone();
        bad_interp["interpretations"] = serde_json::json!([{"canonical": "a\nb", "rationale": ""}]);
        assert!(validate_candidate(&bad_interp, &u()).is_err());
        let mut missing = base;
        missing.as_object_mut().unwrap().remove("assumptions");
        assert!(validate_candidate(&missing, &u()).is_err());
        assert!(validate_candidate(&Value::String("x".into()), &u()).is_err());
    }
}

#[cfg(test)]
mod local_tests;
