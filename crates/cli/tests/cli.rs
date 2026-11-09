//! `koto` CLI の結合テスト(終了コード契約: 技術仕様書 §14)。

use std::path::{Path, PathBuf};
use std::process::Command;

const SHOP: &str = include_str!("../../../examples/shop/src/app.koto");
const SPEC_SAMPLE: &str = include_str!("../../../tests/fixtures/spec_sample/app.koto");

fn koto(dir: &Path, args: &[&str]) -> (i32, String, String) {
    // 利用者の学習記録(~/.kotonoha/learn)に書かないよう、存在しない場所を指す
    let learn = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-learn-dir");
    koto_env(dir, args, &[("KOTONOHA_LEARN_DIR", learn.to_str().unwrap())])
}

fn koto_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_koto"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.arg("-C").arg(dir).args(args).output().unwrap();
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
fn init_doctor_test_and_impact_commands() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("init_case");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    assert_eq!(koto(&base, &["init", "app", "--name", "見本"]).0, 0);
    assert_eq!(koto(&base.join("app"), &["check"]).0, 0, "init の雛形は検査を通る");
    assert_eq!(koto(&base, &["doctor"]).0, 0);
    let (c, _, err) = koto(&base.join("app"), &["test"]);
    assert_eq!(c, 0, "FR-019 契約テストが生成・実行される: {err}");
    // AT-08 影響分析
    let d = project("impact", SHOP, r#""web""#);
    let (c, out, _) = koto(&d, &["--json", "impact", "注文明細"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let nodes: Vec<&str> = v["impacts"].as_array().unwrap().iter().map(|i| i["node"].as_str().unwrap()).collect();
    assert!(nodes.contains(&"screen.売れ筋"), "{nodes:?}");
    let (_, base, _) = koto(&d, &["graph"]);
    std::fs::write(d.join("base.json"), base).unwrap();
    std::fs::write(d.join("src/app.koto"), SHOP.replace("期間 = 直近30日", "期間 = 直近7日")).unwrap();
    let (c, out, _) = koto(&d, &["--json", "impact", "--base", "base.json"]);
    assert_eq!(c, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let nodes: Vec<&str> = v["impacts"].as_array().unwrap().iter().map(|i| i["node"].as_str().unwrap()).collect();
    assert!(nodes.contains(&"screen.売れ筋"), "知識の意味変更 → 売れ筋画面に影響: {nodes:?}");
    assert!(!nodes.contains(&"screen.商品一覧"), "無関係な画面は含まない: {nodes:?}");
}

#[test]
fn adr007_signed_approvals_allow_locked_build_only_with_trusted_key() {
    let src = format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n");
    let d = project("signed_flow", &src, r#""backend""#);
    let keys = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("signed_flow_keys");
    let _ = std::fs::remove_dir_all(&keys);
    std::fs::create_dir_all(&keys).unwrap();
    let (c, trust, err) = koto(&keys, &["keygen", "--out", "approver.key", "--owner", "業務設計者A"]);
    assert_eq!(c, 0, "{err}");
    std::fs::write(keys.join("trust.json"), &trust).unwrap();
    let (c, other, _) = koto(&keys, &["keygen", "--out", "other.key"]);
    assert_eq!(c, 0);
    std::fs::write(keys.join("other-trust.json"), &other).unwrap();
    assert_ne!(koto(&keys, &["keygen", "--out", "approver.key"]).0, 0, "既存の鍵を上書きしない");

    std::fs::write(
        d.join("fixture.json"),
        r#"{"人気商品を表示": {"interpretations":[{"canonical":"売れ筋商品を表示","rationale":"r"}],"assumptions":[],"unresolved":[],"required_effects":["data.read"],"evidence_refs":[]}}"#,
    )
    .unwrap();
    let (_, out, _) = koto(&d, &["--json", "resolve", "--adapter", "fixture", "--fixture", "fixture.json"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = v["candidates"][0]["intent_id"].as_str().unwrap().to_string();
    let key = keys.join("approver.key");
    let (c, _, err) = koto(&d, &["approve", &id, "1", "--approver", "業務設計者A", "--sign-key", key.to_str().unwrap()]);
    assert_eq!(c, 0, "{err}");
    assert_eq!(koto(&d, &["approve", "--write-lock"]).0, 0);
    let trust = keys.join("trust.json");
    let (c, _, err) = koto(&d, &["build", "--target", "backend", "--locked", "--trust", trust.to_str().unwrap()]);
    assert_eq!(c, 0, "信頼鍵で署名を検証できれば本番ビルドできる: {err}");
    assert_eq!(koto(&d, &["build", "--target", "backend", "--locked"]).0, 3, "信頼鍵なしは拒否");
    let other = keys.join("other-trust.json");
    let (c, _, err) = koto(&d, &["build", "--target", "backend", "--locked", "--trust", other.to_str().unwrap()]);
    assert_eq!(c, 3, "別の鍵では拒否: {err}");
    assert!(err.contains("信頼されていません"));
}

#[test]
fn beginner_flow_templates_patterns_safety_and_suggestions() {
    let base = std::env::temp_dir().join(format!("koto-beginner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // ひな形の一覧
    let (c, out, _) = koto(&base, &["init", "--template"]);
    assert_eq!(c, 0);
    for t in ["予約受付", "在庫管理", "問い合わせ", "日報"] {
        assert!(out.contains(t), "{out}");
    }
    // 未知のひな形は日本語のエラー
    let (c, _, err) = koto(&base, &["init", "x", "--template", "謎"]);
    assert_ne!(c, 0);
    assert!(err.contains("使えるひな形"), "{err}");
    // すべてのひな形がそのまま検査に合格する
    for t in ["予約受付", "在庫管理", "問い合わせ", "日報"] {
        let (c, _, err) = koto(&base, &["init", t, "--template", t]);
        assert_eq!(c, 0, "{err}");
        let (c, _, err) = koto(&base.join(t), &["check"]);
        assert_eq!(c, 0, "{t}: {err}");
    }
    assert!(base.join("予約受付/seed.json").exists());
    // 書ける文の一覧は、宣言した名前で具体的に出る
    let (c, out, _) = koto(&base.join("予約受付"), &["patterns"]);
    assert_eq!(c, 0);
    assert!(out.contains("枠を一覧表示") && out.contains("予約は予約者が本人のものだけ閲覧可"), "{out}");
    // 安全対策と公開範囲
    let (c, out, _) = koto(&base.join("予約受付"), &["safety"]);
    assert_eq!(c, 0);
    assert!(out.contains("ログインした本人の予約だけ") && out.contains("CSRF"), "{out}");
    // 解釈できない文には近い書き方を示す
    let src = std::fs::read_to_string(base.join("予約受付/src/app.koto"))
        .unwrap()
        .replace("    枠を一覧表示\n", "    枠の一覧を表示する\n");
    std::fs::write(base.join("予約受付/src/app.koto"), src).unwrap();
    let (c, _, err) = koto(&base.join("予約受付"), &["check"]);
    assert_ne!(c, 0);
    assert!(err.contains("近い書き方") && err.contains("「枠を一覧表示」"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}

/// 模擬の手元モデル(OpenAI 互換)。過去の承認例を見せられたときだけ「売れ筋商品を表示」を先頭に出す。
/// good=false なら常に外れを返す(評価で採用を止められることの確認用)。
fn mock_model(good: bool) -> String {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let mut s = s.unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let body = loop {
                let n = s.read(&mut tmp).unwrap();
                buf.extend_from_slice(&tmp[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map(|x| x.trim().parse().unwrap())
                        .unwrap();
                    if buf.len() >= i + 4 + len {
                        break String::from_utf8_lossy(&buf[i + 4..]).to_string();
                    }
                }
            };
            let req: serde_json::Value = serde_json::from_str(&body).unwrap();
            let user = req["messages"][1]["content"].as_str().unwrap();
            let learned = user.contains("→「売れ筋商品を表示」");
            let its = if good && learned {
                serde_json::json!([{"canonical": "売れ筋商品を表示", "rationale": "過去の承認と同じく人気=売れ筋"}])
            } else {
                serde_json::json!([{"canonical": "商品を一覧表示", "rationale": "人気=全商品"}, {"canonical": "売れ筋商品を表示", "rationale": "人気=売れ筋"}])
            };
            let content = serde_json::json!({"interpretations": its, "assumptions": [], "unresolved": []}).to_string();
            let resp = serde_json::json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string();
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp}",
                    resp.len()
                )
                .as_bytes(),
            );
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn first_candidate(d: &Path) -> (String, Vec<String>) {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join("kotonoha.candidates.json")).unwrap()).unwrap();
    let c = &v["candidates"][0];
    (
        c["intent_id"].as_str().unwrap().to_string(),
        c["interpretations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["canonical"].as_str().unwrap().to_string())
            .collect(),
    )
}

#[test]
fn local_open_model_grows_from_approvals() {
    let learn = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("learn-grow");
    let _ = std::fs::remove_dir_all(&learn);
    let env = [("KOTONOHA_LEARN_DIR", learn.to_str().unwrap())];
    let ep = mock_model(true);
    let k = |d: &Path, args: &[&str]| koto_env(d, args, &env);

    let (c, _, err) = k(Path::new("."), &["learn", "status"]);
    assert_eq!(c, 0);
    assert!(err.contains("まだ有効ではありません"), "{err}");
    assert_eq!(k(Path::new("."), &["learn", "init"]).0, 0);

    // 1 つ目のプロジェクト: まだ何も学んでいないので、外れが先頭に来る → 人が 2 番目を承認
    let d1 = project("learn_p1", &format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n"), r#""backend""#);
    let (c, _, err) = k(&d1, &["resolve", "--adapter", "local", "--model", "m", "--endpoint", &ep]);
    assert_eq!(c, 0, "{err}");
    let (id, its) = first_candidate(&d1);
    assert_eq!(its[0], "商品を一覧表示");
    let (c, _, err) = k(&d1, &["approve", &id, "2", "--approver", "花子"]);
    assert_eq!(c, 0, "{err}");
    assert!(err.contains("学習記録に追加"), "{err}");

    // 2 つ目のプロジェクト: 似た文。過去の承認を例として見せるので、承認された解釈が先頭に来る
    let d2 = project("learn_p2", &format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気の商品を表示\n"), r#""backend""#);
    k(&d2, &["resolve", "--adapter", "local", "--model", "m", "--endpoint", &ep]);
    let (id2, its) = first_candidate(&d2);
    assert_eq!(its[0], "売れ筋商品を表示", "承認から学んで当たりやすくなる");
    assert_eq!(k(&d2, &["approve", &id2, "1", "--approver", "花子"]).0, 0);

    // 同じ文ならモデルなしでも過去の承認から候補が出る
    let d3 = project("learn_p3", &format!("{SHOP}\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n"), r#""backend""#);
    k(&d3, &["resolve", "--adapter", "memory"]);
    assert_eq!(first_candidate(&d3).1, vec!["売れ筋商品を表示".to_string()]);

    // 追加学習用データ
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("learn-export");
    let (c, _, err) = k(&d1, &["learn", "export", "--out", out.to_str().unwrap()]);
    assert_eq!(c, 0, "{err}");
    assert_eq!(std::fs::read_to_string(out.join("sft.jsonl")).unwrap().lines().count(), 2);
    assert_eq!(
        std::fs::read_to_string(out.join("dpo.jsonl")).unwrap().lines().count(),
        1,
        "選ばれなかった候補が負例になる"
    );
    assert!(out.join("README.md").exists());

    // 評価: 良いモデルは基準を満たす。悪いモデルは --min で止まる。推移は status で見られる
    let (c, _, err) = k(&d1, &["learn", "eval", "--model", "good", "--endpoint", &ep, "--min", "0.9"]);
    assert_eq!(c, 0, "{err}");
    let bad = mock_model(false);
    let (c, _, err) = k(&d1, &["learn", "eval", "--model", "bad", "--endpoint", &bad, "--min", "0.9"]);
    assert_ne!(c, 0);
    assert!(err.contains("採用しないでください"), "{err}");
    let (_, _, err) = k(&d1, &["learn", "status"]);
    assert!(err.contains("承認した言い換え 2 件") && err.contains("good") && err.contains("bad"), "{err}");

    // 機械の外のサーバーには送らない
    let (c, _, err) = k(
        &d1,
        &["resolve", "--adapter", "local", "--model", "m", "--endpoint", "https://api.example.com/v1"],
    );
    assert_ne!(c, 0);
    assert!(err.contains("ADR-006"), "{err}");
}

#[test]
fn scenarios_run_in_koto_test_and_report_in_japanese() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scenario_tpl");
    let _ = std::fs::remove_dir_all(&root);
    let (c, _, err) = koto(Path::new("."), &["init", root.to_str().unwrap(), "--template", "予約受付"]);
    assert_eq!(c, 0, "{err}");
    let (c, _, err) = koto(&root, &["test"]);
    assert_eq!(c, 0, "{err}");
    assert!(
        err.contains("✓ 花子(利用者)が 予約する(s2, 1) → 成功") && err.contains("すべて確かめました"),
        "{err}"
    );
    // 期待と違う確かめるは ✗ と実際の値を出し、終了コード 1
    let src = std::fs::read_to_string(root.join("src/app.koto"))
        .unwrap()
        .replace("枠(s2)の予約数は 2", "枠(s2)の予約数は 5");
    std::fs::write(root.join("src/app.koto"), src).unwrap();
    let (c, _, err) = koto(&root, &["test"]);
    assert_eq!(c, 1);
    assert!(err.contains("✗ 枠(s2)の予約数は 5  (実際は: 2)"), "{err}");
    // 書き方の誤りは koto check で分かる
    let src = std::fs::read_to_string(root.join("src/app.koto"))
        .unwrap()
        .replace("枠(s2)の予約数は 5", "花子(店長)が 予約する(s1, 1) → 成功");
    std::fs::write(root.join("src/app.koto"), src).unwrap();
    let (c, _, err) = koto(&root, &["check"]);
    assert_ne!(c, 0);
    assert!(err.contains("役割 `店長` はありません"), "{err}");
}

#[test]
fn theme_color_in_project_file_reaches_web_css_and_rejects_unreadable_colors() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("theme_tpl");
    let _ = std::fs::remove_dir_all(&root);
    let (c, _, err) = koto(Path::new("."), &["init", root.to_str().unwrap(), "--template", "問い合わせ"]);
    assert_eq!(c, 0, "{err}");
    let set = |color: &str| {
        let p = root.join("kotonoha.json");
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v["theme"] = serde_json::json!({ "color": color });
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    };
    set("緑");
    let out = root.join("out-web");
    let (c, _, err) = koto(&root, &["build", "--target", "web", "--out", out.to_str().unwrap()]);
    assert_eq!(c, 0, "{err}");
    let css = std::fs::read_to_string(out.join("assets/app.css")).unwrap();
    assert!(css.contains(":root { --accent: #1d6b45; }"), "テーマ色が入る");
    // ボタンの白い文字が読めない明るい色は、作る前に止める
    set("#ffee58");
    let (c, _, err) = koto(&root, &["check"]);
    assert_ne!(c, 0);
    assert!(err.contains("明るすぎ") && err.contains("藍"), "{err}");
}
