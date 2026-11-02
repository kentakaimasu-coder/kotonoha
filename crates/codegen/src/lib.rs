//! ターゲット別生成の契約(技術仕様書 §11)とクエリの低水準化。
//!
//! 各ターゲットは `supports(node_kind, feature)` を持ち、VAG に未対応の機能が 1 つでもあれば
//! 黙って欠落させずに TGT001 で停止する。
//!
//! 現状の実装範囲(正直な申告):
//! - backend: 知識(集計ランキング)の SQL 低水準化のみ実装(`query.ranking`)。HTTP/認可/CRUD 生成は未実装
//! - web: 未実装(UI 生成フレームワークは ADR-004 で未決定)
//! - ios / android: 契約(本モジュールの trait と support 表)のみ。生成は P1

pub mod sql;

use kotonoha_diag::{Code, Diagnostic, Span};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetKind {
    Web,
    Backend,
    Ios,
    Android,
}

impl TargetKind {
    pub fn parse(s: &str) -> Option<TargetKind> {
        Some(match s {
            "web" => TargetKind::Web,
            "backend" => TargetKind::Backend,
            "ios" => TargetKind::Ios,
            "android" => TargetKind::Android,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Web => "web",
            TargetKind::Backend => "backend",
            TargetKind::Ios => "ios",
            TargetKind::Android => "android",
        }
    }
}

/// VAG ノードから (ノード種別, 機能名) を取り出す。機能名の一覧は docs/spec/targets.md。
pub fn node_feature(node: &Value) -> (String, String) {
    let kind = node["kind"].as_str().unwrap_or("").to_string();
    let feature = match kind.as_str() {
        "screen" => "screen.page".to_string(),
        "display" => format!("ui.{}", node["detail"]["ui"].as_str().unwrap_or("unknown")),
        "query" => "query.ranking".to_string(),
        "navigation" => "nav.select".to_string(),
        "action" => "action.strict".to_string(),
        "validation" => format!("policy.{}", node["detail"]["policy"].as_str().unwrap_or("unknown")),
        _ => "unknown".to_string(),
    };
    (kind, feature)
}

/// ターゲット生成器の契約。
pub trait Target {
    fn kind(&self) -> TargetKind;
    fn supports(&self, node_kind: &str, feature: &str) -> bool;
}

/// 現時点の対応表。実装が増えたらここを更新し、対応するテストを追加すること。
pub struct SupportTable {
    pub target: TargetKind,
    pub supported: &'static [(&'static str, &'static str)],
}

impl Target for SupportTable {
    fn kind(&self) -> TargetKind {
        self.target
    }
    fn supports(&self, node_kind: &str, feature: &str) -> bool {
        self.supported.iter().any(|(k, f)| *k == node_kind && *f == feature)
    }
}

pub fn target_for(kind: TargetKind) -> SupportTable {
    let supported: &'static [(&'static str, &'static str)] = match kind {
        TargetKind::Backend => &[("query", "query.ranking")],
        TargetKind::Web | TargetKind::Ios | TargetKind::Android => &[],
    };
    SupportTable { target: kind, supported }
}

/// VAG 全体の対応可否。未対応の (種別, 機能) ごとに TGT001 を 1 件返す。
pub fn check_support(t: &dyn Target, vag: &Value) -> Vec<Diagnostic> {
    let mut missing: BTreeSet<(String, String)> = BTreeSet::new();
    let mut examples: std::collections::BTreeMap<(String, String), String> = Default::default();
    for n in vag["nodes"].as_array().into_iter().flatten() {
        let (k, f) = node_feature(n);
        if !t.supports(&k, &f) {
            examples
                .entry((k.clone(), f.clone()))
                .or_insert_with(|| n["id"].as_str().unwrap_or("").to_string());
            missing.insert((k, f));
        }
    }
    missing
        .into_iter()
        .map(|(k, f)| {
            let ex = examples[&(k.clone(), f.clone())].clone();
            Diagnostic::error(
                Code::TGT001,
                Span::default(),
                format!("ターゲット {} は {k} / {f} に未対応です(例: {ex})", t.kind().as_str()),
            )
            .hint(match t.kind() {
                TargetKind::Ios | TargetKind::Android => "モバイル生成は P1(M4)で実装予定です。現在は契約のみです",
                TargetKind::Web => "Web 生成は M2 で実装予定です(UI フレームワークは ADR-004 で決定)",
                TargetKind::Backend => "Backend 生成は M2 で実装予定です(現在は知識クエリの SQL 低水準化のみ)",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tc013_unsupported_features_stop_with_tgt001() {
        let vag = json!({"nodes": [
            {"id": "screen.a", "kind": "screen"},
            {"id": "q", "kind": "query", "detail": {}},
            {"id": "b", "kind": "display", "detail": {"ui": "button"}},
        ]});
        assert_eq!(check_support(&target_for(TargetKind::Ios), &vag).len(), 3);
        let be = check_support(&target_for(TargetKind::Backend), &vag);
        assert_eq!(be.len(), 2, "query.ranking だけは backend 対応");
        assert!(be.iter().all(|d| d.code == Code::TGT001));
        let only_query = json!({"nodes": [{"id": "q", "kind": "query", "detail": {}}]});
        assert!(check_support(&target_for(TargetKind::Backend), &only_query).is_empty());
    }

    #[test]
    fn target_names() {
        for t in ["web", "backend", "ios", "android"] {
            assert_eq!(TargetKind::parse(t).unwrap().as_str(), t);
        }
        assert!(TargetKind::parse("windows").is_none());
    }
}
