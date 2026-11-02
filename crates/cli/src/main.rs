//! `koto` CLI(技術仕様書 §14)。
//! 終了コード: 0=成功、2=構文/型、3=曖昧/未承認、4=権限、5=ロック、6=ターゲット非対応、7=内部障害/未実装。

use kotonoha_diag::{Code, Diagnostic, Diagnostics, Span};
use kotonoha_driver as drv;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "使い方: koto [-C <dir>] [--json] <command> [args]

コマンド:
  init [dir] [--name 名前]        新しいプロジェクトを作成
  fmt [--check]                   ソースのテキスト正規化(NFC・LF・行末空白)
  check                           字句/構文/型/知識/意図/効果を検査
  resolve [--adapter offline|fixture] [--fixture file]
                                  @柔軟 の未解決意図の解釈候補を作成(AI は使わない)
  approve <intent_id> <番号> --approver 名前 [--note 文] [--at 時刻]
                                  候補の解釈を承認して記録
  approve --write-lock            検査を通過した状態をロック
  graph [--draft] [--canonical]   Verified Application Graph を出力
  build --target web|backend|ios|android [--locked] [--out dir]
                                  ターゲット生成(未対応機能があれば TGT001 で停止)
  test                            (未実装)
  doctor                          実装状況と未決定事項を表示
  ast <file>                      [開発用] Surface AST を JSON で出力
";

struct Opts {
    dir: PathBuf,
    json: bool,
    args: Vec<String>,
}

fn parse_global(mut it: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut dir = PathBuf::from(".");
    let mut json = false;
    let mut args = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-C" | "--project" => dir = PathBuf::from(it.next().ok_or("-C にはディレクトリが必要です")?),
            "--json" => json = true,
            _ => args.push(a),
        }
    }
    Ok(Opts { dir, json, args })
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(i) = args.iter().position(|a| a == name) {
        args.remove(i);
        true
    } else {
        false
    }
}

fn take_value(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    if let Some(i) = args.iter().position(|a| a == name) {
        if i + 1 >= args.len() {
            return Err(format!("{name} には値が必要です"));
        }
        let v = args.remove(i + 1);
        args.remove(i);
        Ok(Some(v))
    } else {
        Ok(None)
    }
}

/// 診断を出力して終了コードを返す。
fn report(o: &Opts, diags: &Diagnostics, extra: serde_json::Value) -> i32 {
    let code = diags.exit_code();
    if o.json {
        let mut v = json!({"diagnostics": diags.items, "exit_code": code});
        if let (Some(m), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            for (k, x) in e {
                m.insert(k.clone(), x.clone());
            }
        }
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        for d in &diags.items {
            eprintln!("{d}");
        }
        let n_err = diags.errors().count();
        let n_warn = diags.items.len() - n_err;
        if n_err > 0 || n_warn > 0 {
            eprintln!("エラー {n_err} 件、警告 {n_warn} 件");
        }
    }
    code
}

fn single(code: Code, msg: impl Into<String>) -> Diagnostics {
    let mut d = Diagnostics::new();
    d.push(Diagnostic::error(code, Span::default(), msg));
    d
}

fn main() -> ExitCode {
    let o = match parse_global(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(7);
        }
    };
    let code = run(o);
    ExitCode::from(code as u8)
}

fn run(mut o: Opts) -> i32 {
    if o.args.is_empty() {
        eprint!("{USAGE}");
        return 7;
    }
    let cmd = o.args.remove(0);
    let r = match cmd.as_str() {
        "init" => cmd_init(&mut o),
        "fmt" => cmd_fmt(&mut o),
        "check" => cmd_check(&mut o),
        "resolve" => cmd_resolve(&mut o),
        "approve" => cmd_approve(&mut o),
        "graph" => cmd_graph(&mut o),
        "build" => cmd_build(&mut o),
        "test" => Ok(report(&o, &single(Code::INT001, "koto test は未実装です(テスト生成 FR-019 は P1)"), json!({}))),
        "doctor" => Ok(cmd_doctor(&o)),
        "ast" => cmd_ast(&mut o),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(0)
        }
        other => Err(format!("未知のコマンド `{other}` です\n{USAGE}")),
    };
    match r {
        Ok(c) => c,
        Err(e) => {
            let d = single(Code::INT001, e);
            report(&o, &d, json!({}))
        }
    }
}

