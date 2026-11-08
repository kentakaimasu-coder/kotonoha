//! `koto` CLI(技術仕様書 §14)。
//! 終了コード: 0=成功、2=構文/型、3=曖昧/未承認、4=権限、5=ロック、6=ターゲット非対応、7=内部障害/未実装。

use kotonoha_diag::{Code, Diagnostic, Diagnostics, Span};
use kotonoha_driver as drv;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "使い方: koto [-C <dir>] [--json] <command> [args]

はじめての方へ:
  koto init 予約アプリ --template 予約受付   ひな形からプロジェクトを作る(ひな形: 予約受付 / 在庫管理 / 問い合わせ / 日報)
  cd 予約アプリ && koto dev                   検査・生成・初期データ投入・起動を 1 回で行い、ブラウザで開ける状態にする
  koto patterns                               このアプリで使える画面・約束の書き方を、自分のデータ名で一覧表示

コマンド:
  init [dir] [--name 名前] [--template ひな形]
                                  新しいプロジェクトを作成(--template だけ指定するとひな形の一覧を表示)
  dev [--port 3000] [--no-start]  開発用に生成して起動(seed.json があれば初期データも入れる。画面と API は同じポート)
  patterns                        画面・約束に書ける文の一覧(宣言した名前で具体的に表示)
  safety                          生成されるアプリに自動で入る安全対策と、API ごとの公開範囲を表示
  fmt [--check] [--zenkaku]       ソースのテキスト正規化(NFC・LF・行末空白)。--zenkaku で全角英数記号を半角に
  check                           字句/構文/型/知識/意図/効果を検査
  resolve [--adapter offline|fixture] [--fixture file]
                                  @柔軟 の未解決意図の解釈候補を作成(AI は使わない)
  approve <intent_id> <番号> --approver 名前 [--note 文] [--at 時刻] [--sign-key 鍵]
                                  候補の解釈を承認して記録
  approve --write-lock            検査を通過した状態をロック
  keygen --out <file>             承認署名用の Ed25519 鍵を作成(公開鍵と信頼鍵ファイルの雛形を表示)
  graph [--draft] [--canonical]   Verified Application Graph を出力
  build --target web|backend|ios|android [--locked [--trust 信頼鍵.json]] [--out dir]
                                  ターゲット生成(未対応機能があれば TGT001 で停止)
  test [--keep]                   backend を生成し、API 契約から導いた契約テストを実行(Node.js 22.18+ が必要)
  impact <名前> | --base <vag.json>
                                  変更の影響を受ける画面・動作・部品を列挙(名前はデータ / データ.フィールド / 知識 / 動作 / 画面)
  explain <コード>                 エラーコードの意味と直し方を表示(例: koto explain AMB001)
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
            if let Some(ex) = excerpt(&o.dir, d) {
                eprint!("{ex}");
            }
        }
        if diags.errors().next().is_some() {
            eprintln!("(エラーの詳しい説明: koto explain <コード>)");
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
        "dev" => cmd_dev(&mut o),
        "patterns" => cmd_patterns(&mut o),
        "safety" => cmd_safety(&mut o),
        "fmt" => cmd_fmt(&mut o),
        "check" => cmd_check(&mut o),
        "resolve" => cmd_resolve(&mut o),
        "approve" => cmd_approve(&mut o),
        "graph" => cmd_graph(&mut o),
        "build" => cmd_build(&mut o),
        "test" => cmd_test(&mut o),
        "impact" => cmd_impact(&mut o),
        "doctor" => Ok(cmd_doctor(&o)),
        "keygen" => cmd_keygen(&mut o),
        "explain" => match o.args.first().and_then(|c| Code::parse(c)) {
            Some(c) => {
                println!("{c}: {}", c.explain());
                Ok(0)
            }
            None => Err(format!(
                "koto explain <コード>(例: koto explain AMB001)。コード: {}",
                Code::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(" ")
            )),
        },
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

/// ひな形(名前, 説明, ソース, 初期データ)
const TEMPLATES: &[(&str, &str, &str, &str)] = &[
    (
        "予約受付",
        "枠(日時・定員)の一覧と予約、満席の判定、自分の予約、管理者の枠登録",
        include_str!("../templates/reservation.koto"),
        r#"{"枠": [{"ID": "s1", "名前": "午前の回", "開始日時": "2026-12-05T10:00:00+09:00", "定員": 5, "予約数": 0}, {"ID": "s2", "名前": "午後の回", "開始日時": "2026-12-05T14:00:00+09:00", "定員": 2, "予約数": 0}]}"#,
    ),
    (
        "在庫管理",
        "品目の一覧と入庫・出庫(在庫不足の判定)、入出庫の記録",
        include_str!("../templates/inventory.koto"),
        r#"{"品目": [{"ID": "i1", "名前": "コピー用紙", "保管場所": "倉庫A", "在庫数": 10}, {"ID": "i2", "名前": "トナー", "保管場所": "倉庫B", "在庫数": 2}]}"#,
    ),
    (
        "問い合わせ",
        "問い合わせフォーム、本人だけが見られる一覧、対応状況",
        include_str!("../templates/inquiry.koto"),
        "{}",
    ),
    (
        "日報",
        "日報の記入フォームと、本人だけが見られる一覧",
        include_str!("../templates/report.koto"),
        "{}",
    ),
];

fn cmd_init(o: &mut Opts) -> Result<i32, String> {
    let name = take_value(&mut o.args, "--name")?;
    let template = if o.args.iter().any(|a| a == "--template") && o.args.last().map(|a| a == "--template").unwrap_or(false) {
        o.args.pop();
        let mut s = String::from("ひな形の一覧(koto init <フォルダ> --template <名前>):\n");
        for (n, d, _, _) in TEMPLATES {
            s.push_str(&format!("  {n}  {d}\n"));
        }
        print!("{s}");
        return Ok(0);
    } else {
        take_value(&mut o.args, "--template")?
    };
    let dir = match o.args.first() {
        Some(d) => o.dir.join(d),
        None => o.dir.clone(),
    };
    if dir.join(drv::PROJECT_FILE).exists() {
        return Err(format!("{} は既に存在します", dir.join(drv::PROJECT_FILE).display()));
    }
    let tpl = match &template {
        None => None,
        Some(t) => Some(TEMPLATES.iter().find(|x| x.0 == t).ok_or_else(|| {
            format!(
                "ひな形 `{t}` はありません(使えるひな形: {})",
                TEMPLATES.iter().map(|x| x.0).collect::<Vec<_>>().join(" / ")
            )
        })?),
    };
    let name = name.unwrap_or_else(|| {
        dir.canonicalize()
            .ok()
            .or_else(|| Some(dir.clone()))
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "アプリ".into())
    });
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    let cfg = json!({"name": name, "sources": ["src/app.koto"], "targets": ["web", "backend"]});
    std::fs::write(dir.join(drv::PROJECT_FILE), serde_json::to_string_pretty(&cfg).unwrap() + "\n").map_err(|e| e.to_string())?;
    let (app, seed) = match tpl {
        Some((_, _, src, seed)) => (src.replace("{name}", &name), seed.to_string()),
        None => (
            format!(
                "アプリ「{name}」\n\nデータ 商品:\n    ID: 識別子\n    名前: 文字列\n\n画面 商品一覧:\n    商品を一覧表示\n\n約束:\n    商品は誰でも閲覧可\n"
            ),
            r#"{"商品": [{"ID": "p1", "名前": "見本の商品"}]}"#.to_string(),
        ),
    };
    let app_path = dir.join("src/app.koto");
    if !app_path.exists() {
        std::fs::write(&app_path, app).map_err(|e| e.to_string())?;
    }
    let seed_path = dir.join("seed.json");
    if !seed_path.exists() && seed != "{}" {
        let v: serde_json::Value = serde_json::from_str(&seed).expect("ひな形の初期データ");
        std::fs::write(&seed_path, serde_json::to_string_pretty(&v).unwrap() + "\n").map_err(|e| e.to_string())?;
    }
    std::fs::write(dir.join(".gitignore"), ".koto/\nbuild/\n").map_err(|e| e.to_string())?;
    eprintln!("{} にプロジェクトを作成しました", dir.display());
    eprintln!("次にやること:\n  1. src/app.koto を開いて、データ名や項目を自分の用途に書き換える\n  2. koto -C {} dev   (検査・生成・起動。表示された URL をブラウザで開く)\n  3. 書き方に迷ったら koto -C {} patterns", dir.display(), dir.display());
    Ok(0)
}

