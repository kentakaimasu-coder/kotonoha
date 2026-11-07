//! 効果・Capability 検査(FR-008、技術仕様書 §9、SEC-01 デフォルト拒否)。
//!
//! - 動作の必要 Capability = 呼んだアダプタの Capability ∪ エンティティ更新(資源.update)∪ 呼んだ動作の必要 Capability(推移閉包)
//! - `効果 = [...]` を宣言した動作が宣言外の効果を起こすと SEC001
//! - `必要 = [...]` に無い Capability を使うと SEC002。`必要` 未宣言で資源に触れる動作も SEC002(デフォルト拒否)
//! - 画面は表示に必要な読取 Capability を集約する(認可はサーバー側で別途強制する。SEC-03)

use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_intent::{IntentKind, IntentModel};
use kotonoha_types::{Capability, Effect, Env, Facts, Verb};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EffectSummary {
    pub effects: BTreeSet<Effect>,
    pub capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AccessPolicy {
    /// 未認証でも読取できる資源(約束 C-PUBLIC)
    pub public_read: BTreeSet<String>,
    /// 認証必須と明示された資源(約束 C-AUTH。デフォルト拒否のため既定でも認証必須)
    pub auth_required: BTreeSet<String>,
    /// 行レベル認可: データ → 本人の主体 ID を入れるフィールド(約束 C-OWNER)
    pub owner_read: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Validation {
    pub actions: BTreeMap<String, EffectSummary>,
    pub screens: BTreeMap<String, EffectSummary>,
    pub policy: AccessPolicy,
}

/// 直接の Capability と、その発生位置。
fn direct_caps(env: &Env, facts: &Facts) -> Vec<(Capability, Span)> {
    let mut out = Vec::new();
    for (a, sp) in &facts.adapter_calls {
        if let Some(sig) = env.adapters.get(a) {
            out.push((sig.capability(), *sp));
        }
    }
    for (d, _, sp) in &facts.mutations {
        out.push((Capability::new(d.clone(), Verb::Update), *sp));
    }
    out
}

pub fn validate(env: &Env, facts: &BTreeMap<String, Facts>, intents: &IntentModel) -> (Validation, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let mut v = Validation::default();

    // 推移閉包(循環呼出も不動点で扱う)
    let mut caps: BTreeMap<String, BTreeMap<Capability, Span>> = BTreeMap::new();
    for (name, f) in facts {
        let m = caps.entry(name.clone()).or_default();
        for (c, sp) in direct_caps(env, f) {
            m.entry(c).or_insert(sp);
        }
    }
    loop {
        let mut changed = false;
        for (name, f) in facts {
            for (callee, sp) in &f.action_calls {
                let add: Vec<Capability> = caps.get(callee).map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let me = caps.entry(name.clone()).or_default();
                for c in add {
                    if let std::collections::btree_map::Entry::Vacant(e) = me.entry(c) {
                        e.insert(*sp);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    for (name, sig) in &env.actions {
        let computed = caps.get(name).cloned().unwrap_or_default();
        let mut effects: BTreeSet<Effect> = computed.keys().map(|c| c.verb.effect()).collect();
        // 外部関数の宣言効果(推移: 呼んだ動作の外部関数も含む)
        let mut ext_spans: BTreeMap<Effect, Span> = BTreeMap::new();
        let mut stack = vec![name.clone()];
        let mut seen = BTreeSet::new();
        while let Some(n) = stack.pop() {
            if !seen.insert(n.clone()) {
                continue;
            }
            if let Some(f) = facts.get(&n) {
                for (x, sp) in &f.external_calls {
                    for e in env.externals.get(x).map(|s| s.effects.clone()).unwrap_or_default() {
                        effects.insert(e);
                        ext_spans.entry(e).or_insert(*sp);
                    }
                }
                stack.extend(f.action_calls.iter().map(|(a, _)| a.clone()));
            }
        }
        if let Some(decl_eff) = &sig.declared_effects {
            for e in &effects {
                if !decl_eff.contains(e) {
                    let sp = computed
                        .iter()
                        .find(|(c, _)| c.verb.effect() == *e)
                        .map(|(_, s)| *s)
                        .or(ext_spans.get(e).copied())
                        .unwrap_or(sig.span);
                    diags.push(
                        Diagnostic::error(
                            Code::SEC001,
                            sp,
                            format!(
                                "動作 `{name}` は効果 {e} を起こしますが、宣言された効果は [{}] です",
                                join(decl_eff.iter().map(|x| x.as_str().to_string()))
                            ),
                        )
                        .hint(format!("`効果 = [...]` に {e} を加えるか、処理を見直してください")),
                    );
                }
            }
        }
        match &sig.declared_caps {
            None => {
                if !computed.is_empty() {
                    diags.push(
                        Diagnostic::error(
                            Code::SEC002,
                            sig.span,
                            format!("動作 `{name}` は資源に触れますが `必要 = [...]` が宣言されていません"),
                        )
                        .hint(format!("必要 = [{}]", join(computed.keys().map(|c| c.source())))),
                    );
                }
            }
            Some(decl) => {
                for (c, sp) in &computed {
                    if !decl.contains(c) {
                        diags.push(
                            Diagnostic::error(
                                Code::SEC002,
                                *sp,
                                format!("動作 `{name}` は Capability {} を必要としますが宣言されていません", c.source()),
                            )
                            .hint(format!("`必要` に {} を加えてください(更新は読取を含みません)", c.source())),
                        );
                    }
                }
                for c in decl {
                    if !computed.contains_key(c) {
                        diags.push(
                            Diagnostic::warning(Code::SEC002, sig.span, format!("動作 `{name}` の Capability {} は使われていません", c.source()))
                                .hint("最小権限のため削除を検討してください"),
                        );
                    }
                }
            }
        }
        v.actions.insert(
            name.clone(),
            EffectSummary {
                effects,
                capabilities: computed.keys().cloned().collect(),
            },
        );
    }

    // 画面
    for s in &intents.screens {
        let mut sum = EffectSummary::default();
        for i in &s.intents {
            let read = match &i.kind {
                IntentKind::ListData { data } | IntentKind::ShowFields { data, .. } => Some(data.clone()),
                IntentKind::ShowKnowledge { spec, .. } => Some(spec.source.clone()),
                IntentKind::Analysis { analysis } => {
                    for t in env.externals.get(analysis).map(|x| x.tables()).unwrap_or_default() {
                        sum.capabilities.insert(Capability::new(t, Verb::Read));
                        sum.effects.insert(Effect::DataRead);
                    }
                    None
                }
                _ => None,
            };
            if let Some(d) = read {
                sum.capabilities.insert(Capability::new(d, Verb::Read));
                sum.effects.insert(Effect::DataRead);
            }
        }
        v.screens.insert(s.name.clone(), sum);
    }

    // 約束
    let mut public_span: BTreeMap<String, Span> = BTreeMap::new();
    let mut owner_span: BTreeMap<String, Span> = BTreeMap::new();
    for c in &intents.contracts {
        match &c.kind {
            IntentKind::PublicRead { data } => {
                v.policy.public_read.insert(data.clone());
                public_span.insert(data.clone(), c.span);
            }
            IntentKind::AuthRequired { data } => {
                v.policy.auth_required.insert(data.clone());
            }
            IntentKind::OwnerRead { data, field } => {
                if let Some(prev) = v.policy.owner_read.insert(data.clone(), field.clone()) {
                    if &prev != field {
                        diags.push(Diagnostic::error(
                            Code::AMB002,
                            c.span,
                            format!("{data} の本人判定フィールドが複数あります({prev} と {field})"),
                        ));
                    }
                }
                if v.policy.public_read.contains(data) {
                    diags.push(Diagnostic::error(
                        Code::AMB002,
                        c.span,
                        format!("約束が矛盾しています: {data} は「誰でも閲覧可」かつ「本人のものだけ閲覧可」です"),
                    ));
                }
                owner_span.insert(data.clone(), c.span);
            }
            _ => {}
        }
    }
    for d in v.policy.public_read.intersection(&v.policy.auth_required) {
        diags.push(
            Diagnostic::error(
                Code::AMB002,
                public_span[d],
                format!("約束が矛盾しています: {d} は「誰でも閲覧可」かつ「未認証なら不可」です"),
            )
            .hint("どちらの約束を優先するか決めてください"),
        );
    }
    for d in v.policy.public_read.iter() {
        if let Some(sp) = owner_span.get(d) {
            if !diags.iter().any(|x| x.span == *sp && x.code == Code::AMB002) {
                diags.push(Diagnostic::error(
                    Code::AMB002,
                    *sp,
                    format!("約束が矛盾しています: {d} は「誰でも閲覧可」かつ「本人のものだけ閲覧可」です"),
                ));
            }
        }
    }
    (v, diags)
}

fn join(it: impl Iterator<Item = String>) -> String {
    it.collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotonoha_ast::Decl;
    use kotonoha_diag::FileId;

    const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");

    fn run(src: &str) -> (Validation, Vec<Diagnostic>) {
        let (f, pd) = kotonoha_parser::parse_file(src, FileId(0));
        assert!(pd.is_empty(), "{pd:#?}");
        let (env, ed) = kotonoha_types::collect_env(&[&f]);
        assert!(ed.iter().all(|d| !d.is_error()), "{ed:#?}");
        let mut facts = BTreeMap::new();
        for it in &f.items {
            if let Decl::Action(a) = &it.decl {
                let (fa, d) = kotonoha_types::check_action(&env, a);
                assert!(d.iter().all(|x| !x.is_error()), "{d:#?}");
                facts.insert(a.name.name.clone(), fa);
            }
        }
        let (reg, _) = kotonoha_knowledge::build_registry(&[&f], &env);
        let (im, idg) = kotonoha_intent::resolve_intents(&[&f], &env, &reg, &[]);
        assert!(idg.is_empty(), "{idg:#?}");
        validate(&env, &facts, &im)
    }

    fn errs(d: &[Diagnostic]) -> Vec<Code> {
        d.iter().filter(|x| x.is_error()).map(|x| x.code).collect()
    }

    #[test]
    fn shop_passes_and_summaries_are_computed() {
        let (v, d) = run(SHOP);
        assert!(d.is_empty(), "{d:#?}");
        let a = &v.actions["注文する"];
        assert_eq!(
            a.capabilities.iter().map(|c| c.vag()).collect::<Vec<_>>(),
            ["商品.read", "商品.update", "注文.create"]
        );
        assert_eq!(a.effects.iter().map(|e| e.as_str()).collect::<Vec<_>>(), ["data.read", "data.write"]);
        assert_eq!(v.screens["商品一覧"].capabilities.iter().map(|c| c.vag()).collect::<Vec<_>>(), ["商品.read"]);
        assert_eq!(v.screens["売れ筋"].capabilities.iter().map(|c| c.vag()).collect::<Vec<_>>(), ["注文明細.read"]);
        assert!(v.policy.public_read.contains("商品"));
        assert!(v.policy.auth_required.contains("注文"));
    }

    #[test]
    fn tc006_read_only_effect_with_update_is_sec001() {
        let src = SHOP.replace(
            "    必要 = [注文.作成, 商品.読取, 商品.更新]\n",
            "    必要 = [注文.作成, 商品.読取, 商品.更新]\n    効果 = [data.read]\n",
        );
        let (_, d) = run(&src);
        assert_eq!(errs(&d), [Code::SEC001]);
        let want = src.lines().position(|l| l.trim() == "商品.在庫 = 商品.在庫 - 数量").unwrap() as u32 + 1;
        assert_eq!(d.iter().find(|x| x.is_error()).unwrap().span.line, want, "最初の書込み(在庫の更新)の位置");
    }

    #[test]
    fn missing_capability_is_sec002() {
        // 仕様書 §2 の宣言(商品.読取 なし)
        let src = SHOP.replace("[注文.作成, 商品.読取, 商品.更新]", "[注文.作成, 商品.更新]");
        assert_eq!(errs(&run(&src).1), [Code::SEC002]);
        let src = SHOP.replace("    必要 = [注文.作成, 商品.読取, 商品.更新]\n", "");
        assert_eq!(errs(&run(&src).1), [Code::SEC002], "必要 未宣言はデフォルト拒否");
    }

    #[test]
    fn unused_capability_is_warning_only() {
        let src = SHOP.replace("[注文.作成, 商品.読取, 商品.更新]", "[注文.作成, 商品.読取, 商品.更新, 商品.削除]");
        let (_, d) = run(&src);
        assert!(errs(&d).is_empty());
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn transitive_capabilities_through_action_calls() {
        let src = format!(
            "{SHOP}\n動作 まとめ買い(商品ID: 識別子) -> 結果<注文ID, 注文失敗>:\n    必要 = [注文.作成]\n    厳密:\n        注文する(商品ID, 2) を返す\n"
        );
        let (_, d) = run(&src);
        assert_eq!(errs(&d), [Code::SEC002, Code::SEC002]);
    }

    #[test]
    fn owner_and_public_read_conflict_is_amb002() {
        let src = SHOP.replace("    未認証なら注文不可\n", "    未認証なら注文不可\n    注文は誰でも閲覧可\n");
        assert!(errs(&run(&src).1).contains(&Code::AMB002));
        let (v, d) = run(SHOP);
        assert!(d.is_empty());
        assert_eq!(v.policy.owner_read.get("注文").map(String::as_str), Some("注文者"));
    }

    #[test]
    fn contradictory_contracts_are_amb002() {
        let src = SHOP.replace("    未認証なら注文不可\n", "    未認証なら商品不可\n");
        assert_eq!(errs(&run(&src).1), [Code::AMB002]);
    }
}