fn no_extra(args: &[String]) -> Result<(), String> {
    if let Some(a) = args.first() {
        return Err(format!("不明な引数 `{a}` です"));
    }
    Ok(())
}

fn load(o: &Opts) -> Result<(drv::Project, kotonoha_lock::ApprovalsFile), i32> {
    let p = drv::load_project(&o.dir).map_err(|d| report(o, &d, json!({})))?;
    let ap = drv::load_approvals(&o.dir).map_err(|d| report(o, &d, json!({})))?;
    Ok((p, ap))
}

fn cmd_init(o: &mut Opts) -> Result<i32, String> {
    let name = take_value(&mut o.args, "--name")?;
    let dir = match o.args.first() {
        Some(d) => o.dir.join(d),
        None => o.dir.clone(),
    };
    if dir.join(drv::PROJECT_FILE).exists() {
        return Err(format!("{} は既に存在します", dir.join(drv::PROJECT_FILE).display()));
    }
    let name = name.unwrap_or_else(|| {
        dir.canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "アプリ".into())
    });
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let cfg = json!({"name": name, "sources": ["src/app.koto"], "targets": ["web", "backend"]});
    std::fs::write(dir.join(drv::PROJECT_FILE), serde_json::to_string_pretty(&cfg).unwrap() + "\n").map_err(|e| e.to_string())?;
    let app =
        format!("アプリ「{name}」\n\nデータ 商品:\n    ID: 識別子\n    名前: 文字列\n\n画面 商品一覧:\n    商品を一覧表示\n\n約束:\n    商品は誰でも閲覧可\n");
    let app_path = dir.join("src/app.koto");
    if !app_path.exists() {
        std::fs::write(&app_path, app).map_err(|e| e.to_string())?;
    }
    eprintln!("{} にプロジェクトを作成しました", dir.display());
    Ok(0)
}

fn cmd_fmt(o: &mut Opts) -> Result<i32, String> {
    let check = take_flag(&mut o.args, "--check");
    no_extra(&o.args)?;
    let p = match drv::load_project(&o.dir) {
        Ok(p) => p,
        Err(d) => return Ok(report(o, &d, json!({}))),
    };
    let mut changed = Vec::new();
    for (path, content) in p.files.values() {
        let n = drv::normalize_text(content);
        if &n != content {
            changed.push(path.clone());
            if !check {
                std::fs::write(o.dir.join(path), n).map_err(|e| e.to_string())?;
            }
        }
    }
    let mut d = Diagnostics::new();
    if check {
        for c in &changed {
            d.push(Diagnostic {
                file: c.clone(),
                ..Diagnostic::error(Code::SYN002, Span::default(), "正規化されていません(koto fmt で修正できます)")
            });
        }
    } else {
        for c in &changed {
            eprintln!("正規化しました: {c}");
        }
    }
    Ok(report(o, &d, json!({"changed": changed})))
}

fn cmd_check(o: &mut Opts) -> Result<i32, String> {
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    let code = report(
        o,
        &a.diags,
        json!({"status": a.vag["status"], "graph_hash": a.hashes.graph, "unresolved": a.intents.unresolved}),
    );
    if !o.json && code == 0 {
        eprintln!("検査に合格しました(graph_hash {})", &a.hashes.graph[..16]);
    }
    Ok(code)
}