/// 生成された API 契約から、開発用ログインに渡す全権限を集める
fn all_caps(api: &serde_json::Value) -> Vec<String> {
    let mut caps = std::collections::BTreeSet::new();
    for e in api["endpoints"].as_array().into_iter().flatten() {
        for c in e["access"]["capabilities"].as_array().into_iter().flatten() {
            if let Some(c) = c.as_str() {
                caps.insert(c.to_string());
            }
        }
        if let Some(g) = e["access"]["grant"].as_str() {
            caps.insert(g.to_string());
        }
    }
    caps.into_iter().collect()
}

/// 自動で入る安全対策(K-26: 利用者から見えるようにする)
fn safety_text(api: &serde_json::Value) -> String {
    let mut s = String::from("自動で入っている安全対策:\n");
    for line in [
        "ログイン(JWT)の検証と、権限の無い操作の拒否(何も書かなければ拒否)",
        "「本人のものだけ閲覧可」と書いたデータは、本人の行だけを返す",
        "取引: の中の更新はまとめて行い、失敗したら元に戻す(同時の更新でも数が合う)",
        "入力の型・長さの検査(合わない入力は 400)",
        "画面の文字は自動でエスケープ(スクリプトの埋め込みを防ぐ)、CSP などのセキュリティヘッダ",
        "他サイトからの操作(CSRF)・ログインの拒否",
        "書き込みの回数制限、認証の失敗が続く接続元の一時遮断、操作の記録(監査ログ)",
        "応答に製品名やバージョンを出さない",
    ] {
        s.push_str(&format!("  ・{line}\n"));
    }
    s.push_str("\nAPI ごとの公開範囲:\n");
    for e in api["endpoints"].as_array().into_iter().flatten() {
        let a = &e["access"];
        let who = if a["public"].as_bool() == Some(true) {
            "誰でも".to_string()
        } else if let Some(o) = a["owner"].as_str() {
            format!("ログインした本人の{o}だけ")
        } else {
            let caps: Vec<&str> = a["capabilities"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
            let mut v = Vec::new();
            if !caps.is_empty() {
                v.push(format!("権限 {}", caps.join(" + ")));
            }
            if let Some(g) = a["grant"].as_str() {
                v.push(format!("許可 {g}"));
            }
            format!("ログイン + {}", v.join(" または "))
        };
        s.push_str(&format!(
            "  {} {}  … {who}\n",
            e["method"].as_str().unwrap_or(""),
            e["path"].as_str().unwrap_or("")
        ));
    }
    s
}

fn analyzed(o: &Opts) -> Result<(drv::Project, drv::Analysis), i32> {
    let (p, ap) = load(o)?;
    let a = drv::analyze(&p, &ap);
    if !a.ok() {
        return Err(report(o, &a.diags, json!({})));
    }
    Ok((p, a))
}

fn cmd_safety(o: &mut Opts) -> Result<i32, String> {
    no_extra(&o.args)?;
    let (p, a) = match analyzed(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let b = drv::build_target(&p, &a, "backend").map_err(|d| d.iter().map(|x| x.message_ja.clone()).collect::<Vec<_>>().join("\n"))?;
    let api: serde_json::Value = serde_json::from_str(&b.files.iter().find(|f| f.0 == "api.json").unwrap().1).unwrap();
    print!("{}", safety_text(&api));
    Ok(0)
}

fn cmd_patterns(o: &mut Opts) -> Result<i32, String> {
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    let screen = kotonoha_intent::example_sentences(&a.env, &a.registry, false);
    let contract = kotonoha_intent::example_sentences(&a.env, &a.registry, true);
    let mut s = String::from("画面(画面 名前: の下)に書ける文:\n");
    for x in &screen {
        s.push_str(&format!("    {x}\n"));
    }
    s.push_str("\n約束(約束: の下)に書ける文:\n");
    for x in &contract {
        s.push_str(&format!("    {x}\n"));
    }
    s.push_str("\n補足:\n  ・「…を押すと 動作(引数)」の引数には、画面の入力データ(例: 商品.ID)や数・文字列を書けます\n  ・「…を選ぶと 画面 へ」の遷移先の画面では、選んだデータを「データの項目と項目を表示」で表示できます\n  ・詳しい説明: docs/tutorial.md、docs/spec/intent_patterns.md\n");
    print!("{s}");
    Ok(0)
}

fn run_cmd(cmd: &mut std::process::Command, what: &str) -> Result<(), String> {
    let st = cmd
        .status()
        .map_err(|e| format!("{what} を実行できません: {e}(Node.js 22.18 以上と npm が必要です)"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{what} に失敗しました(終了コード {:?})", st.code()))
    }
}

/// `koto dev`: 検査 → backend と web を生成 → 初期データ → 起動(画面と API は同じポート)
fn cmd_dev(o: &mut Opts) -> Result<i32, String> {
    let port: u16 = take_value(&mut o.args, "--port")?
        .map(|p| p.parse().map_err(|_| "--port は数字です".to_string()))
        .transpose()?
        .unwrap_or(3000);
    let no_start = take_flag(&mut o.args, "--no-start");
    no_extra(&o.args)?;
    let (p, a) = match analyzed(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let dev = o.dir.join(".koto").join("dev");
    let mut api = serde_json::Value::Null;
    for t in ["backend", "web"] {
        let b = match drv::build_target(&p, &a, t) {
            Ok(b) => b,
            Err(d) => {
                let mut ds = a.diags.clone();
                ds.extend(d);
                return Ok(report(o, &ds, json!({})));
            }
        };
        let out = dev.join(t);
        let _ = std::fs::remove_dir_all(out.join("src"));
        for (rel, content) in &b.files {
            let path = out.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, content).map_err(|e| e.to_string())?;
            if rel == "api.json" {
                api = serde_json::from_str(content).unwrap_or_default();
            }
        }
    }
    let web = dev.join("web");
    let be = dev.join("backend");
    if !web.join("node_modules").exists() {
        eprintln!("初回だけ、画面の部品(React など)をダウンロードします。1 分ほどかかります…");
        run_cmd(
            std::process::Command::new("npm")
                .args(["install", "--no-audit", "--no-fund", "--silent"])
                .current_dir(&web),
            "npm install",
        )?;
    }
    run_cmd(
        std::process::Command::new("npm").args(["run", "build", "--silent"]).current_dir(&web),
        "画面のビルド",
    )?;
    // 開発用の秘密(この PC の .koto/dev だけに置く。ソースや生成物には入れない)
    let secret_path = dev.join("secret");
    let secret = match std::fs::read_to_string(&secret_path) {
        Ok(s) if s.trim().len() >= 64 => s.trim().to_string(),
        _ => {
            let out = std::process::Command::new("node")
                .args(["-e", "process.stdout.write(require('crypto').randomBytes(32).toString('hex'))"])
                .output()
                .map_err(|e| format!("node を実行できません: {e}"))?;
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            std::fs::write(&secret_path, &s).map_err(|e| e.to_string())?;
            s
        }
    };
    let db = dev.join("data.sqlite");
    let seed = o.dir.join("seed.json");
    if seed.exists() {
        run_cmd(
            std::process::Command::new("node")
                .args(["--no-warnings", "tools/seed.ts"])
                .arg(seed.canonicalize().map_err(|e| e.to_string())?)
                .env("KOTONOHA_DB", &db)
                .env("KOTONOHA_JWT_SECRET", &secret)
                .current_dir(&be),
            "初期データの投入(seed.json)",
        )?;
    }
    let caps = all_caps(&api);
    let entry = api["entry_screen"].as_str().unwrap_or("");
    eprintln!("\n準備ができました。");
    eprintln!("  画面:   http://127.0.0.1:{port}/   (最初の画面: {entry})");
    eprintln!("  ログイン: 画面右上の「ログイン」→「開発用: 名前を入れてすぐ入る」");
    eprintln!("  API:    http://127.0.0.1:{port}/api/…  (Authorization: Bearer で呼べます)");
    eprintln!("  データ: {}(消すと空からやり直せます)", db.display());
    eprintln!("  止めるとき: Ctrl+C\n");
    eprint!("{}", safety_text(&api));
    if no_start {
        return Ok(0);
    }
    let bport = port.checked_add(1).ok_or("--port が大きすぎます")?;
    let mut backend = std::process::Command::new("node")
        .args(["--no-warnings", "src/main.ts"])
        .env("PORT", bport.to_string())
        .env("KOTONOHA_DB", &db)
        .env("KOTONOHA_JWT_SECRET", &secret)
        .env("KOTONOHA_EXIT_WITH_PARENT", std::process::id().to_string())
        .current_dir(&be)
        .spawn()
        .map_err(|e| format!("backend を起動できません: {e}"))?;
    let mut webp = std::process::Command::new("node")
        .args(["--no-warnings", "src/server-main.ts"])
        .env("PORT", port.to_string())
        .env("KOTONOHA_BACKEND_URL", format!("http://127.0.0.1:{bport}"))
        .env("KOTONOHA_DEV_LOGIN", "1")
        .env("KOTONOHA_DEV_JWT_SECRET", &secret)
        .env("KOTONOHA_DEV_CAPS", caps.join(","))
        .env("KOTONOHA_EXIT_WITH_PARENT", std::process::id().to_string())
        .current_dir(&web)
        .spawn()
        .map_err(|e| format!("画面のサーバーを起動できません: {e}"))?;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let b = backend.try_wait().map_err(|e| e.to_string())?;
        let w = webp.try_wait().map_err(|e| e.to_string())?;
        if b.is_some() || w.is_some() {
            let _ = backend.kill();
            let _ = webp.kill();
            eprintln!("サーバーが止まりました(ポート {port} / {bport} が使用中でないか確認してください。--port で変えられます)");
            return Ok(1);
        }
    }
}

fn cmd_fmt(o: &mut Opts) -> Result<i32, String> {
    let check = take_flag(&mut o.args, "--check");
    let zenkaku = take_flag(&mut o.args, "--zenkaku");
    no_extra(&o.args)?;
    let p = match drv::load_project(&o.dir) {
        Ok(p) => p,
        Err(d) => return Ok(report(o, &d, json!({}))),
    };
    let mut changed = Vec::new();
    for (path, content) in p.files.values() {
        let mut n = drv::normalize_text(content);
        if zenkaku {
            n = drv::fix_zenkaku(&n);
        }
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
    let sign_key = match take_value(&mut o.args, "--sign-key")? {
        None => None,
        Some(f) => Some(kotonoha_lock::SigningKey::from_hex(
            &std::fs::read_to_string(&f).map_err(|e| format!("署名鍵 {f} を読めません: {e}"))?,
        )?),
    };
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
    let mut rec = rec;
    if let Some(k) = &sign_key {
        k.sign(&mut rec);
    }
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
    let trust_path = take_value(&mut o.args, "--trust")?.or_else(|| std::env::var("KOTONOHA_TRUST_FILE").ok());
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
        let trust = match &trust_path {
            None => None,
            Some(t) => {
                let text = std::fs::read_to_string(t).map_err(|e| format!("信頼鍵ファイル {t} を読めません: {e}"))?;
                Some(serde_json::from_str::<kotonoha_lock::TrustFile>(&text).map_err(|e| format!("信頼鍵ファイルの形式が不正です: {e}"))?)
            }
        };
        let d = drv::verify_lock_with(&p, &a, &ap, lock_text.as_deref(), trust.as_ref());
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
        "implemented": ["lexer", "parser", "ast-printer", "type-check(strict layer)", "knowledge-registry(ranking)", "intent-patterns", "effects/capabilities", "vag", "lock", "approval-records(ed25519 signing)", "resolver(offline/fixture)", "sql-lowering(ranking: sqlite/postgres)", "app-ir + api-contract", "functions + builtins", "external functions(typescript/python)", "analysis(read-only sql)", "extension components", "experiments(a/b)", "backend(node/ts/sqlite/postgres)", "web(react ssr)", "ios(swiftui, CI build)", "android(compose, CI build)"],
        "targets": {
            "web": kotonoha_codegen::verification_status(kotonoha_codegen::TargetKind::Web),
            "backend": kotonoha_codegen::verification_status(kotonoha_codegen::TargetKind::Backend),
            "ios": kotonoha_codegen::verification_status(kotonoha_codegen::TargetKind::Ios),
            "android": kotonoha_codegen::verification_status(kotonoha_codegen::TargetKind::Android)
        },
        "blockers": ["ADR-001 ライセンス", "ADR-002 認証プロバイダー(暫定: HS256 JWT)", "ADR-004 UI 生成フレームワーク(暫定: React SSR + esbuild)", "ADR-005 モバイル最低 OS", "ADR-006 AI プロバイダー", "ADR-007 承認署名基盤", "ADR-008 ビルド互換性"],
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

fn cmd_impact(o: &mut Opts) -> Result<i32, String> {
    let base = take_value(&mut o.args, "--base")?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    if !a.ok() {
        return Ok(report(o, &a.diags, json!({})));
    }
    let impacts = match (base, o.args.first()) {
        (Some(b), None) => {
            let t = std::fs::read_to_string(o.dir.join(&b)).map_err(|e| format!("{b} を読めません: {e}"))?;
            let v: serde_json::Value = serde_json::from_str(&t).map_err(|e| format!("{b} は VAG の JSON ではありません: {e}"))?;
            drv::impact_diff(&v, &a.vag)
        }
        (None, Some(name)) => drv::impact_of(&a.vag, name),
        _ => return Err("koto impact <名前> または koto impact --base <vag.json> のどちらかを指定してください".into()),
    };
    if o.json {
        println!("{}", serde_json::to_string_pretty(&json!({"impacts": impacts, "exit_code": 0})).unwrap());
    } else if impacts.is_empty() {
        println!("影響を受けるノードはありません");
    } else {
        for i in &impacts {
            println!("{}\t{}", i.node, i.reason);
        }
        eprintln!("{} 件のノードが影響を受けます", impacts.len());
    }
    Ok(0)
}

fn cmd_test(o: &mut Opts) -> Result<i32, String> {
    let keep = take_flag(&mut o.args, "--keep");
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    if !a.ok() {
        return Ok(report(o, &a.diags, json!({})));
    }
    let b = match drv::build_target(&p, &a, "backend") {
        Ok(b) => b,
        Err(d) => {
            let mut ds = a.diags.clone();
            ds.extend(d);
            return Ok(report(o, &ds, json!({})));
        }
    };
    let dir = std::env::temp_dir().join(format!("kotonoha-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, content) in &b.files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
    }
    let status = std::process::Command::new("node")
        .args(["--no-warnings", "--test", "tests/contract.test.ts", "tests/actions.todo.test.ts"])
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("node を実行できません(Node.js 22.18 以上が必要です): {e}"))?;
    if keep {
        eprintln!("生成物: {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    // テスト失敗は終了コード 1(ADR-0103 追補)
    Ok(if status.success() { 0 } else { 1 })
}

fn cmd_keygen(o: &mut Opts) -> Result<i32, String> {
    let out = take_value(&mut o.args, "--out")?.ok_or("koto keygen --out <秘密鍵ファイル>")?;
    let owner = take_value(&mut o.args, "--owner")?.unwrap_or_else(|| "承認者".into());
    no_extra(&o.args)?;
    let path = o.dir.join(&out);
    if path.exists() {
        return Err(format!("{} は既に存在します(上書きしません)", path.display()));
    }
    let k = kotonoha_lock::SigningKey::generate();
    write_secret(&path, &(k.to_hex() + "\n")).map_err(|e| e.to_string())?;
    let trust = serde_json::json!({"trusted_keys": [{"owner": owner, "pubkey": k.public_hex()}]});
    println!("{}", serde_json::to_string_pretty(&trust).unwrap());
    eprintln!("秘密鍵を {} に書きました(他人に渡さない・リポジトリに入れない)。", path.display());
    eprintln!("上の JSON が信頼鍵ファイルの雛形です。リポジトリの外(CI の秘密設定など)で管理し、koto build --locked --trust で渡してください。");
    Ok(0)
}

#[cfg(unix)]
fn write_secret(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(content.as_bytes())
}

#[cfg(not(unix))]
fn write_secret(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(content.as_bytes())
}

/// 診断の位置の行を抜き出し、`^` で位置を示す(NR-01)。
fn excerpt(dir: &Path, d: &Diagnostic) -> Option<String> {
    if d.file.is_empty() || d.span.line == 0 {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(&d.file)).ok()?;
    let line = text.lines().nth(d.span.line as usize - 1)?;
    let col = d.span.column.max(1) as usize - 1;
    let prefix: String = line.chars().take(col).collect();
    // 全角文字は端末で 2 桁の幅になるため、印の位置も幅で合わせる
    let width: usize = prefix.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum();
    let len = {
        let start = d.span.start as usize;
        let end = (d.span.end as usize).max(start);
        let seg = text.get(start..end).unwrap_or("");
        seg.chars()
            .take_while(|c| *c != '\n')
            .map(|c| if is_wide(c) { 2 } else { 1 })
            .sum::<usize>()
            .max(1)
    };
    let no = d.span.line.to_string();
    Some(format!(
        "{pad} |\n{no} | {line}\n{pad} | {sp}{marks}\n",
        pad = " ".repeat(no.len()),
        sp = " ".repeat(width),
        marks = "^".repeat(len)
    ))
}

fn is_wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6)
}
