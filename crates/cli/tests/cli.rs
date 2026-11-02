//! `koto` CLI の結合テスト(終了コード契約: 技術仕様書 §14)。

use std::path::{Path, PathBuf};
use std::process::Command;

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");
const SPEC_SAMPLE: &str = include_str!("../../../tests/fixtures/spec_sample/app.koto");

fn koto(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_koto")).arg("-C").arg(dir).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn project(name: &str, src: &str, targets: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("kotonoha.json"),
        format!(r#"{{"name":"{name}","sources":["src/app.koto"],"targets":[{targets}]}}"#),
    )
    .unwrap();
    std::fs::write(dir.join("src/app.koto"), src).unwrap();
    dir
}

#[test]
fn check_shop_succeeds() {
    let d = project("shop_ok", SHOP, r#""web","backend""#);
    let (c, _, err) = koto(&d, &["check"]);
    assert_eq!(c, 0, "{err}");
    assert!(err.contains("検査に合格"));
}

#[test]
fn spec_sample_fails_with_machine_readable_diagnostics() {
    let d = project("spec_sample", SPEC_SAMPLE, r#""web""#);
    let (c, out, _) = koto(&d, &["--json", "check"]);
    assert_eq!(c, 2);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["exit_code"], 2);
    let diags = v["diagnostics"].as_array().unwrap();
    assert!(!diags.is_empty());
    for d in diags {
        for k in ["code", "severity", "message_ja", "file", "span", "hints"] {
            assert!(d.get(k).is_some(), "診断に {k} がありません: {d}");
        }
    }
}

#[test]
fn flexible_intent_resolve_approve_lock_and_locked_build_policy() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let d = project("flex_flow", &src, r#""backend""#);
    assert_eq!(koto(&d, &["check"]).0, 3, "未承認の曖昧さ");
    assert_eq!(koto(&d, &["build", "--target", "backend", "--locked"]).0, 3, "TC-004 未承認は本番ビルド拒否");
    std::fs::write(
        d.join("fixture.json"),
        r#"{"人気商品を表示": {"interpretations":[{"canonical":"売れ筋商品を表示","rationale":"人気=売れ筋(売上金額)"}],"assumptions":["人気は売上金額で測る"],"unresolved":[],"required_effects":["data.read"],"evidence_refs":["知識 売れ筋商品"]}}"#,
    )
    .unwrap();
    let (c, out, err) = koto(&d, &["--json", "resolve", "--adapter", "fixture", "--fixture", "fixture.json"]);
    assert_eq!(c, 0, "{err}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = v["candidates"][0]["intent_id"].as_str().unwrap().to_string();
    let (c, _, err) = koto(&d, &["approve", &id, "1", "--approver", "業務設計者A", "--at", "2026-11-03T00:00:00Z"]);
    assert_eq!(c, 0, "{err}");
    assert_eq!(koto(&d, &["check"]).0, 0);
    assert_eq!(koto(&d, &["approve", "--write-lock"]).0, 0);
    let (c, _, err) = koto(&d, &["build", "--target", "backend", "--locked"]);
    assert_eq!(c, 3, "署名基盤未決定のため承認入りの本番ビルドは拒否: {err}");
    assert!(err.contains("ADR-007"));
}

#[test]
fn offline_resolve_cannot_be_approved() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let d = project("offline_resolve", &src, r#""backend""#);
    let (c, out, _) = koto(&d, &["--json", "resolve"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = v["candidates"][0]["intent_id"].as_str().unwrap().to_string();
    assert!(v["candidates"][0]["interpretations"].as_array().unwrap().is_empty());
    let (c, _, err) = koto(&d, &["approve", &id, "1", "--approver", "x"]);
    assert_eq!(c, 3, "未確定事項が残る候補は承認不可: {err}");
}

#[test]
fn lock_stale_and_target_unsupported_exit_codes() {
    let d = project("lock_stale", SHOP, r#""web","backend","ios","android""#);
    assert_eq!(koto(&d, &["build", "--target", "backend", "--locked"]).0, 5, "ロックなし");
    assert_eq!(koto(&d, &["approve", "--write-lock"]).0, 0);
    let out = d.join("out-ios");
    assert_eq!(
        koto(&d, &["build", "--target", "ios", "--locked", "--out", out.to_str().unwrap()]).0,
        0,
        "ロック一致なら生成できる"
    );
    assert!(out.join("Sources/Screens.generated.swift").exists());
    let dec = project(
        "tgt_unsupported",
        &SHOP.replace("    在庫: 整数\n", "    在庫: 整数\n    重さ: 選択<小数>\n"),
        r#""android""#,
    );
    assert_eq!(koto(&dec, &["build", "--target", "android"]).0, 6, "TC-013 未対応の型");
    std::fs::write(d.join("src/app.koto"), SHOP.replace("件数 = 20", "件数 = 30")).unwrap();
    let (c, _, err) = koto(&d, &["build", "--target", "backend", "--locked"]);
    assert_eq!(c, 5, "TC-010 知識変更でロック失効: {err}");
    assert!(err.contains("knowledge_hash"));
}

#[test]
fn backend_build_writes_queries_for_supported_project() {
    let src = include_str!("../../../examples/ranking/src/app.koto")
        .split("\n画面 ")
        .next()
        .unwrap()
        .to_string();
    let d = project("backend_queries", &src, r#""backend""#);
    assert_eq!(koto(&d, &["approve", "--write-lock"]).0, 0);
    let (c, _, err) = koto(&d, &["build", "--target", "backend", "--locked"]);
    assert_eq!(c, 0, "{err}");
    let sql = std::fs::read_to_string(d.join("build/backend/queries/売れ筋商品.sqlite.sql")).unwrap();
    assert!(sql.contains("LIMIT ?4"));
}

#[test]
fn graph_is_reproducible_and_fmt_check_works() {
    let d = project("graph_fmt", SHOP, r#""web""#);
    let (c1, g1, _) = koto(&d, &["graph", "--canonical"]);
    let (c2, g2, _) = koto(&d, &["graph", "--canonical"]);
    assert_eq!((c1, c2), (0, 0));
    assert_eq!(g1, g2, "TC-009");
    assert_eq!(koto(&d, &["fmt", "--check"]).0, 0);
    std::fs::write(d.join("src/app.koto"), SHOP.replace('\n', "\r\n")).unwrap();
    assert_eq!(koto(&d, &["fmt", "--check"]).0, 2);
    assert_eq!(koto(&d, &["fmt"]).0, 0);
    assert_eq!(std::fs::read_to_string(d.join("src/app.koto")).unwrap(), SHOP);
}

#[test]
fn init_doctor_and_unimplemented_test_command() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("init_case");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    assert_eq!(koto(&base, &["init", "app", "--name", "見本"]).0, 0);
    assert_eq!(koto(&base.join("app"), &["check"]).0, 0, "init の雛形は検査を通る");
    assert_eq!(koto(&base, &["doctor"]).0, 0);
    assert_eq!(koto(&base.join("app"), &["test"]).0, 7, "未実装は成功扱いしない");
}