fn cmd_resolve(o: &mut Opts) -> Result<i32, String> {
    use kotonoha_resolver::{validate_candidate, CandidatesFile, FixtureAdapter, OfflineAdapter, ResolverAdapter};
    let adapter_name = take_value(&mut o.args, "--adapter")?.unwrap_or_else(|| "offline".into());
    let fixture = take_value(&mut o.args, "--fixture")?;
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let adapter: Box<dyn ResolverAdapter> = match adapter_name.as_str() {
        "offline" => Box::new(OfflineAdapter),
        "fixture" => {
            let f = fixture.ok_or("--adapter fixture には --fixture <file> が必要です")?;
            let t = std::fs::read_to_string(o.dir.join(&f)).map_err(|e| format!("{f} を読めません: {e}"))?;
            Box::new(FixtureAdapter::from_json(&t)?)
        }
        other => return Err(format!("未知のアダプタ `{other}` です(offline / fixture。AI プロバイダは ADR-006 で未決定)")),
    };
    if adapter.sends_source() {
        return Err("ソースを外部送信するアダプタは明示同意の仕組み(NFR-010)が未実装のため使えません".into());
    }
    let a = drv::analyze(&p, &ap);
    let mut diags = Diagnostics::new();
    let mut cands = Vec::new();
    for u in a.unresolved() {
        if u.mode == kotonoha_intent::Mode::Strict {
            diags.push(Diagnostic::warning(
                Code::AMB001,
                u.span,
                format!("「{}」は @厳密 の宣言にあるため候補を作成しません", u.sentence),
            ));
            continue;
        }
        let raw = match adapter.propose(u) {
            Ok(r) => r,
            Err(e) => {
                diags.push(Diagnostic::warning(Code::AMB001, u.span, e));
                continue;
            }
        };
        match validate_candidate(&raw, u) {
            Err(errs) => diags.push(Diagnostic::error(
                Code::AMB001,
                u.span,
                format!("候補がスキーマに違反しています: {}", errs.join("; ")),
            )),
            Ok(mut c) => {
                for r in drv::filter_candidate(&p, &ap, &mut c, u) {
                    diags.push(Diagnostic::warning(Code::AMB001, u.span, r));
                }
                cands.push(c);
            }
        }
    }
    let file = CandidatesFile {
        candidates_version: "1".into(),
        adapter: adapter.name().into(),
        candidates: cands.clone(),
    };
    std::fs::write(o.dir.join(drv::CANDIDATES_FILE), serde_json::to_string_pretty(&file).unwrap() + "\n").map_err(|e| e.to_string())?;
    if !o.json {
        for c in &cands {
            eprintln!("{}:", c.intent_id);
            for (i, it) in c.interpretations.iter().enumerate() {
                eprintln!("  [{}] {} — {}", i + 1, it.canonical, it.rationale);
            }
            for q in &c.unresolved {
                eprintln!("  未確定: {q}");
            }
        }
        eprintln!("{} 件の候補を {} に書きました", cands.len(), drv::CANDIDATES_FILE);
    }
    for d in diags.items.iter_mut() {
        if let Some((path, _)) = p.files.get(&d.span.file) {
            d.file = path.clone();
        }
    }
    Ok(report(o, &diags, json!({"candidates": cands})))
}

