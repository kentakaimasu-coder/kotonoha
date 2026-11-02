//! パイプライン全体のテスト(受入テストケース TC-xxx との対応は docs/spec/test_spec.md)。

use super::*;
use kotonoha_diag::Code;
use kotonoha_lock::ApprovalRecord;

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");
const RANKING: &str = include_str!("../../../examples/ranking/src/app.koto");
const GOLDEN_VAG: &str = include_str!("../../../examples/shop/expected/vag.json");

fn shop() -> Project {
    project_from_sources("商店", &[("src/app.koto", SHOP)], &["web", "backend", "ios", "android"])
}

fn with_src(src: &str) -> Project {
    project_from_sources("t", &[("src/app.koto", src)], &["web", "backend"])
}

fn none() -> ApprovalsFile {
    ApprovalsFile::empty()
}

#[test]
fn shop_is_validated_and_matches_golden_vag() {
    let a = analyze(&shop(), &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    assert_eq!(a.vag["status"], "validated");
    let pretty = serde_json::to_string_pretty(&a.vag).unwrap() + "\n";
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/shop/expected/vag.json"), &pretty).unwrap();
    }
    assert_eq!(
        pretty, GOLDEN_VAG,
        "ゴールデン VAG と一致しません(意図した変更なら UPDATE_GOLDEN=1 で更新しレビューする)"
    );
}

#[test]
fn tc009_same_input_gives_identical_canonical_vag_without_ai() {
    let a1 = analyze(&shop(), &none());
    let a2 = analyze(&shop(), &none());
    assert_eq!(kotonoha_graph::canonical_json(&a1.vag), kotonoha_graph::canonical_json(&a2.vag));
    assert_eq!(a1.hashes, a2.hashes);
}

#[test]
fn tc004_at05_unapproved_ambiguity_blocks_build() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let p = with_src(&src);
    let a = analyze(&p, &none());
    assert!(!a.ok());
    assert_eq!(a.diags.exit_code(), 3);
    assert_eq!(a.vag["status"], "draft");
    assert_eq!(a.unresolved().len(), 1);
}

#[test]
fn tc003_strict_mode_undefined_term_asks_question() {
    let src = format!("{SHOP}\n画面 人気:\n    人気商品を表示\n");
    let a = analyze(&with_src(&src), &none());
    let d: Vec<_> = a.diags.errors().collect();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, Code::AMB001);
    assert_eq!(d[0].file, "src/app.koto");
    assert!(d[0].hints.iter().any(|h| h.starts_with("質問:")));
}

fn approve_flow(src: &str, canonical: &str) -> (Project, ApprovalsFile, Analysis) {
    let p = with_src(src);
    let a = analyze(&p, &none());
    let u = a.unresolved()[0].clone();
    let fx = kotonoha_resolver::FixtureAdapter::from_json(&format!(
        r#"{{"{}": {{"interpretations":[{{"canonical":"{canonical}","rationale":"r"}},{{"canonical":"DROP TABLE 商品","rationale":"不正"}}],"assumptions":[],"unresolved":[],"required_effects":["data.read"],"evidence_refs":[]}}}}"#,
        u.sentence
    ))
    .unwrap();
    use kotonoha_resolver::ResolverAdapter;
    let mut cand = kotonoha_resolver::validate_candidate(&fx.propose(&u).unwrap(), &u).unwrap();
    let rejected = filter_candidate(&p, &none(), &mut cand, &u);
    assert_eq!(rejected.len(), 1, "成立しない解釈は除外される");
    assert_eq!(cand.interpretations.len(), 1);
    let mut ap = none();
    ap.upsert(ApprovalRecord::new(
        &u.intent_id,
        &u.sentence,
        &u.sentence_hash,
        &cand.interpretations[0].canonical,
        "業務設計者A",
        "2026-11-03T00:00:00Z",
        "",
    ));
    let a2 = analyze(&p, &ap);
    (p, ap, a2)
}

#[test]
fn approval_resolves_flexible_intent_and_is_recorded_in_vag() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let (_, _, a) = approve_flow(&src, "売れ筋商品を表示");
    assert!(a.ok(), "{:#?}", a.diags.items);
    let node = a.vag["nodes"].as_array().unwrap().iter().find(|n| n["sentence"] == "人気商品を表示").unwrap();
    assert_eq!(node["kind"], "query");
    assert_eq!(node["resolution"]["via"], "approved");
}

#[test]
fn locked_build_with_approvals_is_refused_until_signing_is_decided() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let (p, ap, a) = approve_flow(&src, "売れ筋商品を表示");
    let lock = compute_lock(&p, &a, &ap).to_pretty_json();
    let d = verify_lock(&p, &a, &ap, Some(&lock));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, Code::APP001, "自己申告の承認だけで本番ビルドを通さない(SEC-09)");
}

#[test]
fn tampered_approval_is_rejected() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let (p, mut ap, _) = approve_flow(&src, "売れ筋商品を表示");
    ap.records[0].approver = "なりすまし".into();
    let a = analyze(&p, &ap);
    assert!(a.diags.codes().contains(&Code::APP001));
    assert!(a.diags.codes().contains(&Code::AMB001), "改ざん記録は適用されない");
}

