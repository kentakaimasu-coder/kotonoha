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
    let m = stale(SHOP.replace("    在庫: 整数\n", "    在庫: 整数\n    説明: 選択<文字列>\n"));
    assert!(m.iter().any(|x| x.contains("schema_hash")), "{m:#?}");
    // コメントだけの変更でも source_hash は変わる(意味は同じでも再承認が必要)
    let m = stale(SHOP.replace("アプリ「商店」", "アプリ「商店」 # 変更"));
    assert!(m.iter().any(|x| x.contains("source_hash")), "{m:#?}");
    assert!(!m.iter().any(|x| x.contains("knowledge_hash")));
    // ロックなし
    assert_eq!(verify_lock(&p, &a, &none(), None)[0].code, Code::LOCK001);
}

#[test]
fn tc013_unsupported_types_stop_every_target_with_tgt001() {
    // 小数は精度方針(ADR-0106)が未決定のため、どのターゲットも生成しない
    let src = SHOP.replace("    在庫: 整数\n", "    在庫: 整数\n    重さ: 選択<小数>\n");
    let p = project_from_sources("商店", &[("src/app.koto", &src)], &["web", "backend", "ios", "android"]);
    let a = analyze(&p, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    for t in ["ios", "android", "web", "backend"] {
        let e = build_target(&p, &a, t).err().unwrap_or_else(|| panic!("{t} は小数を生成できないはず"));
        assert!(e.iter().all(|d| d.code == Code::TGT001), "{t}: {e:#?}");
    }
    let p2 = with_src(SHOP);
    let a2 = analyze(&p2, &none());
    assert_eq!(build_target(&p2, &a2, "ios").err().unwrap()[0].code, Code::TGT001, "targets にないターゲット");
}

#[test]
fn all_targets_generate_and_share_one_api_contract() {
    // インターフェース整合性: 全ターゲットの api.json が同一で、クライアントが使う動作はすべて契約にある
    let p = shop();
    let a = analyze(&p, &none());
    let mut contracts = Vec::new();
    for t in ["backend", "web", "ios", "android"] {
        let out = build_target(&p, &a, t).unwrap_or_else(|e| panic!("{t}: {e:#?}"));
        let api = out.files.iter().find(|f| f.0 == "api.json").unwrap().1.clone();
        contracts.push(api);
        let v: serde_json::Value = serde_json::from_str(&contracts[0]).unwrap();
        let actions: Vec<String> = v["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["method"] == "POST")
            .map(|e| e["segments"][2].as_str().unwrap().to_string())
            .collect();
        let code: String = out.files.iter().filter(|f| f.0.contains("generated")).map(|f| f.1.as_str()).collect();
        for name in ["注文する", "商品登録"] {
            assert!(actions.contains(&name.to_string()));
            assert!(code.contains(&format!("\"{name}\"")), "{t} が {name} を呼んでいない");
        }
    }
    assert!(contracts.windows(2).all(|w| w[0] == w[1]), "ターゲット間で API 契約が異なる");
}

#[test]
fn backend_also_lowers_knowledge_queries() {
    let p = with_src(RANKING);
    let a = analyze(&p, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    let out = build_target(&p, &a, "backend").unwrap();
    let sql = out.files.iter().find(|f| f.0 == "queries/売れ筋商品.sqlite.sql").unwrap();
    assert_eq!(sql.1, include_str!("../../../examples/ranking/expected/ranking.sqlite.sql"));
    assert!(out.files.iter().any(|f| f.0 == "src/app.generated.ts"));
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

/// K-12: 生成コード(手書きランタイムを除く)のゴールデン。意図した変更なら UPDATE_GOLDEN=1 で更新し差分をレビューする。
#[test]
fn generated_code_matches_golden_snapshots() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/shop/expected/generated"));
    let p = shop();
    let a = analyze(&p, &none());
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let mut diffs = Vec::new();
    for t in ["backend", "web", "ios", "android"] {
        let out = build_target(&p, &a, t).unwrap();
        for (path, content) in out
            .files
            .iter()
            .filter(|(p, _)| p.contains(".generated.") || p == "api.json" || p.ends_with("MainActivity.kt") || p == "tests/actions.todo.test.ts")
        {
            let file = root.join(t).join(path);
            if update {
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(&file, content).unwrap();
            } else {
                match std::fs::read_to_string(&file) {
                    Ok(g) if &g == content => {}
                    Ok(_) => diffs.push(format!("{t}/{path} が変わりました")),
                    Err(_) => diffs.push(format!("{t}/{path} のゴールデンがありません")),
                }
            }
        }
    }
    assert!(
        diffs.is_empty(),
        "{diffs:#?}\n意図した変更なら UPDATE_GOLDEN=1 cargo test -p kotonoha-driver で更新し、差分をレビューしてください"
    );
}

// ---- 拡張(関数・外部関数・分析・部品・実験): ADR-0122〜0125 / NR-05〜08 ----

const LAB: &str = include_str!("../../../examples/lab/src/app.koto");

fn lab_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ext/pricing.ts", include_str!("../../../examples/lab/ext/pricing.ts")),
        ("ml/forecast.py", include_str!("../../../examples/lab/ml/forecast.py")),
        (
            "analytics/sales_by_product.sql",
            include_str!("../../../examples/lab/analytics/sales_by_product.sql"),
        ),
        ("analytics/sales_since.sql", include_str!("../../../examples/lab/analytics/sales_since.sql")),
        ("ext/StarRating.ts", include_str!("../../../examples/lab/ext/StarRating.ts")),
        ("ext/StarRating.swift", include_str!("../../../examples/lab/ext/StarRating.swift")),
        ("ext/StarRating.kt", include_str!("../../../examples/lab/ext/StarRating.kt")),
        ("ext/NumberRating.ts", include_str!("../../../examples/lab/ext/NumberRating.ts")),
        ("ext/NumberRating.swift", include_str!("../../../examples/lab/ext/NumberRating.swift")),
        ("ext/NumberRating.kt", include_str!("../../../examples/lab/ext/NumberRating.kt")),
    ]
}

fn lab_with(src: &str, files: &[(&str, &str)]) -> Project {
    let mut p = project_from_sources("実験室", &[("src/app.koto", src)], &["web", "backend", "ios", "android"]);
    for (k, v) in files {
        p.ext_files.insert(k.to_string(), v.to_string());
    }
    p
}

fn lab() -> Project {
    lab_with(LAB, &lab_files())
}

fn errors_of(p: &Project) -> String {
    analyze(p, &none())
        .diags
        .items
        .iter()
        .filter(|d| d.severity == kotonoha_diag::Severity::Error)
        .map(|d| format!("{}: {}", d.code.as_str(), d.message_ja))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn lab_example_is_validated_and_records_extension_hashes() {
    let a = analyze(&lab(), &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    let x = &a.vag["extensions"];
    assert_eq!(x["functions"].as_array().unwrap().len(), 2);
    assert_eq!(x["analyses"][0]["tables"], serde_json::json!(["注文"]));
    assert_eq!(x["experiments"].as_array().unwrap().len(), 2);
    assert_eq!(x["functions"][0]["file"]["sha256"].as_str().unwrap().len(), 64);
    for t in ["backend", "web", "ios", "android"] {
        build_target(&lab(), &a, t).unwrap_or_else(|d| panic!("{t}: {d:#?}"));
    }
}

#[test]
fn changing_an_implementation_file_changes_the_graph_hash() {
    let a = analyze(&lab(), &none());
    let mut files = lab_files();
    files[0].1 = "export function 割引額(価格: number): number { return 0; }\n";
    let b = analyze(&lab_with(LAB, &files), &none());
    assert!(b.ok());
    // 実装を差し替えるとロックが合わなくなる(承認なしに挙動を変えられない)
    assert_ne!(a.hashes.graph, b.hashes.graph);
}

#[test]
fn pure_functions_reject_side_effects_time_and_principal() {
    let base = "アプリ「t」\nデータ 商品:\n    ID: 識別子\n    在庫: 整数\nアダプタ 商品を取得(ID: 識別子) -> 商品:\n    操作 = 取得(商品)\n";
    for (body, want) in [
        ("関数 f(x: 識別子) -> 整数:\n    商品を取得(x).在庫 を返す\n", "副作用"),
        ("関数 f() -> 日時:\n    現在日時() を返す\n", "現在日時"),
        ("関数 f() -> 識別子:\n    本人 を返す\n", "本人"),
        ("関数 f(x: 整数) -> 整数:\n    もし x > 0:\n        x を返す\n", "全ての経路"),
        ("関数 f(x: 整数) -> 文字列:\n    x を返す\n", "TYP"),
        ("関数 f(x: 整数) -> 整数:\n    長さ(x) を返す\n", "TYP"),
    ] {
        let e = errors_of(&with_src(&format!("{base}{body}")));
        assert!(e.contains(want), "{body}\n→ {e}");
    }
}

#[test]
fn external_files_must_stay_inside_the_project_and_exist() {
    for (path, want) in [
        ("../secret.ts", "相対パス"),
        ("/etc/passwd.ts", "相対パス"),
        ("C:\\\\x.ts", "相対パス"),
        ("ext/none.ts", "読めません"),
        ("ext/pricing.rb", "拡張子"),
    ] {
        let src = LAB.replace("実装 = \"ext/pricing.ts\"", &format!("実装 = \"{path}\""));
        let e = errors_of(&lab_with(&src, &lab_files()));
        assert!(e.contains(want), "{path} → {e}");
    }
    // 実際のファイルシステムでもシンボリックリンク等で外へ出られない
    let dir = std::env::temp_dir().join(format!("koto-ext-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("ext")).unwrap();
    let mut p = lab();
    p.root = dir.clone();
    p.ext_files.clear();
    assert!(p.read_ext("ext/none.ts").is_err());
    #[cfg(unix)]
    {
        // プロジェクトの外に実在するファイルを作り、そこを指すリンクを置く(OS に依存しない)
        let outside = std::env::temp_dir().join(format!("koto-outside-{}.ts", std::process::id()));
        std::fs::write(&outside, "export {};\n").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("ext/link.ts")).unwrap();
        let err = p.read_ext("ext/link.ts").unwrap_err();
        std::fs::remove_file(&outside).ok();
        assert!(err.contains("外"), "{err}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn analyses_must_be_read_only_select_over_declared_tables() {
    for (sql, want) in [
        ("DELETE FROM \"注文\"", "SELECT"),
        ("SELECT * FROM \"商品\"", "宣言されていません"),
        ("SELECT 1; SELECT 2", "1 文"),
        ("SELECT load_extension('x') FROM \"注文\"", "load_extension"),
        ("SELECT \"商品ID\" FROM \"注文\" WHERE \"ID\" = ?", ":名前"),
    ] {
        let mut files = lab_files();
        files[2].1 = sql;
        let e = errors_of(&lab_with(LAB, &files));
        assert!(e.contains(want), "{sql} → {e}");
    }
    // 対象に未定義のデータ・戻り値が表でない
    let e = errors_of(&lab_with(
        &LAB.replace(
            "対象 = [注文]\n    SQL = \"analytics/sales_by_product.sql\"",
            "対象 = [謎]\n    SQL = \"analytics/sales_by_product.sql\"",
        ),
        &lab_files(),
    ));
    assert!(e.contains("謎"), "{e}");
    // 引数を取る分析は画面に直接出せない
    let e = errors_of(&lab_with(&LAB.replace("    商品別売上を表示", "    期間売上を表示"), &lab_files()));
    assert!(e.contains("引数"), "{e}");
}

#[test]
fn experiments_are_checked_against_screens_variants_weights_and_goal() {
    for (from, to, want) in [
        (
            "割合 = [50, 50]\n    成果 = 購入する\n\n実験 評価",
            "割合 = [50, 40]\n    成果 = 購入する\n\n実験 評価",
            "合計 100",
        ),
        ("変種 = [購入, 今すぐ購入]", "変種 = [買う, 今すぐ購入]", "ラベル「買う」"),
        ("変種 = [購入, 今すぐ購入]", "変種 = [購入]", "2〜10"),
        ("変種 = [星評価, 数字評価]", "変種 = [星評価, 謎の部品]", "謎の部品"),
        ("成果 = 購入する\n\n実験 評価", "成果 = 謎の動作\n\n実験 評価", "謎の動作"),
        ("    画面 = 商品詳細\n    変種 = [購入", "    画面 = 謎の画面\n    変種 = [購入", "謎の画面"),
        (
            "    割合 = [50, 50]\n    成果 = 購入する\n\n実験 評価",
            "    割合 = [50, 50]\n    色 = 赤\n    成果 = 購入する\n\n実験 評価",
            "色",
        ),
    ] {
        assert!(LAB.contains(from), "{from}");
        let e = errors_of(&lab_with(&LAB.replacen(from, to, 1), &lab_files()));
        assert!(e.contains(want), "{to} → {e}");
    }
    // 同じ対象に実験が 2 つ
    let dup = LAB.to_string() + "\n実験 もう一つ:\n    画面 = 商品詳細\n    変種 = [購入, 買う]\n    割合 = [50, 50]\n    成果 = 購入する\n";
    assert!(errors_of(&lab_with(&dup, &lab_files())).contains("2 つ"));
}

#[test]
fn components_without_a_platform_implementation_stop_that_target_with_tgt001() {
    let src = LAB.replace("    android = \"ext/StarRating.kt\"\n", "");
    let p = lab_with(&src, &lab_files());
    let a = analyze(&p, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    assert!(build_target(&p, &a, "web").is_ok());
    let e = build_target(&p, &a, "android").err().expect("android は止まるはず");
    assert!(e.iter().any(|d| d.code == Code::TGT001 && d.message_ja.contains("星評価")), "{e:#?}");
}

#[test]
fn external_functions_declared_effects_flow_into_actions() {
    // 外部関数が network.send を宣言 → 動作の効果宣言に無ければ SEC001
    let src = LAB
        .replace("    実装 = \"ext/pricing.ts\"", "    実装 = \"ext/pricing.ts\"\n    効果 = [network.send]")
        .replace(
            "    必要 = [注文.作成, 商品.読取]\n",
            "    必要 = [注文.作成, 商品.読取]\n    効果 = [data.read, data.write]\n",
        );
    let e = errors_of(&lab_with(&src, &lab_files()));
    assert!(e.contains("SEC001") && e.contains("network.send"), "{e}");
}

#[test]
fn roles_are_checked_and_normalized_to_contract_capabilities() {
    let base = "アプリ「t」\nデータ 注文:\n    ID: 識別子\n    卓: 識別子\n動作 何か() -> 識別子:\n    必要 = []\n    厳密:\n        \"x\" を返す\n";
    let ok = with_src(&format!(
        "{base}役割 厨房:\n    権限 = [注文.読取, 注文.update, 全件閲覧.注文, 動作.何か, 実験.結果]\n"
    ));
    let a = analyze(&ok, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    assert_eq!(a.env.roles["厨房"], vec!["全件閲覧.注文", "動作.何か", "実験.結果", "注文.read", "注文.update"]);
    assert_eq!(a.vag["roles"]["厨房"][0], "全件閲覧.注文");
    for (bad, want) in [
        ("権限 = [注文.見る]", "動詞"),
        ("権限 = [謎.読取]", "謎"),
        ("権限 = [動作.謎]", "謎"),
        ("権限 = [全件閲覧.謎]", "謎"),
        ("権限 = [注文]", "区切って"),
        ("色 = 赤", "キー"),
    ] {
        let e = errors_of(&with_src(&format!("{base}役割 厨房:\n    {bad}\n")));
        assert!(e.contains(want), "{bad} → {e}");
    }
    let e = errors_of(&with_src(&format!(
        "{base}役割 厨房:\n    権限 = [注文.読取]\n役割 厨房:\n    権限 = [注文.読取]\n"
    )));
    assert!(e.contains("既に定義"), "{e}");
}

#[test]
fn analyses_can_be_made_public_explicitly_even_over_owner_data() {
    let src = include_str!("../../../examples/izakaya/src/app.koto")
        .replace("    メニューは誰でも閲覧可\n", "    メニューは誰でも閲覧可\n    品名別の売上は誰でも閲覧可\n");
    let files = [(
        "analytics/sales_by_item.sql",
        include_str!("../../../examples/izakaya/analytics/sales_by_item.sql"),
    )];
    let mut p = project_from_sources("居酒屋", &[("src/app.koto", &src)], &["backend"]);
    for (k, v) in files {
        p.ext_files.insert(k.into(), v.into());
    }
    let a = analyze(&p, &none());
    assert!(a.ok(), "{:#?}", a.diags.items);
    let b = build_target(&p, &a, "backend").unwrap();
    let api: serde_json::Value = serde_json::from_str(&b.files.iter().find(|f| f.0 == "api.json").unwrap().1).unwrap();
    let e = api["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "analysis.品名別の売上")
        .unwrap();
    assert_eq!(e["access"]["public"], true);
    // 宣言しなければ全件閲覧の許可が必要
    let mut p2 = project_from_sources(
        "居酒屋",
        &[("src/app.koto", include_str!("../../../examples/izakaya/src/app.koto"))],
        &["backend"],
    );
    for (k, v) in files {
        p2.ext_files.insert(k.into(), v.into());
    }
    let a2 = analyze(&p2, &none());
    let b2 = build_target(&p2, &a2, "backend").unwrap();
    let api2: serde_json::Value = serde_json::from_str(&b2.files.iter().find(|f| f.0 == "api.json").unwrap().1).unwrap();
    let e2 = api2["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "analysis.品名別の売上")
        .unwrap();
    assert!(e2["access"]["capabilities"].as_array().unwrap().iter().any(|c| c == "全件閲覧.注文"));
}