fn cmd_approve(o: &mut Opts) -> Result<i32, String> {
    let write_lock = take_flag(&mut o.args, "--write-lock");
    let approver = take_value(&mut o.args, "--approver")?;
    let note = take_value(&mut o.args, "--note")?.unwrap_or_default();
    let at = take_value(&mut o.args, "--at")?;
    let (p, mut ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    if write_lock {
        no_extra(&o.args)?;
        let a = drv::analyze(&p, &ap);
        if !a.ok() {
            let c = report(o, &a.diags, json!({}));
            eprintln!("検査エラーがあるためロックできません");
            return Ok(c);
        }
        let lock = drv::compute_lock(&p, &a, &ap);
        std::fs::write(o.dir.join(drv::LOCK_FILE), lock.to_pretty_json()).map_err(|e| e.to_string())?;
        if !o.json {
            eprintln!("{} を書きました(graph_hash {})", drv::LOCK_FILE, &lock.graph_hash[..16]);
        }
        return Ok(report(o, &a.diags, json!({"lock": lock})));
    }
    if o.args.len() != 2 {
        return Err("koto approve <intent_id> <候補番号> --approver 名前".into());
    }
    let (intent_id, n) = (o.args[0].clone(), o.args[1].parse::<usize>().map_err(|_| "候補番号は 1 以上の整数です")?);
    let approver = approver.ok_or("--approver が必要です(承認者を記録します)")?;
    let text = std::fs::read_to_string(o.dir.join(drv::CANDIDATES_FILE))
        .map_err(|_| format!("{} がありません。先に koto resolve を実行してください", drv::CANDIDATES_FILE))?;
    let cf: kotonoha_resolver::CandidatesFile = serde_json::from_str(&text).map_err(|e| format!("{} の形式が不正です: {e}", drv::CANDIDATES_FILE))?;
    let cand = cf
        .candidates
        .iter()
        .find(|c| c.intent_id == intent_id)
        .ok_or(format!("`{intent_id}` の候補がありません"))?;
    if !cand.unresolved.is_empty() {
        return Ok(report(
            o,
            &single(
                Code::APP001,
                format!("候補に未確定事項が残っているため承認できません: {}", cand.unresolved.join(" / ")),
            ),
            json!({}),
        ));
    }
    let it = cand
        .interpretations
        .get(n.wrapping_sub(1))
        .ok_or(format!("候補番号 {n} はありません(1〜{})", cand.interpretations.len()))?;
    let a = drv::analyze(&p, &ap);
    let u = a
        .unresolved()
        .iter()
        .find(|u| u.intent_id == intent_id)
        .ok_or(format!("`{intent_id}` は現在未解決の意図ではありません(ソースが変わった可能性)"))?;
    if u.sentence_hash != cand.sentence_hash {
        return Ok(report(
            o,
            &single(Code::APP001, "候補作成後に文が変更されています。koto resolve をやり直してください"),
            json!({}),
        ));
    }
    let rec = kotonoha_lock::ApprovalRecord::new(
        &u.intent_id,
        &u.sentence,
        &u.sentence_hash,
        &it.canonical,
        &approver,
        &at.unwrap_or_else(drv::now_rfc3339),
        &note,
    );
    ap.upsert(rec.clone());
    let a2 = drv::analyze(&p, &ap);
    if a2.unresolved().iter().any(|x| x.intent_id == intent_id) || a2.diags.errors().any(|d| d.code == Code::APP001) {
        return Ok(report(
            o,
            &single(Code::APP001, format!("解釈「{}」は現在のソースで成立しません", it.canonical)),
            json!({}),
        ));
    }
    let v = serde_json::to_string_pretty(&ap).unwrap() + "\n";
    std::fs::write(o.dir.join(drv::APPROVALS_FILE), v).map_err(|e| e.to_string())?;
    if !o.json {
        eprintln!("承認しました: 「{}」→「{}」(承認者 {approver})", u.sentence, it.canonical);
        eprintln!("注意: 承認の真正性(署名)は ADR-007 未決定のため検証されません。本番ビルドは拒否されます");
    }
    Ok(report(o, &a2.diags, json!({"record": rec})))
}

fn cmd_graph(o: &mut Opts) -> Result<i32, String> {
    let draft = take_flag(&mut o.args, "--draft");
    let canonical = take_flag(&mut o.args, "--canonical");
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    if !a.ok() && !draft {
        let c = report(o, &a.diags, json!({}));
        eprintln!("検査エラーがあるため VAG を出力しません(--draft で参考出力)");
        return Ok(c);
    }
    let s = if canonical {
        kotonoha_graph::canonical_json(&a.vag)
    } else {
        serde_json::to_string_pretty(&a.vag).unwrap()
    };
    println!("{s}");
    if !o.json {
        for d in &a.diags.items {
            eprintln!("{d}");
        }
    }
    Ok(if a.ok() { 0 } else { a.diags.exit_code() })
}

fn cmd_build(o: &mut Opts) -> Result<i32, String> {
    let target = take_value(&mut o.args, "--target")?.ok_or("--target web|backend|ios|android が必要です")?;
    let locked = take_flag(&mut o.args, "--locked");
    let out = take_value(&mut o.args, "--out")?;
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    if !a.ok() {
        return Ok(report(o, &a.diags, json!({})));
    }
    let mut diags = a.diags.clone();
    if locked {
        let lock_text = std::fs::read_to_string(o.dir.join(drv::LOCK_FILE)).ok();
        let d = drv::verify_lock(&p, &a, &ap, lock_text.as_deref());
        if !d.is_empty() {
            diags.extend(d);
            return Ok(report(o, &diags, json!({})));
        }
    } else if !o.json {
        eprintln!("注意: --locked なしの開発ビルドです(本番ビルドは --locked 必須)");
    }
    match drv::build_target(&p, &a, &target) {
        Err(d) => {
            diags.extend(d);
            Ok(report(o, &diags, json!({})))
        }
        Ok(b) => {
            let out_dir = out.map(PathBuf::from).unwrap_or_else(|| o.dir.join("build").join(&target));
            for (rel, content) in &b.files {
                let path = out_dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, content).map_err(|e| e.to_string())?;
            }
            if !o.json {
                eprintln!("{} 個のファイルを {} に生成しました", b.files.len(), out_dir.display());
            }
            Ok(report(o, &diags, json!({"files": b.files.iter().map(|f| &f.0).collect::<Vec<_>>()})))
        }
    }
}

