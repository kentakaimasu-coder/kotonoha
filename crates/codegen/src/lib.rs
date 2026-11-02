//! ターゲット別生成の契約(技術仕様書 §11)とクエリの低水準化。
//!
//! 各ターゲットは `supports(node_kind, feature)` を持ち、VAG に未対応の機能が 1 つでもあれば
//! 黙って欠落させずに TGT001 で停止する。
//!
//! 現状の実装範囲(正直な申告):
//! - backend: 知識(集計ランキング)の SQL 低水準化のみ実装(`query.ranking`)。HTTP/認可/CRUD 生成は未実装
//! - web: 未実装(UI 生成フレームワークは ADR-004 で未決定)
//! - ios / android: 契約(本モジュールの trait と support 表)のみ。生成は P1

pub mod android;
pub mod appir;
pub mod backend;
pub mod ios;
pub mod sql;
pub mod ts;
pub mod web;

/// 生成ファイル(ターゲットの出力ディレクトリからの相対パス)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

impl GeneratedFile {
    pub fn text(path: &str, content: String) -> Self {
        GeneratedFile {
            path: path.to_string(),
            content,
        }
    }
}

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

pub const ALL_FEATURES: &[(&str, &str)] = &[
    ("screen", "screen.page"),
    ("display", "ui.list"),
    ("display", "ui.detail"),
    ("display", "ui.button"),
    ("display", "ui.form"),
    ("query", "query.ranking"),
    ("navigation", "nav.select"),
    ("action", "action.strict"),
    ("validation", "policy.public_read"),
    ("validation", "policy.auth_required"),
];

/// 検証状況(doctor と docs/status.md 用)。生成コードを実際にビルド・実行して確認したか。
pub fn verification_status(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Backend => "生成・起動・API テスト済み(Node.js 22)",
        TargetKind::Web => "生成・ビルド・ブラウザ E2E 済み(React SSR)",
        TargetKind::Ios => "生成のみ(Swift ツールチェーンなしのためコンパイル未検証)",
        TargetKind::Android => "生成のみ(Android SDK なしのためビルド未検証)",
    }
}

pub fn target_for(kind: TargetKind) -> SupportTable {
    let supported: &'static [(&'static str, &'static str)] = match kind {
        // 全ターゲットが同じ機能集合に対応する(UI ノードは backend ではデータ API の提供として、
        // action はクライアントでは API 呼出として扱う)。型レベルの未対応は各生成器が TGT001 を返す。
        TargetKind::Backend | TargetKind::Web | TargetKind::Ios | TargetKind::Android => ALL_FEATURES,
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
        // 全ターゲットが現行の機能集合に対応する
        let vag = json!({"nodes": [
            {"id": "screen.a", "kind": "screen"},
            {"id": "q", "kind": "query", "detail": {}},
            {"id": "b", "kind": "display", "detail": {"ui": "button"}},
            {"id": "f", "kind": "display", "detail": {"ui": "form"}},
        ]});
        for t in [TargetKind::Web, TargetKind::Backend, TargetKind::Ios, TargetKind::Android] {
            assert!(check_support(&target_for(t), &vag).is_empty(), "{t:?}");
        }
        // 未知の機能は黙って落とさず TGT001
        let unknown = json!({"nodes": [{"id": "x", "kind": "display", "detail": {"ui": "map"}}, {"id": "y", "kind": "display", "detail": {"ui": "chart"}}]});
        let d = check_support(&target_for(TargetKind::Ios), &unknown);
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.code == Code::TGT001));
    }

    #[test]
    fn target_names() {
        for t in ["web", "backend", "ios", "android"] {
            assert_eq!(TargetKind::parse(t).unwrap().as_str(), t);
        }
        assert!(TargetKind::parse("windows").is_none());
    }
}