#[test]
fn lock_roundtrip_and_mutation_tests() {
    let p = shop();
    let a = analyze(&p, &none());
    let lock = compute_lock(&p, &a, &none()).to_pretty_json();
    assert!(verify_lock(&p, &a, &none(), Some(&lock)).is_empty(), "同一入力ならロック一致");

    let stale = |src: String| -> Vec<String> {
        let p2 = project_from_sources("商店", &[("src/app.koto", &src)], &["web", "backend", "ios", "android"]);
        let a2 = analyze(&p2, &none());
        assert!(a2.ok(), "{:#?}", a2.diags.items);
        let d = verify_lock(&p2, &a2, &none(), Some(&lock));
        assert!(d.iter().all(|x| x.code == Code::LOCK001));
        d.iter().map(|x| x.message_ja.clone()).collect()
    };
    // TC-010 知識変更 → LOCK001(knowledge_hash)
    let m = stale(SHOP.replace("件数 = 20", "件数 = 10"));
    assert!(m.iter().any(|x| x.contains("knowledge_hash")), "{m:#?}");
    // スキーマ変更
    let m = stale(SHOP.replace("    在庫: 整数\n", "    在庫: 整数\n    説明: 文字列\n"));
    assert!(m.iter().any(|x| x.contains("schema_hash")), "{m:#?}");
    // コメントだけの変更でも source_hash は変わる(意味は同じでも再承認が必要)
    let m = stale(SHOP.replace("アプリ「商店」", "アプリ「商店」 # 変更"));
    assert!(m.iter().any(|x| x.contains("source_hash")), "{m:#?}");
    assert!(!m.iter().any(|x| x.contains("knowledge_hash")));
    // ロックなし
    assert_eq!(verify_lock(&p, &a, &none(), None)[0].code, Code::LOCK001);
}

#[test]
fn tc013_mobile_and_unimplemented_targets_stop_with_tgt001() {
    let p = shop();
    let a = analyze(&p, &none());
    for t in ["ios", "android", "web", "backend"] {
        let e = build_target(&p, &a, t).err().unwrap_or_else(|| panic!("{t} は未対応機能を含むはず"));
        assert!(e.iter().all(|d| d.code == Code::TGT001), "{t}");
    }
    let p2 = with_src(SHOP);
    assert!(build_target(&p2, &a, "ios").is_err(), "targets にないターゲット");
}

#[test]
fn backend_lowers_knowledge_queries_when_everything_is_supported() {
    // 画面を持たない(=backend が対応済みの機能だけの)プロジェクト
    let only_knowledge: String = RANKING.split("\n画面 ").next().unwrap().to_string();
    let p = with_src(&only_knowledge);
    let a = analyze(&p, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    // 知識はノードではないので backend はクエリを生成できる(Web は vag だけでも未対応機能なし)
    let out = build_target(&p, &a, "backend").unwrap();
    let names: Vec<&str> = out.files.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["vag.json", "queries/売れ筋商品.sqlite.sql", "queries/売れ筋商品.postgres.sql"]);
    assert_eq!(out.files[1].1, include_str!("../../../examples/ranking/expected/ranking.sqlite.sql"));
}

#[test]
fn spec_sample_reports_all_phase0_findings() {
    let src = include_str!("../../../tests/fixtures/spec_sample/app.koto");
    let a = analyze(&with_src(src), &none());
    let codes = a.diags.codes();
    assert!(codes.contains(&Code::SYN001), "注文IDを返す");
    assert!(codes.contains(&Code::REF001), "注文ID/注文失敗/アダプタ未定義");
    assert!(codes.contains(&Code::AMB001), "個人情報は本人だけ閲覧");
    assert_eq!(a.diags.exit_code(), 2);
}

#[test]
fn normalize_text_rules() {
    assert_eq!(normalize_text("\u{FEFF}a  \r\nb\n\n\n"), "a\nb\n");
    assert_eq!(normalize_text("\u{304B}\u{3099}"), "\u{304C}\n");
    assert_eq!(normalize_text("x"), "x\n");
}

#[test]
fn rfc3339_shape() {
    let t = now_rfc3339();
    assert_eq!(t.len(), 20);
    assert!(t.ends_with('Z') && t.as_bytes()[10] == b'T');
}

#[test]
fn nfr004_check_1000_lines_under_5_seconds() {
    // 開発機基準は未固定(NFR-004)。ここでは CI 上の上限確認として 1,000 行超のソースを全段検査する。
    let mut src = String::from(SHOP);
    let mut i = 0;
    while src.lines().count() < 1000 {
        src.push_str(&format!(
            "\nデータ 追加{i}:\n    ID: 識別子\n    名前: 文字列\n    数: 整数\n\n画面 一覧{i}:\n    追加{i}を一覧表示\n"
        ));
        i += 1;
    }
    let p = with_src(&src);
    let t = std::time::Instant::now();
    let a = analyze(&p, &none());
    let el = t.elapsed();
    assert!(a.ok(), "{:#?}", a.diags.items.first());
    assert!(el.as_secs_f64() < 5.0, "{el:?}");
}