fn cmd_doctor(o: &Opts) -> i32 {
    let info = json!({
        "compiler_version": drv::COMPILER_VERSION,
        "implemented": ["lexer", "parser", "ast-printer", "type-check(strict layer)", "knowledge-registry(ranking)", "intent-patterns", "effects/capabilities", "vag", "lock", "approval-records(unsigned)", "resolver(offline/fixture)", "sql-lowering(ranking: sqlite/postgres)"],
        "targets": {
            "web": "未実装(ADR-004 UI フレームワーク未決定)",
            "backend": "知識クエリの SQL 低水準化のみ。HTTP/認可/CRUD 生成は未実装",
            "ios": "契約のみ(P1)",
            "android": "契約のみ(P1)"
        },
        "blockers": ["ADR-001 ライセンス", "ADR-002 認証プロバイダー", "ADR-004 UI 生成フレームワーク", "ADR-005 モバイル最低 OS", "ADR-006 AI プロバイダー", "ADR-007 承認署名基盤", "ADR-008 ビルド互換性"],
    });
    if o.json {
        println!("{}", serde_json::to_string_pretty(&info).unwrap());
    } else {
        println!("koto {}", drv::COMPILER_VERSION);
        println!(
            "実装済み: {}",
            info["implemented"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (k, v) in info["targets"].as_object().unwrap() {
            println!("ターゲット {k}: {}", v.as_str().unwrap());
        }
        println!(
            "未決定(ブロッカー): {}",
            info["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
    0
}

fn cmd_ast(o: &mut Opts) -> Result<i32, String> {
    let f = o.args.first().ok_or("koto ast <file>")?.clone();
    let path = if Path::new(&f).is_absolute() { PathBuf::from(&f) } else { o.dir.join(&f) };
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{f} を読めません: {e}"))?;
    let (ast, d) = kotonoha_parser_parse(&src);
    println!("{}", serde_json::to_string_pretty(&ast).unwrap());
    let mut ds = Diagnostics::new();
    for mut x in d {
        x.file = f.clone();
        ds.push(x);
    }
    Ok(report(o, &ds, json!({})))
}

fn kotonoha_parser_parse(src: &str) -> (serde_json::Value, Vec<Diagnostic>) {
    let (ast, d) = drv::parse_for_cli(src);
    (ast, d)
}
