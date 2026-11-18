//! `koto` CLI(技術仕様書 §14)。
//! 終了コード: 0=成功、2=構文/型、3=曖昧/未承認、4=権限、5=ロック、6=ターゲット非対応、7=内部障害/未実装。

use kotonoha_ast::Decl;
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
  studio [--port 4000]            画面で作る(データ・画面・動作・役割を画面で足す。日本語で指示。iPhone・Android の各機種での見た目を並べて確認)
  patterns                        画面・約束に書ける文の一覧(宣言した名前で具体的に表示)
  outline                         アプリの構成・検査の結果・書ける文を JSON で出す(Studio などの道具向け)
  publish [--out 公開用]          公開の準備: 検査・本番用の一式(backend と画面・設定のひな形・起動の手順・点検の結果)を作る
  rename <今の名前> <新しい名前>   データ・画面・動作・役割・列挙・項目の名前を、使っている所と初期データ(seed*.json)ごと変える
                                  (例: koto rename 枠 イベント / koto rename 枠.定員 上限)。誤りが増えるときは変えない
  safety                          生成されるアプリに自動で入る安全対策と、API ごとの公開範囲を表示
  token --sub 名前 [--role 役割 | --admin | --caps 権限,…]
                                  koto dev で動かしているアプリ用の動作確認トークンを作る(curl などで Authorization: Bearer に使う)
  fmt [--check] [--zenkaku]       ソースのテキスト正規化(NFC・LF・行末空白)。--zenkaku で全角英数記号を半角に
  check                           字句/構文/型/知識/意図/効果を検査
  resolve [--adapter offline|fixture|local|memory] [--fixture file] [--model 名前] [--endpoint URL]
                                  @柔軟 の未解決意図の解釈候補を作成。local は手元で動くオープンモデル
                                  (Ollama など OpenAI 互換。既定 http://127.0.0.1:11434/v1。外部送信はしない)、
                                  memory は過去の承認だけから候補を出す(AI を使わない)
  learn init|status|collect|export --out dir|eval [--model 名前] [--min 0.8]
                                  承認した解釈を学習記録として貯め、手元のモデルを育てる(~/.kotonoha/learn。
                                  KOTONOHA_LEARN_DIR で変更)。export は追加学習用の SFT / DPO データ、
                                  eval は過去の承認をどれだけ当てられるかの評価(新しいモデルを入れる前の確認)
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
        "outline" => cmd_outline(&mut o),
        "rename" => cmd_rename(&mut o),
        "studio" => cmd_studio(&mut o),
        "publish" => cmd_publish(&mut o),
        "safety" => cmd_safety(&mut o),
        "token" => cmd_token(&mut o),
        "fmt" => cmd_fmt(&mut o),
        "check" => cmd_check(&mut o),
        "resolve" => cmd_resolve(&mut o),
        "learn" => cmd_learn(&mut o),
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
    let code = match r {
        Ok(c) => c,
        Err(e) => {
            let d = single(Code::INT001, e);
            report(&o, &d, json!({}))
        }
    };
    usage_log(json!({"kind": "cli", "cmd": cmd, "code": code}));
    code
}

/// 操作の記録(評価用。KOTONOHA_USAGE_LOG にファイルの場所があるときだけ、1 行ずつ JSON で足す)。
/// 行動の指標(黒い画面のコマンドの回数・誤りの回数)を自己申告でなく記録から取るため(第 6 回の分析)
fn usage_log(mut v: serde_json::Value) {
    let Ok(path) = std::env::var("KOTONOHA_USAGE_LOG") else { return };
    if path.is_empty() {
        return;
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    v["t"] = json!(t);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{v}");
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
    // `koto init --help` で「--help」というフォルダを作らない(第 5 回評価)
    if o.args.iter().any(|a| a == "--help" || a == "-h") {
        println!("使い方: koto init <フォルダ> [--name 名前] [--template ひな形]\n  ひな形: 予約受付 / 在庫管理 / 問い合わせ / 日報(省略すると最小のひな形)");
        return Ok(0);
    }
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
    let cfg = json!({"name": name, "sources": ["src/app.koto"], "targets": ["web", "backend", "ios", "android"]});
    std::fs::write(dir.join(drv::PROJECT_FILE), serde_json::to_string_pretty(&cfg).unwrap() + "\n").map_err(|e| e.to_string())?;
    let (app, seed) = match tpl {
        Some((_, _, src, seed)) => (src.replace("{name}", &name), seed.to_string()),
        None => (
            format!(
                "アプリ「{name}」\n\n# 「商品」は見本です。Studio の上の欄に「「商品」を「本」に変える」のように書くと、自分の言葉に変わります\nデータ 商品:\n    ID: 識別子\n    名前: 文字列\n\n画面 商品一覧:\n    商品を一覧表示\n\n約束:\n    商品は誰でも閲覧可\n"
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
    std::fs::write(dir.join(".gitignore"), ".koto/\nbuild/\n公開用/\n").map_err(|e| e.to_string())?;
    // ダブルクリックで Studio を開くファイル(黒い画面でコマンドを打たずに済むように。第 6 回の分析)
    let mac = dir.join("Studio を開く.command");
    std::fs::write(&mac, "#!/bin/sh\n# ダブルクリックで KOTONOHA Studio を開きます(閉じるときはこの窓を閉じる)\ncd \"$(dirname \"$0\")\"\nKOTONOHA_OPEN_BROWSER=1 exec koto studio\n")
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&mac, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::write(dir.join("Studio を開く.bat"), "@echo off\r\nrem ダブルクリックで KOTONOHA Studio を開きます(閉じるときはこの窓を閉じる)\r\ncd /d \"%~dp0\"\r\nset KOTONOHA_OPEN_BROWSER=1\r\nkoto studio\r\n")
        .map_err(|e| e.to_string())?;
    eprintln!("{} にプロジェクトを作成しました", dir.display());
    eprintln!("次にやること:\n  1. src/app.koto を開いて、データ名や項目を自分の用途に書き換える\n  2. koto -C {} dev   (検査・生成・起動。表示された URL をブラウザで開く)\n  3. 書き方に迷ったら koto -C {} patterns\n  (名前を変える: koto -C {} rename 枠 イベント。画面で作る: koto -C {} studio)", dir.display(), dir.display(), dir.display(), dir.display());
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
        // 本人限定のデータは、開発用の管理者として入れば全員の行を見られるように
        if let Some(o) = e["access"]["owner"].as_str() {
            caps.insert(format!("全件閲覧.{o}"));
        }
    }
    caps.into_iter().collect()
}

/// 権限の名前をソースと同じ日本語で表示する(`予約.create` → `予約.作成`)。トークンにはどちらの書き方でも入れられる
fn ja_cap(c: &str) -> String {
    for (en, ja) in [("read", "読取"), ("create", "作成"), ("update", "更新"), ("delete", "削除")] {
        if let Some(d) = c.strip_suffix(&format!(".{en}")) {
            return format!("{d}.{ja}");
        }
    }
    c.to_string()
}

/// 役割を書かない人(開発用の「一般の利用者」・確かめるで役割を省いた人)の権限(ADR-0140)。
/// 役割に関わらない操作(必要な権限も動作の許可も、どの役割にも書いていない)の権限と許可だけ。全件閲覧は持たない。
/// runtime/backend/scenarios.ts と同じ決め方
fn general_caps(api: &serde_json::Value) -> Vec<String> {
    let role_caps: std::collections::BTreeSet<&str> = api["roles"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, cs)| cs.as_array().into_iter().flatten().filter_map(|c| c.as_str()))
        .collect();
    let mut out = std::collections::BTreeSet::new();
    for e in api["endpoints"].as_array().into_iter().flatten() {
        let a = &e["access"];
        let caps: Vec<&str> = a["capabilities"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
        let grant = a["grant"].as_str();
        let restricted = caps.iter().any(|c| role_caps.contains(c)) || grant.is_some_and(|g| role_caps.contains(g));
        for c in &caps {
            if !c.starts_with("全件閲覧.") && !role_caps.contains(c) {
                out.insert(c.to_string());
            }
        }
        if let Some(g) = grant.filter(|_| !restricted) {
            out.insert(g.to_string());
        }
    }
    out.into_iter().collect()
}

/// 公開前の点検: だれが何を見られる・できるかを、API の言葉を使わずに書く(第 9 回評価)
fn plain_access(api: &serde_json::Value) -> String {
    let mut s = String::new();
    let who = |a: &serde_json::Value| -> String {
        if a["public"].as_bool() == Some(true) {
            return "だれでも(ログインしなくても)".into();
        }
        if let Some(o) = a["owner"].as_str() {
            return format!("ログインした本人の分だけ(全件閲覧.{o} を持つ役割の人は全員分)");
        }
        if a["logged_in"].as_bool() == Some(true) {
            return "ログインした人みんな(役割に関わらず)".into();
        }
        let roles: Vec<String> = api["roles"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, caps)| {
                let cs: Vec<&str> = caps.as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
                let need: Vec<&str> = a["capabilities"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
                a["grant"].as_str().is_some_and(|g| cs.contains(&g)) || (!need.is_empty() && need.iter().all(|c| cs.contains(c)))
            })
            .map(|(r, _)| r.clone())
            .collect();
        if roles.is_empty() {
            "ログインした人".into()
        } else {
            format!("ログインした {} の人", roles.join("・"))
        }
    };
    for e in api["endpoints"].as_array().into_iter().flatten() {
        let path = e["path"].as_str().unwrap_or("");
        let method = e["method"].as_str().unwrap_or("");
        let a = &e["access"];
        if method == "GET" {
            if let Some(d) = path.strip_prefix("/api/data/").filter(|d| !d.contains('/') && !d.contains('{')) {
                s.push_str(&format!("- 「{d}」の一覧を見られる: {}\n", who(a)));
            } else if let Some(n) = path.strip_prefix("/api/analyses/") {
                s.push_str(&format!("- 集計「{n}」を見られる: {}\n", who(a)));
            }
        } else if let Some(n) = path.strip_prefix("/api/actions/") {
            s.push_str(&format!("- 「{n}」をできる: {}\n", who(a)));
        }
    }
    if s.is_empty() {
        s.push_str("- (見られるデータ・できる操作はありません)\n");
    }
    // 役割の書いてある人は、本番では認証基盤の役割で決まる。開発中の「一般の利用者(役割なし)」・確かめるで役割を省いた人は
    // それより広く試せるので、食い違って見える(第 12 回評価。ADR-0140)
    if api["roles"].as_object().is_some_and(|r| !r.is_empty()) {
        s.push_str("\n※「ログインした ○○ の人」は、その役割が付いた人です(管理者が「利用者の管理」の画面で付けます)。役割の付いていない人は、上で「ログインした人」「だれでも」と書いた所だけ使えます。\n  開発中の「役割なし(ログインしただけの人)」と、確かめるで役割を書かなかった人も同じです(本番と同じ)。\n");
    }
    s
}

/// 自動で入る安全対策(K-26: 利用者から見えるようにする)
fn safety_text(api: &serde_json::Value) -> String {
    let mut s = String::from("自動で入っている安全対策:\n");
    for line in [
        // 言葉はやさしく、専門の名前はかっこの中に(JWT・CSRF が分からなかった。第 9 回評価)
        "毎回、ログインしている本人か・その操作をしてよい人かを確かめ、許されていない操作は断る(JWT の検証。何も書かなければ断る)",
        "「本人のものだけ閲覧可」と書いたデータは、その人の分だけを見せる(全件閲覧.データ名 を持つ役割の人だけは全員分)",
        "一度に行う変更はまとめて行い、途中で失敗したら全部元に戻す(同時に操作されても数が合う)",
        "入力の形・長さを確かめ、合わない入力は断る",
        "名前などに仕込まれたプログラムが画面で動かないように、文字として表示する(エスケープ・CSP)",
        "ほかのサイトから、このアプリを勝手に操作させない(CSRF 対策)",
        "短い時間に大量の操作をさせない・ログインの失敗が続く相手を一時的に止める・だれが何をしたかを記録する",
        "使っている部品の名前や版を外に出さない",
    ] {
        s.push_str(&format!("  ・{line}\n"));
    }
    if let Some(rs) = api["roles"].as_object().filter(|m| !m.is_empty()) {
        s.push_str("\n役割(ソースの `役割` 宣言。トークンの roles に役割名を入れると、その権限を持つ):\n");
        for (r, caps) in rs {
            let cs: Vec<String> = caps.as_array().into_iter().flatten().filter_map(|c| c.as_str()).map(ja_cap).collect();
            s.push_str(&format!("  {r}: {}\n", cs.join(", ")));
        }
        // 役割ごとに実行できる動作(意図しない役割が実行できないかを見るため。ADR-0132)
        s.push_str("\n役割ごとに実行できる動作(この役割の人が押せる操作):\n");
        for (r, caps) in rs {
            let cs: Vec<&str> = caps.as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
            let can: Vec<&str> = api["endpoints"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["method"] == "POST" && e["path"].as_str().is_some_and(|p| p.starts_with("/api/actions/")))
                .filter(|e| {
                    let a = &e["access"];
                    let need: Vec<&str> = a["capabilities"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
                    a["grant"].as_str().is_some_and(|g| cs.contains(&g)) || (!need.is_empty() && need.iter().all(|c| cs.contains(c)))
                })
                .filter_map(|e| e["path"].as_str().and_then(|p| p.strip_prefix("/api/actions/")))
                .collect();
            s.push_str(&format!("  {r}: {}\n", if can.is_empty() { "なし".to_string() } else { can.join(", ") }));
        }
        s.push_str("  (ある動作を特定の役割だけにするには、その役割の権限に `動作.動作名` を書く。書いた役割だけが実行できる)\n");
    }
    s.push_str("\nAPI ごとの公開範囲:\n");
    for e in api["endpoints"].as_array().into_iter().flatten() {
        let a = &e["access"];
        let who = if a["public"].as_bool() == Some(true) {
            "誰でも".to_string()
        } else if let Some(o) = a["owner"].as_str() {
            format!("ログインした本人の{o}だけ(許可 全件閲覧.{o} を持つ人は全員分)")
        } else {
            let caps: Vec<&str> = a["capabilities"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
            let mut v = Vec::new();
            if !caps.is_empty() {
                v.push(format!("権限 {}", caps.iter().map(|c| ja_cap(c)).collect::<Vec<_>>().join(" + ")));
            }
            if let Some(g) = a["grant"].as_str() {
                v.push(format!("許可 {g}"));
            }
            if v.is_empty() {
                "ログインした人".to_string()
            } else {
                format!("ログイン + {}", v.join(" または "))
            }
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
    let b = match drv::build_target(&p, &a, "backend") {
        Ok(b) => b,
        Err(d) => {
            let mut ds = a.diags.clone();
            ds.extend(d);
            return Ok(report(o, &ds, json!({})));
        }
    };
    let api: serde_json::Value = serde_json::from_str(&b.files.iter().find(|f| f.0 == "api.json").unwrap().1).unwrap();
    print!("{}", safety_text(&api));
    Ok(0)
}

/// `koto outline`: アプリの構成(宣言とその行)・検査の結果・書ける文を JSON で出す(Studio が使う。ADR-0135)
fn cmd_outline(o: &mut Opts) -> Result<i32, String> {
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    let mut decls = Vec::new();
    for f in &a.asts {
        let Some((path, content)) = p.files.get(&f.file) else { continue };
        let lines: Vec<&str> = content.lines().collect();
        let mut starts: Vec<usize> = f.items.iter().map(|it| it.decl.span().line as usize).collect();
        starts.push(lines.len() + 1);
        for (i, it) in f.items.iter().enumerate() {
            let line = starts[i];
            // 宣言の終わり: 次の宣言の前の、空行・注釈行でない最後の行
            let mut end = starts[i + 1].saturating_sub(1).max(line);
            while end > line && lines.get(end - 1).is_none_or(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) {
                end -= 1;
            }
            let raw = |xs: &Vec<kotonoha_ast::RawText>| xs.iter().map(|l| json!({"text": l.text, "line": l.span.line})).collect::<Vec<_>>();
            let v = match &it.decl {
                Decl::App(d) => json!({"kind": "app", "name": d.name}),
                Decl::Enum(d) => json!({"kind": "enum", "name": d.name.name, "values": d.variants.iter().map(|v| v.name.clone()).collect::<Vec<_>>()}),
                Decl::Data(d) => {
                    json!({"kind": "data", "name": d.name.name,
                        "fields": d.fields.iter().map(|f| json!({"name": f.name.name, "type": kotonoha_parser::printer::type_str(&f.ty), "line": f.span.line})).collect::<Vec<_>>(),
                        "computed": d.computed.iter().map(|c| json!({"name": c.name.name, "expr": kotonoha_parser::printer::expr_str(&c.expr), "line": c.span.line})).collect::<Vec<_>>()})
                }
                Decl::Action(d) => {
                    json!({"kind": "action", "name": d.name.name, "params": d.params.iter().map(|p| json!({"name": p.name.name, "type": kotonoha_parser::printer::type_str(&p.ty)})).collect::<Vec<_>>()})
                }
                Decl::Screen(d) => json!({"kind": "screen", "name": d.name.name, "lines": raw(&d.lines)}),
                Decl::Scenario(d) => json!({"kind": "scenario", "name": d.name.name, "lines": raw(&d.lines)}),
                Decl::Contract(d) => json!({"kind": "rules", "name": "約束", "lines": raw(&d.lines)}),
                Decl::Role(d) => {
                    json!({"kind": "role", "name": d.name.name, "caps": a.env.roles.get(&d.name.name).map(|cs| cs.iter().map(|c| ja_cap(c)).collect::<Vec<_>>()).unwrap_or_default()})
                }
                Decl::External(d) if d.kind == kotonoha_ast::ExternalKind::Analysis => json!({"kind": "analysis", "name": d.name.name,
                    "columns": d.ret.as_ref().map(kotonoha_parser::printer::type_str).unwrap_or_default()}),
                Decl::External(d) => json!({"kind": "external", "name": d.name.name}),
                Decl::Function(d) => json!({"kind": "function", "name": d.name.name}),
                Decl::Adapter(d) => json!({"kind": "adapter", "name": d.name.name}),
                Decl::Experiment(d) => json!({"kind": "experiment", "name": d.name.name}),
                Decl::Knowledge(_) => json!({"kind": "knowledge"}),
                Decl::TypeAlias(_) => json!({"kind": "alias"}),
            };
            let mut v = v;
            v["file"] = json!(path);
            v["line"] = json!(line);
            v["end"] = json!(end);
            decls.push(v);
        }
    }
    let out = json!({
        "app": a.env.app_name,
        "files": p.files.values().map(|(path, _)| path.clone()).collect::<Vec<_>>(),
        "ok": a.ok(),
        "decls": decls,
        "diagnostics": a.diags.items,
        "patterns": {
            "screen": kotonoha_intent::example_sentences(&a.env, &a.registry, false),
            "rules": kotonoha_intent::example_sentences(&a.env, &a.registry, true),
        },
        "types": ["文字列", "整数", "真偽", "日付", "日時", "識別子", "金額", "メール", "URL"],
        "enums": a.env.enums,
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    Ok(0)
}

/// 名前の置き換え。注釈(# …)の行は変えない。
/// - 宣言の名前(field = false): 部分一致で変える(枠ID・枠一覧・枠を追加 も)。ただし直後がひらがな(助詞を除く)なら変えない
///   (「問い合わせる」が「お迎え連絡る」になった。第 6 回評価)
/// - 項目の名前(field = true): 前後の助詞・記号で区切りを見る。文字列("…")の中と、列挙の値の行は変えない
///   (列挙の値「内容が空」だけが変わり、失敗("内容が空") と合わなくなった。第 6 回評価)
fn rename_text(text: &str, from: &str, to: &str, field: bool) -> (String, usize) {
    const BEFORE: &str = " \t.(,、「のとはが[<=";
    const AFTER: &str = " \t:：)],、.」をがのはでにと順ご+-*/=<>!";
    const PARTICLES: &str = "をのはがにでとへかもや";
    let is_hira = |c: char| ('ぁ'..='ゖ').contains(&c);
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    let mut in_enum = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !line.starts_with([' ', '\t']) && !t.is_empty() {
            in_enum = t.starts_with("列挙 ");
        }
        if t.starts_with('#') || (field && in_enum && line.starts_with([' ', '\t'])) {
            out.push_str(line);
            continue;
        }
        let cs: Vec<char> = line.chars().collect();
        let fs: Vec<char> = from.chars().collect();
        let mut i = 0;
        let mut quoted = false;
        while i < cs.len() {
            if cs[i] == '"' {
                quoted = !quoted;
            }
            if cs[i..].starts_with(&fs) && !(field && quoted) {
                let before = if i == 0 { None } else { Some(cs[i - 1]) };
                let after = cs.get(i + fs.len()).copied();
                let ok = if field {
                    before.is_none_or(|c| BEFORE.contains(c)) && after.is_none_or(|c| c == '\n' || c == '\r' || AFTER.contains(c))
                } else {
                    after.is_none_or(|c| !is_hira(c) || PARTICLES.contains(c))
                };
                if ok {
                    out.push_str(to);
                    n += 1;
                    i += fs.len();
                    continue;
                }
            }
            out.push(cs[i]);
            i += 1;
        }
    }
    (out, n)
}

/// 項目の名前を変える。ほかのデータの同じ名前の項目(「名前」など)は変えない(第 6 回評価: 全部変わって誤りになり、戻された)。
/// 変える所: そのデータの宣言の中、動作の中で `v = データを取得(…)` とした v の `v.項目`、`データを登録(項目: …)` の項目名、
/// そのデータの名前を含む画面・約束・確かめるの文
fn rename_field_text(text: &str, data: &str, from: &str, to: &str) -> (String, usize) {
    // そのデータを登録する動作で、項目と同じ名前の引数(フォームの欄)も変える(第 7 回評価)
    let mut full: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    {
        let mut cur: Option<(String, bool, bool)> = None; // (動作名, 引数にある, 登録している)
        let mut flush = |c: &mut Option<(String, bool, bool)>| {
            if let Some((n, p, r)) = c.take() {
                if p && r {
                    full.insert(n);
                }
            }
        };
        for line in text.lines() {
            let t = line.trim_start();
            if !line.starts_with([' ', '\t']) && !t.is_empty() && !t.starts_with('#') {
                flush(&mut cur);
                if let Some(rest) = t.strip_prefix("動作 ") {
                    let name = rest.split(['(', ':', ' ']).next().unwrap_or("").to_string();
                    let params = rest.split_once('(').map(|(_, x)| x.split(')').next().unwrap_or("")).unwrap_or("");
                    let has = params.split(',').any(|p| p.split(':').next().map(str::trim) == Some(from));
                    cur = Some((name, has, false));
                }
            } else if let Some(c) = cur.as_mut() {
                if line.contains(&format!("{data}を登録(")) {
                    c.2 = true;
                }
            }
        }
        flush(&mut cur);
    }
    let mut out = String::with_capacity(text.len());
    let mut n = 0;
    let mut kind = String::new();
    let mut block = String::new();
    let mut vars: Vec<String> = Vec::new();
    let word = |l: &str| rename_text(l, from, to, true);
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !line.starts_with([' ', '\t']) && !t.is_empty() && !t.starts_with('#') {
            let mut it = t.splitn(2, ' ');
            kind = it.next().unwrap_or("").to_string();
            block = it.next().unwrap_or("").split([':', '(', ' ']).next().unwrap_or("").to_string();
            vars.clear();
        }
        if t.starts_with('#') || t.is_empty() {
            out.push_str(line);
            continue;
        }
        let new = match kind.as_str() {
            "データ" if block == data => word(line),
            "動作" if full.contains(&block) => word(line),
            "データ" | "列挙" | "役割" => (line.to_string(), 0),
            "動作" | "関数" if line.starts_with([' ', '\t']) => {
                if let Some((v, rhs)) = t.split_once('=') {
                    if rhs.trim_start().starts_with(&format!("{data}を取得(")) {
                        vars.push(v.trim().to_string());
                    }
                }
                let mut l = line.to_string();
                let mut k = 0;
                for v in &vars {
                    let (x, c) = rename_text(&l, &format!("{v}.{from}"), &format!("{v}.{to}"), true);
                    l = x;
                    k += c;
                }
                if l.contains(&format!("{data}を登録(")) {
                    for sep in ["(", ", ", ","] {
                        for colon in [":", " :"] {
                            let pat = format!("{sep}{from}{colon}");
                            k += l.matches(&pat).count();
                            l = l.replace(&pat, &format!("{sep}{to}{colon}"));
                        }
                    }
                }
                (l, k)
            }
            "動作" | "関数" => (line.to_string(), 0),
            _ if line.contains(data) => word(line),
            _ => (line.to_string(), 0),
        };
        out.push_str(&new.0);
        n += new.1;
    }
    (out, n)
}

/// JSON の文字列のうち、キー(直後が `:`)だけを f で置き換える(値や並び順・書式は変えない)
fn rename_json_keys(text: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let cs: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '"' {
            out.push(cs[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < cs.len() && cs[j] != '"' {
            j += if cs[j] == '\\' { 2 } else { 1 };
        }
        let s: String = cs[i + 1..j.min(cs.len())].iter().collect();
        let mut k = j + 1;
        while k < cs.len() && cs[k].is_whitespace() {
            k += 1;
        }
        let is_key = cs.get(k) == Some(&':');
        match (is_key, if is_key && !s.contains('\\') { f(&s) } else { None }) {
            (true, Some(n)) => out.push_str(&format!("\"{n}\"")),
            _ => out.extend(&cs[i..(j + 1).min(cs.len())]),
        }
        i = j + 1;
    }
    out
}

/// `koto rename 枠 イベント` / `koto rename 枠.定員 上限`: 名前を、使っている所と初期データごと変える(第 5 回評価:
/// ひな形の「枠」を自分の言葉に置き換える所を探すのが大変だった)。変えた後に検査し、誤りが増えるなら元に戻す
fn cmd_rename(o: &mut Opts) -> Result<i32, String> {
    if o.args.len() != 2 {
        return Err("使い方: koto rename <今の名前> <新しい名前>(項目は データ.項目。例: koto rename 枠 イベント / koto rename 枠.定員 上限)".into());
    }
    let (from, to) = (o.args[0].clone(), o.args[1].clone());
    let bad = |c: char| c.is_whitespace() || ":：()（）「」\"'`,、。=<>[]{}#.-+*/\\".contains(c);
    if to.is_empty() || to.chars().any(bad) || to.starts_with(|c: char| c.is_ascii_digit() || ('０'..='９').contains(&c)) {
        return Err(format!("新しい名前「{to}」は使えません(空白・記号を含まず、数字で始まらない名前にします)"));
    }
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    let errs_before = a.diags.errors().count();
    let decl_names: Vec<String> = a
        .asts
        .iter()
        .flat_map(|f| f.items.iter())
        .filter_map(|it| match &it.decl {
            Decl::Data(d) => Some(d.name.name.clone()),
            Decl::Enum(d) => Some(d.name.name.clone()),
            Decl::Action(d) => Some(d.name.name.clone()),
            Decl::Screen(d) => Some(d.name.name.clone()),
            Decl::Role(d) => Some(d.name.name.clone()),
            Decl::Scenario(d) => Some(d.name.name.clone()),
            Decl::Function(d) => Some(d.name.name.clone()),
            _ => None,
        })
        .collect();
    if decl_names.contains(&to) {
        return Err(format!("「{to}」は既にあります"));
    }
    // 何の名前か: 宣言 → 「データ.項目」→ どれか 1 つのデータの項目
    let (field_of, name): (Option<String>, String) = if decl_names.contains(&from) {
        (None, from.clone())
    } else if let Some((dn, f)) = from.split_once('.') {
        match a.env.data.get(dn) {
            Some(d) if d.field(f).is_some() => (Some(dn.to_string()), f.to_string()),
            Some(_) => return Err(format!("データ「{dn}」に項目「{f}」はありません")),
            None => return Err(format!("データ「{dn}」はありません")),
        }
    } else {
        let owners: Vec<&String> = a.env.data.iter().filter(|(_, d)| d.field(&from).is_some()).map(|(n, _)| n).collect();
        match owners.as_slice() {
            [] => {
                // 似た名前(前に変えた名前の残りなど)を示す
                let near: Vec<&String> = decl_names
                    .iter()
                    .filter(|n| {
                        let common = from.chars().filter(|c| n.contains(*c)).count();
                        common * 2 >= from.chars().count().max(1)
                    })
                    .take(5)
                    .collect();
                return Err(format!(
                    "「{from}」という名前のデータ・画面・動作・役割・列挙・項目はありません{}",
                    if near.is_empty() {
                        String::new()
                    } else {
                        format!("(似た名前: {})", near.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("、"))
                    }
                ));
            }
            [d] => (Some((*d).clone()), from.clone()),
            ds => {
                return Err(format!(
                    "項目「{from}」は {} にあります。koto rename {}.{from} {to} のように書きます",
                    ds.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("・"),
                    ds[0]
                ))
            }
        }
    };
    let root = &p.root;
    let mut saved: Vec<(PathBuf, String)> = Vec::new();
    let mut count = 0usize;
    for (rel, content) in p.files.values() {
        let (new, n) = match &field_of {
            Some(d) => rename_field_text(content, d, &name, &to),
            None => rename_text(content, &name, &to, false),
        };
        if n > 0 {
            let path = root.join(rel);
            saved.push((path.clone(), content.clone()));
            std::fs::write(&path, new).map_err(|e| format!("{rel} を書けません: {e}"))?;
            count += n;
        }
    }
    // 初期データ(seed*.json)のキー
    let mut seeds = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if fname.starts_with("seed") && fname.ends_with(".json") {
                seeds.push(e.path());
            }
        }
    }
    let field_unique = field_of.is_some() && a.env.data.values().filter(|d| d.field(&name).is_some()).count() == 1;
    for sp in seeds {
        let Ok(t) = std::fs::read_to_string(&sp) else { continue };
        // ソースと同じ置き換えを、JSON のキーにだけ行う(データ名は「予約数」のような項目名にも含まれるので部分一致で)
        let u = if field_of.is_none() {
            rename_json_keys(&t, |k| k.contains(name.as_str()).then(|| k.replace(name.as_str(), &to)))
        } else if field_unique {
            rename_json_keys(&t, |k| (k == name).then(|| to.clone()))
        } else {
            t.clone()
        };
        if u != t {
            saved.push((sp.clone(), t));
            std::fs::write(&sp, u).map_err(|e| format!("{} を書けません: {e}", sp.display()))?;
        }
    }
    // 検査し直して、誤りが増えたら元に戻す
    let after = drv::load_project(&o.dir).ok().map(|p2| drv::analyze(&p2, &ap));
    let errs_after = after.as_ref().map(|x| x.diags.errors().count()).unwrap_or(usize::MAX);
    if errs_after > errs_before {
        for (path, text) in &saved {
            let _ = std::fs::write(path, text);
        }
        if let Some(x) = &after {
            for d in x.diags.errors().take(3) {
                eprintln!("{d}");
            }
        }
        eprintln!("名前を変えると誤りが増えるので、変えずに元に戻しました");
        return Ok(1);
    }
    let what = match &field_of {
        Some(d) => format!("「{d}」の項目「{name}」"),
        None => format!("「{name}」"),
    };
    println!("{what}を「{to}」に変えました({count} か所。初期データも含めて {} ファイル)", saved.len());
    // 一緒に変わったほかの名前(枠ID・枠を追加・枠一覧 など)も示す(次に古い名前で探して迷った。第 6 回評価)
    if field_of.is_none() {
        let also: Vec<String> = decl_names
            .iter()
            .filter(|n| **n != name)
            .filter_map(|n| {
                let (r, _) = rename_text(n, &name, &to, false);
                (r != *n).then(|| format!("{n} → {r}"))
            })
            .collect();
        if !also.is_empty() {
            println!("一緒に変わった名前: {}", also.join("、"));
        }
    }
    Ok(0)
}

/// `koto publish`: 公開の準備(第 6 回の分析: 「作ったアプリを使ってもらう所まで自分でできない」)。
/// 検査に合格していれば、本番用の一式を作る: backend と画面(生成物)、設定のひな形(.env。秘密は新しく作る)、
/// 起動の手順(start.sh / start.bat / Dockerfile)、点検の結果(自動で入る安全対策・公開範囲・警告)。
/// 置き場所(サーバー・独自の URL)は外部のサービスとの契約が要るため、手順として書く。ログインはアプリの中のログイン(ADR-0142)
fn cmd_publish(o: &mut Opts) -> Result<i32, String> {
    let out_name = take_value(&mut o.args, "--out")?.unwrap_or_else(|| "公開用".into());
    no_extra(&o.args)?;
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let a = drv::analyze(&p, &ap);
    if !a.ok() {
        report(o, &a.diags, json!({}));
        return Err("検査に合格していないので、公開の準備はできません。上の誤りを直してください".into());
    }
    let out = p.root.join(&out_name);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    for t in ["backend", "web"] {
        let st = std::process::Command::new(&exe)
            .args(["-C"])
            .arg(&p.root)
            .args(["build", "--target", t, "--out"])
            .arg(out.join(t))
            .env("KOTONOHA_USAGE_LOG", "")
            .status()
            .map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(format!("{t} の生成に失敗しました"));
        }
    }
    // 設定のひな形: 既にあれば上書きしない(秘密を変えるとログイン中の人が締め出される)
    let env_path = out.join(".env");
    if !env_path.exists() {
        let secret = std::process::Command::new("node")
            .args(["-e", "process.stdout.write(require('crypto').randomBytes(48).toString('hex'))"])
            .output()
            .map_err(|e| format!("node を実行できません: {e}"))?;
        let secret = String::from_utf8_lossy(&secret.stdout).to_string();
        if secret.len() < 64 {
            return Err("秘密の値を作れませんでした".into());
        }
        let env = format!(
            "# 本番の設定(このファイルは人に見せない・Git に入れない)\nKOTONOHA_ENV=production\nKOTONOHA_JWT_SECRET={secret}\nKOTONOHA_DB=./data/data.sqlite\nPORT=8080\n# HTTPS で公開する(置き場所のサービスが HTTPS にする)。開発用のログインは使わない\nKOTONOHA_COOKIE_SECURE=1\nKOTONOHA_DEV_LOGIN=0\n# ログイン: アプリの中で利用者を登録する(外部のサービスは要らない。ADR-0142)。利用者の一覧は data/users.json に入る\nKOTONOHA_LOCAL_LOGIN=1\nKOTONOHA_USERS_FILE=./data/users.json\n# 招待のリンクに使う、このアプリの URL(例: https://yoyaku.example.jp)\nKOTONOHA_PUBLIC_URL=\n# 会社・学校のログインの仕組み(OIDC: Google Workspace・Microsoft Entra・Auth0 など)を使うときだけ、下に値を入れる\nKOTONOHA_OIDC_ISSUER=\nKOTONOHA_OIDC_CLIENT_ID=\nKOTONOHA_OIDC_CLIENT_SECRET=\nKOTONOHA_OIDC_REDIRECT_URI=https://あなたのドメイン/login/callback\n"
        );
        std::fs::write(&env_path, env).map_err(|e| e.to_string())?;
    }
    std::fs::write(out.join(".gitignore"), ".env\ndata/\nnode_modules/\n").map_err(|e| e.to_string())?;
    let start_sh = "#!/bin/sh\n# 本番の起動(Node.js 22.18 以上)。.env の値を読み込んで、画面(PORT)と API(PORT+1)を動かす\nset -e\ncd \"$(dirname \"$0\")\"\n# .env があれば読む(コンテナでは置き場所のサービスの「環境変数」に入れる)\nif [ -f ./.env ]; then set -a; . ./.env; set +a; fi\nif [ -z \"$KOTONOHA_OIDC_ISSUER\" ] && [ \"$KOTONOHA_LOCAL_LOGIN\" != \"1\" ]; then echo \"ログインの仕組みがまだです(.env の KOTONOHA_LOCAL_LOGIN=1 か KOTONOHA_OIDC_…)。公開の手順.md の 1 を見てください\" >&2; exit 1; fi\nmkdir -p data\nif [ -n \"$KOTONOHA_OIDC_ISSUER\" ]; then unset KOTONOHA_LOCAL_LOGIN; else unset KOTONOHA_OIDC_ISSUER KOTONOHA_OIDC_CLIENT_ID KOTONOHA_OIDC_CLIENT_SECRET KOTONOHA_OIDC_REDIRECT_URI; fi\nAPI_PORT=$((PORT + 1))\n(cd web && npm install --no-audit --no-fund && npm run build)\n(cd backend && PORT=$API_PORT KOTONOHA_DB=\"../$KOTONOHA_DB\" exec node --no-warnings src/main.ts) &\ncd web && KOTONOHA_USERS_FILE=\"../${KOTONOHA_USERS_FILE:-./data/users.json}\" KOTONOHA_BACKEND_URL=http://127.0.0.1:$API_PORT exec node --no-warnings src/server-main.ts\n";
    std::fs::write(out.join("start.sh"), start_sh).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out.join("start.sh"), std::fs::Permissions::from_mode(0o755));
    }
    std::fs::write(
        out.join("Dockerfile"),
        "# コンテナで動かすとき(Render・Fly.io・社内の Docker など)\nFROM node:22-slim\nWORKDIR /app\nCOPY . .\nRUN cd web && npm install --no-audit --no-fund && npm run build\nENV PORT=8080\nEXPOSE 8080\nCMD [\"sh\", \"start.sh\"]\n",
    )
    .map_err(|e| e.to_string())?;
    // 点検の結果(自動で入る安全対策・公開範囲・警告)
    let safety = std::process::Command::new(&exe)
        .arg("-C")
        .arg(&p.root)
        .arg("safety")
        .env("KOTONOHA_USAGE_LOG", "")
        .output()
        .map_err(|e| e.to_string())?;
    let app_name = a.env.app_name.clone().unwrap_or_else(|| "アプリ".into());
    let warns: Vec<String> = a.diags.items.iter().map(|d| format!("- ⚠ {}", d.message_ja)).collect();
    let api_json: serde_json::Value = std::fs::read_to_string(out.join("backend").join("api.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let check = format!(
        "# 公開前の点検({})\n\n## 注意(警告)\n{}\n\n## だれが何を見られる・できるか\n{}\n## 自動で入っている安全対策と、API ごとの公開範囲(詳しい人向け)\n```\n{}{}```\n",
        app_name,
        if warns.is_empty() { "- なし".to_string() } else { warns.join("\n") },
        plain_access(&api_json),
        String::from_utf8_lossy(&safety.stdout),
        String::from_utf8_lossy(&safety.stderr)
    );
    std::fs::write(out.join("点検の結果.md"), check).map_err(|e| e.to_string())?;
    std::fs::write(out.join("公開の手順.md"), PUBLISH_GUIDE.replace("{app}", &app_name)).map_err(|e| e.to_string())?;
    println!("公開の準備ができました: {}", out.display());
    println!("  1. 点検の結果.md で、だれが何を見られるかと注意を確かめる");
    println!("  2. 公開の手順.md のとおりに、置き場所を用意する(ここだけ外部のサービスを使います。ログインはアプリの中で利用者を登録します)");
    Ok(0)
}

const PUBLISH_GUIDE: &str = "# 「{app}」を公開する手順

この一式は `koto publish` が作りました。KOTONOHA が用意するのは、本番用の画面と API・安全な既定の設定・点検の結果までです。
**置き場所(インターネットから開けるサーバー)は、外部のサービスを使います**(契約・費用はそのサービスの条件によります)。
ログインは、最初から**アプリの中で利用者を登録する形**になっています(外部のサービスは要りません)。

## 1. ログインの仕組み
本番では「名前を入れてすぐ入る」(開発用ログイン)は使えません。最初から次の形になっています(`.env` の `KOTONOHA_LOCAL_LOGIN=1`)。
1. 初めて起動すると、起動の記録(ログ)に「最初の管理者を作ります。… /setup?t=… を開いてください」と出ます。その URL を開いて、管理者のログイン名とパスワードを決めます(この URL は一度だけ使えます)
2. ほかの人は、管理者が「利用者の管理」(ログインの画面から開けます)で**招待のリンク**を作って送ります。役割(受付係など)はそこで付けます
3. パスワードを忘れた人には、管理者が「再設定のリンク」を作って送ります
4. 利用者の一覧は `data/users.json` に入ります(パスワードはそのまま保存せず、元に戻せない形にしてあります)。**データと一緒にコピーを取ってください**
5. `.env` の `KOTONOHA_PUBLIC_URL` に、このアプリの URL(https://…)を入れておくと、招待のリンクがその URL で作られます

会社・学校のログインの仕組み(Google Workspace / Microsoft Entra ID / Auth0 など、OIDC)を使いたいときは、`.env` の `KOTONOHA_OIDC_…` に値を入れます
(入れると、アプリの中のログインの代わりにそちらを使います)。戻り先の URL には `https://あなたのドメイン/login/callback` を登録します。

## 2. 置き場所を決めて動かす
- **コンテナを置けるサービス**(Render・Fly.io・Google Cloud Run など): このフォルダを Git に入れて、`Dockerfile` で動かします。`.env` の値は、そのサービスの「環境変数」の画面に入れます(`.env` ファイルは上げません)
- **自分のサーバー・社内のパソコン**: Node.js 22.18 以上を入れて、`sh start.sh` で動かします。インターネットに出すときは HTTPS にします
- データは `data/data.sqlite` に入ります。**毎日コピーを取ってください**(バックアップ)。人数が多いときは PostgreSQL も使えます(`KOTONOHA_PG_URL`)

## 3. 使い始める前の確認
- `点検の結果.md` の「だれが何を見られるか」が思ったとおりか
- 注意(⚠)が残っていないか
- 試しに、役割ごとの人でログインして、見えてはいけないものが見えないか
- 初期データ(seed.json)は本番には入りません。最初のデータは画面から入れます
";

const STUDIO_FILES: &[(&str, &str)] = &[
    ("server.ts", include_str!("../../../runtime/studio/server.ts")),
    ("index.html", include_str!("../../../runtime/studio/index.html")),
    ("studio.js", include_str!("../../../runtime/studio/studio.js")),
    ("studio.css", include_str!("../../../runtime/studio/studio.css")),
];

/// `koto studio`: 画面でアプリを作る(ADR-0135)。データ・画面・動作・役割・約束・確かめるを画面で足し、
/// 日本語で指示でき、iPhone・Android の各機種の大きさでの表示を並べて確かめられる。変更はすべてソースに文として入る
fn cmd_studio(o: &mut Opts) -> Result<i32, String> {
    let port: u16 = take_value(&mut o.args, "--port")?
        .map(|p| p.parse().map_err(|_| "--port は数字です".to_string()))
        .transpose()?
        .unwrap_or(4000);
    no_extra(&o.args)?;
    std::fs::create_dir_all(&o.dir).map_err(|e| format!("{} を作れません: {e}", o.dir.display()))?;
    let root = o.dir.canonicalize().map_err(|e| format!("{} を開けません: {e}", o.dir.display()))?;
    // プロジェクトが無いフォルダで開くと、Studio の画面で「新しいアプリを作る」(ひな形を選ぶ)から始める
    // (黒い画面で koto init を打たずに済むように。第 6 回の分析)
    let dir = root.join(".koto").join("studio");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, content) in STUDIO_FILES {
        std::fs::write(dir.join(name), content).map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    eprintln!("KOTONOHA Studio を起動します: http://127.0.0.1:{port}/");
    eprintln!("  ブラウザで開くと、画面でデータ・画面・動作・役割を作れます。日本語で指示もできます。");
    eprintln!("  「見た目の確認」で iPhone・Android の各機種の大きさでの表示を並べて確かめられます。止めるとき: Ctrl+C");
    let status = std::process::Command::new("node")
        .args(["--no-warnings", "server.ts"])
        .env("KOTO_BIN", exe)
        .env("KOTO_PROJECT", &root)
        .env(
            "KOTONOHA_TEMPLATES",
            serde_json::to_string(&TEMPLATES.iter().map(|(n, d, _, _)| json!({"name": n, "desc": d})).collect::<Vec<_>>()).unwrap(),
        )
        .env("PORT", port.to_string())
        .env("KOTONOHA_EXIT_WITH_PARENT", std::process::id().to_string())
        .current_dir(&dir)
        .status()
        .map_err(|e| format!("Studio を起動できません(Node.js 22.18 以上が必要です): {e}"))?;
    Ok(status.code().unwrap_or(1))
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
    // 動作の中に書ける文(自分のデータ名で)
    s.push_str("\n動作(動作 名前(引数) -> 結果<識別子, 失敗の列挙>: の下)に書ける文:\n");
    for (d, info) in &a.env.data {
        let fields: Vec<&str> = info.fields.iter().map(|(n, _)| n.as_str()).filter(|n| *n != "ID").collect();
        if info.field("ID").is_none() {
            continue;
        }
        s.push_str(&format!("    件 = {d}を取得({d}ID)            … 1 件を取り出す\n"));
        if let Some((f, _)) = info.fields.iter().find(|(n, t)| n != "ID" && t.to_string() == "整数") {
            s.push_str(&format!("    件.{f} = 件.{f} + 1              … 書き換え(自動で保存)\n"));
        }
        s.push_str(&format!(
            "    {d}を登録({}) を返す   … 新しく登録(項目名: 値。順は自由、選択<…> の項目は省ける)\n",
            fields.iter().map(|f| format!("{f}: …")).collect::<Vec<_>>().join(", ")
        ));
    }
    s.push_str("    数量 > 0 を確認 または 失敗(\"数量不正\")   … 条件を満たさなければ失敗(それまでの変更は元に戻る)\n");
    // 確かめる
    let role = a.env.roles.keys().next().cloned();
    let who = role.as_ref().map(|r| format!("花子({r})")).unwrap_or_else(|| "花子".into());
    s.push_str("\n確かめる(確かめる 名前: の下。koto dev と koto test が自動で試す)に書ける文:\n");
    if let Some((act, sig)) = a.env.actions.iter().next() {
        let args: Vec<String> = sig.params.iter().map(|(n, _)| n.clone()).collect();
        s.push_str(&format!("    {who}が {act}({}) → 成功\n", args.join(", ")));
        s.push_str(&format!("    {who}が {act}({}) → 失敗(\"…\")\n", args.join(", ")));
        s.push_str(&format!("    ログインしていない人が {act}({}) → ログインが必要\n", args.join(", ")));
    }
    if let Some((d, info)) = a.env.data.iter().next() {
        s.push_str(&format!("    花子から {d}が 1件見える\n"));
        if let Some((f, _)) = info.fields.iter().find(|(n, _)| n != "ID") {
            s.push_str(&format!("    {d}(ID)の{f}は 値\n"));
        }
    }
    s.push_str("\n役割(誰が何をできるか。トップレベルに書く):\n    役割 厨房:\n        権限 = [注文.読取, 注文.更新, 全件閲覧.注文]\n");
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

/// `koto token`: koto dev の秘密で、動作確認用のトークンを作る
fn cmd_token(o: &mut Opts) -> Result<i32, String> {
    if take_flag(&mut o.args, "--help") || take_flag(&mut o.args, "-h") {
        println!("使い方: koto token --sub 名前 [--role 役割 | --admin | --caps 権限,…]\n  koto dev で動かしているアプリの動作確認用トークン(12 時間有効)を標準出力に出します。\n  ソースに役割があれば --role で役割を指定します。--admin は全ての権限(全件閲覧を含む)。");
        return Ok(0);
    }
    let sub = take_value(&mut o.args, "--sub")?.ok_or("--sub 名前 が必要です(例: koto token --sub 花子)")?;
    let admin = take_flag(&mut o.args, "--admin");
    let role = take_value(&mut o.args, "--role")?;
    let caps_arg = take_value(&mut o.args, "--caps")?;
    no_extra(&o.args)?;
    let dev = o.dir.join(".koto").join("dev");
    let secret = std::fs::read_to_string(dev.join("secret")).map_err(|_| "先に koto dev を一度実行してください(開発用の秘密がまだありません)".to_string())?;
    let caps = match (caps_arg, role.as_deref()) {
        (Some(c), _) => c,
        (None, Some(r)) => {
            let api: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dev.join("backend/api.json")).map_err(|e| e.to_string())?).unwrap_or_default();
            let v = api["roles"].get(r).and_then(|x| x.as_array()).ok_or_else(|| {
                let names: Vec<String> = api["roles"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
                format!(
                    "役割 `{r}` はありません(ソースの役割: {})",
                    if names.is_empty() { "なし".into() } else { names.join(" / ") }
                )
            })?;
            v.iter().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(",")
        }
        (None, None) if !admin => {
            // ソースに役割があるのに指定が無いと「何でもできる人」になり紛らわしいので、役割を選ばせる
            let api: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dev.join("backend/api.json")).map_err(|e| e.to_string())?).unwrap_or_default();
            if let Some(m) = api["roles"].as_object().filter(|m| !m.is_empty()) {
                return Err(format!(
                    "このアプリには役割があります。--role で選んでください(役割: {})。全ての権限が必要なら --admin",
                    m.keys().cloned().collect::<Vec<_>>().join(" / ")
                ));
            }
            all_caps(&api).into_iter().filter(|c| !c.starts_with("全件閲覧.")).collect::<Vec<_>>().join(",")
        }
        (None, None) => {
            let api: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dev.join("backend/api.json")).map_err(|e| e.to_string())?).unwrap_or_default();
            all_caps(&api)
                .into_iter()
                .filter(|c| admin || !c.starts_with("全件閲覧."))
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    let out = std::process::Command::new("node")
        .args(["--no-warnings", "tools/dev-token.ts", "--sub", &sub, "--caps", &caps, "--ttl", "43200"])
        .env("KOTONOHA_JWT_SECRET", secret.trim())
        .current_dir(dev.join("backend"))
        .output()
        .map_err(|e| format!("node を実行できません: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    if !o.json && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        let who = match (&role, admin) {
            (Some(r), _) => format!("役割「{r}」"),
            (None, true) => "管理者(全件閲覧あり)".into(),
            _ => "一般の利用者".into(),
        };
        eprintln!("({sub} として{who}。12 時間有効。使い方: curl -H \"Authorization: Bearer <この文字列>\" …)");
    }
    Ok(0)
}

/// `koto dev` の作り直しの合図(ソースが変わった)
const DEV_RESTART: i32 = -100;

/// ソースの変更を見張る対象(kotonoha.json・src・seed.json・分析の SQL・拡張の実装)の更新時刻と大きさ
fn source_fingerprint(root: &Path) -> Vec<(PathBuf, Option<std::time::SystemTime>, u64)> {
    fn walk(d: &Path, out: &mut Vec<(PathBuf, Option<std::time::SystemTime>, u64)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == "build" || name == "out" {
                continue;
            }
            match e.metadata() {
                Ok(m) if m.is_dir() => walk(&p, out),
                Ok(m) => out.push((p, m.modified().ok(), m.len())),
                Err(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

/// `koto dev`: 検査 → backend と web を生成 → 初期データ → 起動(画面と API は同じポート)。
/// 起動した後はソースを見張り、変わったら作り直して起動し直す(止めなくてよい)
fn cmd_dev(o: &mut Opts) -> Result<i32, String> {
    let port: u16 = take_value(&mut o.args, "--port")?
        .map(|p| p.parse().map_err(|_| "--port は数字です".to_string()))
        .transpose()?
        .unwrap_or(3000);
    let no_start = take_flag(&mut o.args, "--no-start");
    no_extra(&o.args)?;
    let mut started = false;
    loop {
        let fp = source_fingerprint(&o.dir);
        let code = dev_once(o, port, no_start, &mut started)?;
        if code == DEV_RESTART {
            eprintln!("\nソースが変わりました。検査して作り直します…\n");
            continue;
        }
        if !started || no_start {
            return Ok(code);
        }
        // 一度起動した後に検査で止まったときは、直すのを待つ(Ctrl+C で終わる)
        eprintln!("\nソースを直して保存すると、自動でやり直します(止めるとき: Ctrl+C)");
        while source_fingerprint(&o.dir) == fp {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        eprintln!("\nソースが変わりました。検査して作り直します…\n");
    }
}

fn dev_once(o: &mut Opts, port: u16, no_start: bool, ever_started: &mut bool) -> Result<i32, String> {
    let fp = source_fingerprint(&o.dir);
    let (p, a) = match analyzed(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    // 子プロセスは別のフォルダで動くので、絶対パスにしておく(相対の -C だと DB が別の場所にできていた)
    let root = o.dir.canonicalize().map_err(|e| format!("{} を開けません: {e}", o.dir.display()))?;
    let dev = root.join(".koto").join("dev");
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
    // 部品のダウンロードが最後まで終わった印。印が無い node_modules は途中で切れたものなので取り直す
    // (ディスクがいっぱいで途中で切れ、「画面のビルド に失敗しました」が直らなかった。第 8 回評価)
    let installed = web.join("node_modules").join(".koto-installed");
    let install = |why: &str| -> Result<(), String> {
        eprintln!("{why}");
        let _ = std::fs::remove_dir_all(web.join("node_modules"));
        run_cmd(
            std::process::Command::new("npm")
                .args(["install", "--no-audit", "--no-fund", "--silent"])
                .current_dir(&web),
            "画面の部品のダウンロード(npm install)",
        )?;
        std::fs::write(&installed, "ok\n").map_err(|e| format!("画面の部品の印を書けません: {e}(ディスクの空きを確かめてください)"))
    };
    if !installed.exists() {
        install(if web.join("node_modules").exists() {
            "画面の部品のダウンロードが途中で終わっていたので、取り直します。1 分ほどかかります…"
        } else {
            "初回だけ、画面の部品(React など)をダウンロードします。1 分ほどかかります…"
        })?;
    }
    let build = || {
        run_cmd(
            std::process::Command::new("npm").args(["run", "build", "--silent"]).current_dir(&web),
            "画面のビルド",
        )
    };
    if build().is_err() {
        // 部品が壊れていることがあるので、一度だけ取り直してから作り直す
        install("画面のビルドに失敗したので、画面の部品を取り直してもう一度作ります…")?;
        build().map_err(|e| format!("{e}。ディスクの空き・ネットワークを確かめてください(.koto/dev/web/node_modules を消すと、次に取り直します)"))?;
    }
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
    let seed = root.join("seed.json");
    // データの形(項目や型)を変えて、開発用のデータを自動で移せないときは、前のデータを脇に置いて作り直す
    // (開発用のデータは消えてよい。止まって INT001 を見せるより分かりやすい。第 5 回評価)
    if db.exists() {
        let out = std::process::Command::new("node")
            .args(["--no-warnings", "-e", "import('./src/runtime/db.ts').then(m => import('./src/app.generated.ts').then(a => m.openDb(process.env.KOTONOHA_DB, a.tables))).then(d => d.close()).catch(e => { console.error(String(e?.message ?? e)); process.exit(3); })"])
            .env("KOTONOHA_DB", &db)
            .current_dir(&be)
            .output()
            .map_err(|e| format!("node を実行できません: {e}"))?;
        if !out.status.success() {
            let old = dev.join("data.sqlite.前の形");
            let _ = std::fs::remove_file(&old);
            for ext in ["", "-wal", "-shm"] {
                let from = dev.join(format!("data.sqlite{ext}"));
                if from.exists() {
                    std::fs::rename(&from, dev.join(format!("data.sqlite.前の形{ext}"))).map_err(|e| e.to_string())?;
                }
            }
            eprintln!("データの形が変わったので、開発用のデータを作り直しました(初期データ seed.json を入れ直します)。");
            eprintln!(
                "  前のデータは {} に残しています。理由: {}",
                old.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    // 初期データ(seed.json)を書き換えたら、開発用のデータを作り直す(同じ ID は飛ばすので、ひな形の古いデータが残った。第 6 回評価)
    // 列挙の値を変えたときも作り直す(前の値の行が残り、読むときに「内部エラー」になった。第 8 回評価)
    let applied = dev.join("seed.applied.json");
    let enums_applied = dev.join("enums.applied.json");
    let enums_now = api.get("enums").map(|e| e.to_string()).unwrap_or_default();
    let enums_changed = enums_applied.exists() && std::fs::read_to_string(&enums_applied).unwrap_or_default() != enums_now;
    let seed_now = std::fs::read_to_string(&seed).unwrap_or_default();
    if db.exists() && ((applied.exists() && std::fs::read_to_string(&applied).unwrap_or_default() != seed_now) || enums_changed) {
        let old = dev.join("data.sqlite.前の初期データ");
        for ext in ["", "-wal", "-shm"] {
            let from = dev.join(format!("data.sqlite{ext}"));
            let _ = std::fs::remove_file(dev.join(format!("data.sqlite.前の初期データ{ext}")));
            if from.exists() {
                std::fs::rename(&from, dev.join(format!("data.sqlite.前の初期データ{ext}"))).map_err(|e| e.to_string())?;
            }
        }
        eprintln!(
            "{}ので、開発用のデータを作り直しました(前のデータは {} に残しています)。",
            if enums_changed {
                "列挙(選択肢)の値が変わった"
            } else {
                "初期データ(seed.json)が変わった"
            },
            old.display()
        );
    }
    let _ = std::fs::write(&enums_applied, &enums_now);
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
        let _ = std::fs::write(&applied, &seed_now);
    }
    // ソースの「確かめる」を、開発用とは別の空のデータベースで試す(結果だけ表示し、失敗しても起動は続ける)
    if be.join("tools/scenarios.ts").exists() {
        eprintln!();
        if !run_scenarios(&be, &seed)? {
            eprintln!("(上の ✗ は、書いた「確かめる」と動きが違うところです。ソースを直すか、確かめるの期待を直してください)");
        }
    }
    let caps = all_caps(&api);
    let entry = api["entry_screen"].as_str().unwrap_or("");
    eprintln!("\n準備ができました。");
    eprintln!("  画面:   http://127.0.0.1:{port}/   (最初の画面: {entry})");
    let role_names: Vec<String> = api["roles"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    if role_names.is_empty() {
        eprintln!("  ログイン: 画面右上の「ログイン」→ 名前を入れて「一般の利用者として入る」か「管理者として入る」");
    } else {
        // 役割に「管理者」があるときは、組込みの全権限ログインを別の名前で出す(同じ名前のボタンが 2 つ並ばないように)
        let all = if role_names.iter().any(|r| r == "管理者") {
            "すべての権限で入る"
        } else {
            "管理者として入る"
        };
        eprintln!(
            "  ログイン: 画面右上の「ログイン」→ 名前を入れて {}「{all}」",
            role_names.iter().map(|r| format!("「{r}として入る」")).collect::<Vec<_>>().join("")
        );
    }
    eprintln!("  API:    http://127.0.0.1:{port}/api/…  (Authorization: Bearer で呼べます)");
    eprintln!(
        "  動作確認のトークン: koto -C {} token --sub 花子 [--admin]   (koto dev 専用の秘密で作ります)",
        o.dir.display()
    );
    eprintln!("  データ: {}(消すと空からやり直せます)", db.display());
    eprintln!("  止めるとき: Ctrl+C(ソースを直して保存すると、自動で作り直します)\n");
    eprint!("{}", safety_text(&api));
    if no_start {
        return Ok(0);
    }
    // backend のポートは OS に空きを選ばせる(port+1 だと使用中や、ブラウザの fetch が禁止するポート(4045 など)に当たることがある)
    let bport = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("空きポートを探せません: {e}"))?;
    let mut backend = std::process::Command::new("node")
        .args(["--no-warnings", "src/main.ts"])
        .env("PORT", bport.to_string())
        .env("KOTONOHA_DB", &db)
        .env("KOTONOHA_JWT_SECRET", &secret)
        .env("KOTONOHA_EXIT_WITH_PARENT", std::process::id().to_string())
        .current_dir(&be)
        .spawn()
        .map_err(|e| format!("backend を起動できません: {e}"))?;
    // backend が接続を受け付けるまで待ってから画面のサーバーを起動する
    // (先に画面だけ開けると、最初の操作が「内部エラー」になることがある)
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], bport));
    let started = std::time::Instant::now();
    while std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200)).is_err() {
        if backend.try_wait().map_err(|e| e.to_string())?.is_some() {
            eprintln!("backend が起動できませんでした(上のメッセージを確認してください)");
            return Ok(1);
        }
        if started.elapsed() > std::time::Duration::from_secs(120) {
            let _ = backend.kill();
            eprintln!("backend の起動を 120 秒待ちましたが応答がありません");
            return Ok(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut webp = std::process::Command::new("node")
        .args(["--no-warnings", "src/server-main.ts"])
        .env("PORT", port.to_string())
        .env("KOTONOHA_BACKEND_URL", format!("http://127.0.0.1:{bport}"))
        .env("KOTONOHA_DEV_LOGIN", "1")
        .env("KOTONOHA_DEV_JWT_SECRET", &secret)
        .env("KOTONOHA_DEV_CAPS", caps.join(","))
        .env("KOTONOHA_DEV_ROLES", serde_json::to_string(&api["roles"]).unwrap_or_else(|_| "{}".into()))
        .env("KOTONOHA_DEV_GENERAL_CAPS", general_caps(&api).join(","))
        .env("KOTONOHA_EXIT_WITH_PARENT", std::process::id().to_string())
        .current_dir(&web)
        .spawn()
        .map_err(|e| format!("画面のサーバーを起動できません: {e}"))?;
    *ever_started = true;
    let mut tick = 0u32;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let b = backend.try_wait().map_err(|e| e.to_string())?;
        let w = webp.try_wait().map_err(|e| e.to_string())?;
        if b.is_some() || w.is_some() {
            let _ = backend.kill();
            let _ = webp.kill();
            eprintln!("サーバーが止まりました(ポート {port} が使用中でないか確認してください。--port で変えられます)");
            return Ok(1);
        }
        tick += 1;
        if tick.is_multiple_of(3) && source_fingerprint(&o.dir) != fp {
            let _ = backend.kill();
            let _ = webp.kill();
            let _ = backend.wait();
            let _ = webp.wait();
            return Ok(DEV_RESTART);
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
    let mut diags = a.diags.clone();
    // kotonoha.json の targets に書いたターゲットで生成できるかも確かめる
    // (check は合格したのに koto dev / build で TGT001 になる、を防ぐ。第 3 回ペルソナ評価)
    if a.ok() {
        let mut seen = std::collections::BTreeSet::new();
        for t in &p.config.targets {
            if let Err(ds) = drv::build_target(&p, &a, t) {
                for d in ds {
                    if seen.insert(d.message_ja.clone()) {
                        diags.push(d.hint(format!(
                            "kotonoha.json の targets に {t} があるため検査しました(そのターゲットが要らなければ targets から外せます)"
                        )));
                    }
                }
            }
        }
    }
    for d in diags.items.iter_mut() {
        if d.file.is_empty() {
            if let Some((path, _)) = p.files.get(&d.span.file) {
                if d.span.line > 0 {
                    d.file = path.clone();
                }
            }
        }
    }
    let code = report(
        o,
        &diags,
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
    let (p, ap) = match load(o) {
        Ok(x) => x,
        Err(c) => return Ok(c),
    };
    let local = LocalOpts::take(&mut o.args)?;
    no_extra(&o.args)?;
    let a = drv::analyze(&p, &ap);
    let adapter: Box<dyn ResolverAdapter> = match adapter_name.as_str() {
        "offline" => Box::new(OfflineAdapter),
        "fixture" => {
            let f = fixture.ok_or("--adapter fixture には --fixture <file> が必要です")?;
            let t = std::fs::read_to_string(o.dir.join(&f)).map_err(|e| format!("{f} を読めません: {e}"))?;
            Box::new(FixtureAdapter::from_json(&t)?)
        }
        "local" => Box::new(local.adapter(&a, learn_memory())?),
        "memory" => Box::new(kotonoha_resolver::local::MemoryAdapter { memory: learn_memory() }),
        other => {
            return Err(format!(
                "未知のアダプタ `{other}` です(offline / fixture / local / memory。外部の AI への送信は ADR-006 で未決定)"
            ))
        }
    };
    if adapter.sends_source() {
        return Err("ソースを外部送信するアダプタは明示同意の仕組み(NFR-010)が未実装のため使えません".into());
    }
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
    // 学習記録(koto learn init で有効にした人だけ): 選んだ解釈と、選ばなかった候補を貯める
    if let Some(dir) = kotonoha_resolver::learn::default_dir().filter(|d| d.is_dir()) {
        let rejected = cand
            .interpretations
            .iter()
            .filter(|x| x.canonical != it.canonical)
            .map(|x| x.canonical.clone())
            .collect();
        let r = learn_record(&p, &a, u, &it.canonical, &it.rationale, rejected, &rec.approved_at);
        match kotonoha_resolver::learn::Memory::append(&dir, r) {
            Ok(_) if !o.json => eprintln!("学習記録に追加しました({})", dir.display()),
            Ok(_) => {}
            Err(e) => eprintln!("注意: {e}"),
        }
    }
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
        "implemented": ["lexer", "parser", "ast-printer", "type-check(strict layer)", "knowledge-registry(ranking)", "intent-patterns", "effects/capabilities", "vag", "lock", "approval-records(ed25519 signing)", "resolver(offline/fixture/local open model/memory)", "learning memory(koto learn: export sft/dpo, eval gate)", "sql-lowering(ranking: sqlite/postgres)", "app-ir + api-contract", "functions + builtins", "external functions(typescript/python)", "analysis(read-only sql)", "extension components", "experiments(a/b)", "backend(node/ts/sqlite/postgres)", "web(react ssr)", "ios(swiftui, CI build)", "android(compose, CI build)"],
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
    // ソースの「確かめる」(ADR-0130)
    let scen_ok = run_scenarios(&dir, &o.dir.join("seed.json"))?;
    if keep {
        eprintln!("生成物: {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
    // テスト失敗は終了コード 1(ADR-0103 追補)
    Ok(if status.success() && scen_ok { 0 } else { 1 })
}

/// 生成した backend で「確かめる」を実行し、結果を日本語で表示する。確かめるが無ければ何もしない
fn run_scenarios(be: &Path, seed: &Path) -> Result<bool, String> {
    if !be.join("tools/scenarios.ts").exists() {
        return Ok(true);
    }
    let mut cmd = std::process::Command::new("node");
    cmd.args(["--no-warnings", "tools/scenarios.ts"]).current_dir(be);
    if seed.exists() {
        cmd.env("KOTONOHA_SEED", seed.canonicalize().map_err(|e| e.to_string())?);
    }
    let out = cmd.output().map_err(|e| format!("node を実行できません: {e}"))?;
    eprint!("{}", String::from_utf8_lossy(&out.stdout));
    if !out.status.success() && out.stdout.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.status.success())
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

/// 手元のモデルの指定(--model / --endpoint、または環境変数)。
struct LocalOpts {
    model: Option<String>,
    endpoint: String,
}

impl LocalOpts {
    fn take(args: &mut Vec<String>) -> Result<LocalOpts, String> {
        let model = take_value(args, "--model")?.or_else(|| std::env::var("KOTONOHA_LOCAL_MODEL").ok().filter(|x| !x.is_empty()));
        let endpoint = take_value(args, "--endpoint")?
            .or_else(|| std::env::var("KOTONOHA_LOCAL_ENDPOINT").ok().filter(|x| !x.is_empty()))
            .unwrap_or_else(|| kotonoha_resolver::local::DEFAULT_ENDPOINT.into());
        kotonoha_resolver::local::parse_local_endpoint(&endpoint)?;
        Ok(LocalOpts { model, endpoint })
    }
    fn adapter(&self, a: &drv::Analysis, memory: kotonoha_resolver::learn::Memory) -> Result<kotonoha_resolver::local::LocalModelAdapter, String> {
        let model = self
            .model
            .clone()
            .ok_or("--model でモデル名を指定してください(例: --model qwen2.5:7b。環境変数 KOTONOHA_LOCAL_MODEL でも可)")?;
        let secs = std::env::var("KOTONOHA_LOCAL_TIMEOUT").ok().and_then(|x| x.parse().ok()).unwrap_or(180);
        Ok(kotonoha_resolver::local::LocalModelAdapter {
            endpoint: self.endpoint.clone(),
            model,
            timeout: std::time::Duration::from_secs(secs),
            api_key: std::env::var("KOTONOHA_LOCAL_API_KEY").ok().filter(|x| !x.is_empty()),
            screen_examples: kotonoha_intent::example_sentences(&a.env, &a.registry, false),
            contract_examples: kotonoha_intent::example_sentences(&a.env, &a.registry, true),
            memory,
        })
    }
}

fn learn_memory() -> kotonoha_resolver::learn::Memory {
    kotonoha_resolver::learn::default_dir()
        .map(|d| kotonoha_resolver::learn::Memory::load(&d).0)
        .unwrap_or_default()
}

fn learn_record(
    p: &drv::Project,
    a: &drv::Analysis,
    u: &kotonoha_intent::Unresolved,
    chosen: &str,
    rationale: &str,
    rejected: Vec<String>,
    at: &str,
) -> kotonoha_resolver::learn::MemoryRecord {
    let pool = kotonoha_intent::example_sentences(&a.env, &a.registry, u.owner.starts_with("action."));
    let ctx = kotonoha_resolver::learn::PromptContext::new(&u.sentence, &u.questions, &pool);
    kotonoha_resolver::learn::MemoryRecord {
        v: 1,
        sentence: u.sentence.clone(),
        chosen: chosen.into(),
        rationale: rationale.into(),
        rejected,
        context: ctx.examples,
        questions: u.questions.clone(),
        project: p.config.name.clone(),
        at: at.into(),
    }
}

fn cmd_learn(o: &mut Opts) -> Result<i32, String> {
    use kotonoha_resolver::learn::{self, Memory};
    let sub = if o.args.is_empty() { String::from("status") } else { o.args.remove(0) };
    let dir = learn::default_dir().ok_or("学習記録の置き場所が決められません(KOTONOHA_LEARN_DIR を指定してください)")?;
    match sub.as_str() {
        "init" => {
            no_extra(&o.args)?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("{} を作れません: {e}", dir.display()))?;
            if !o.json {
                eprintln!("学習記録を有効にしました: {}", dir.display());
                eprintln!("これから koto approve で承認した解釈が記録され、koto resolve --adapter local / memory で使われます");
                eprintln!("今あるプロジェクトの承認を取り込むには、各プロジェクトで koto learn collect を実行してください");
            }
            Ok(report(o, &Diagnostics::new(), json!({"dir": dir})))
        }
        "status" => {
            no_extra(&o.args)?;
            let (m, bad) = Memory::load(&dir);
            let evals = learn::load_evals(&dir);
            let projects: std::collections::BTreeSet<&str> = m.records.iter().map(|r| r.project.as_str()).collect();
            let negatives: usize = m.records.iter().map(|r| r.rejected.len()).sum();
            if !o.json {
                if !dir.is_dir() {
                    eprintln!("学習記録はまだ有効ではありません(koto learn init で有効にします)");
                } else {
                    eprintln!("学習記録: {}", dir.display());
                    eprintln!(
                        "  承認した言い換え {} 件(プロジェクト {} 個)、選ばれなかった候補 {} 件",
                        m.records.len(),
                        projects.len(),
                        negatives
                    );
                    if bad > 0 {
                        eprintln!("  読めなかった行 {bad} 件");
                    }
                    if evals.is_empty() {
                        eprintln!("  評価の記録はまだありません(koto learn eval --model 名前)");
                    } else {
                        eprintln!("  評価の推移(1 番目の候補が当たった割合):");
                        for e in &evals {
                            eprintln!(
                                "    {}  {}  {:.0}%({}/{}、壊れた応答 {})",
                                e.at,
                                e.model,
                                e.top1_rate() * 100.0,
                                e.top1,
                                e.cases,
                                e.broken
                            );
                        }
                    }
                }
            }
            Ok(report(
                o,
                &Diagnostics::new(),
                json!({"dir": dir, "records": m.records.len(), "negatives": negatives, "evals": evals}),
            ))
        }
        "collect" => {
            no_extra(&o.args)?;
            if !dir.is_dir() {
                return Err("学習記録が有効ではありません。先に koto learn init を実行してください".into());
            }
            let (p, ap) = match load(o) {
                Ok(x) => x,
                Err(c) => return Ok(c),
            };
            // 承認記録が当たる前の状態(未解決の意図)を作り、その文脈で記録する
            let a0 = drv::analyze(&p, &kotonoha_lock::ApprovalsFile::empty());
            let mut added = 0;
            for r in &ap.records {
                let Some(u) = a0
                    .unresolved()
                    .iter()
                    .find(|u| u.intent_id == r.intent_id && u.sentence_hash == r.sentence_hash)
                else {
                    continue;
                };
                if Memory::append(&dir, learn_record(&p, &a0, u, &r.canonical, &r.note, vec![], &r.approved_at))? {
                    added += 1;
                }
            }
            if !o.json {
                eprintln!("このプロジェクトの承認 {} 件のうち {added} 件を学習記録に追加しました", ap.records.len());
            }
            Ok(report(o, &Diagnostics::new(), json!({"added": added})))
        }
        "export" => {
            let out = take_value(&mut o.args, "--out")?.ok_or("--out <ディレクトリ> を指定してください")?;
            no_extra(&o.args)?;
            let (m, _) = Memory::load(&dir);
            if m.records.is_empty() {
                return Err("学習記録が空です(koto approve で承認すると貯まります)".into());
            }
            let (sft, dpo) = learn::export(&m);
            let out = PathBuf::from(out);
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            std::fs::write(out.join("sft.jsonl"), &sft).map_err(|e| e.to_string())?;
            std::fs::write(out.join("dpo.jsonl"), &dpo).map_err(|e| e.to_string())?;
            std::fs::write(out.join("README.md"), LEARN_EXPORT_README).map_err(|e| e.to_string())?;
            if !o.json {
                eprintln!(
                    "{} に書き出しました: sft.jsonl {} 件、dpo.jsonl {} 件(使い方は README.md)",
                    out.display(),
                    sft.lines().count(),
                    dpo.lines().count()
                );
            }
            Ok(report(o, &Diagnostics::new(), json!({"sft": sft.lines().count(), "dpo": dpo.lines().count()})))
        }
        "eval" => {
            let min: Option<f64> = match take_value(&mut o.args, "--min")? {
                None => None,
                Some(x) => Some(x.parse().map_err(|_| "--min は 0〜1 の数です(例: 0.8)")?),
            };
            let local = LocalOpts::take(&mut o.args)?;
            no_extra(&o.args)?;
            let (m, _) = Memory::load(&dir);
            if m.records.is_empty() {
                return Err("学習記録が空のため評価できません".into());
            }
            let model = local.model.clone().ok_or("--model でモデル名を指定してください")?;
            let ad = kotonoha_resolver::local::LocalModelAdapter {
                endpoint: local.endpoint.clone(),
                model: model.clone(),
                timeout: std::time::Duration::from_secs(std::env::var("KOTONOHA_LOCAL_TIMEOUT").ok().and_then(|x| x.parse().ok()).unwrap_or(180)),
                api_key: std::env::var("KOTONOHA_LOCAL_API_KEY").ok().filter(|x| !x.is_empty()),
                screen_examples: vec![],
                contract_examples: vec![],
                memory: Memory::default(),
            };
            let mut seen = std::collections::BTreeSet::new();
            let (mut cases, mut top1, mut any, mut broken) = (0, 0, 0, 0);
            let mut misses = Vec::new();
            for (i, r) in m.records.iter().enumerate() {
                if !seen.insert((learn::normalize(&r.sentence), r.chosen.clone())) {
                    continue;
                }
                cases += 1;
                let u = kotonoha_intent::Unresolved {
                    intent_id: format!("評価#{i}"),
                    owner: "評価".into(),
                    sentence: r.sentence.clone(),
                    sentence_hash: "評価".into(),
                    mode: kotonoha_intent::Mode::FlexConfirm,
                    questions: r.questions.clone(),
                    span: Span::default(),
                };
                let ctx = learn::PromptContext {
                    sentence: r.sentence.clone(),
                    questions: r.questions.clone(),
                    examples: r.context.clone(),
                };
                // その文の答えは見せない(他の文の承認だけを例にする)
                let got = ad
                    .ask(&u, &ctx, &m.without(&r.sentence))
                    .and_then(|v| kotonoha_resolver::validate_candidate(&v, &u).map_err(|e| e.join("; ")));
                match got {
                    Err(e) => {
                        broken += 1;
                        misses.push(format!("「{}」: 応答が使えません({e})", r.sentence));
                    }
                    Ok(c) => {
                        let names: Vec<String> = c.interpretations.iter().map(|x| learn::normalize(&x.canonical)).collect();
                        let want = learn::normalize(&r.chosen);
                        if names.first() == Some(&want) {
                            top1 += 1;
                        } else {
                            misses.push(format!("「{}」: 期待「{}」/ 候補 {:?}", r.sentence, r.chosen, names));
                        }
                        if names.contains(&want) {
                            any += 1;
                        }
                    }
                }
            }
            let e = learn::EvalRecord {
                at: drv::now_rfc3339(),
                model,
                cases,
                top1,
                any,
                broken,
            };
            learn::append_eval(&dir, &e)?;
            let prev = learn::load_evals(&dir).into_iter().rev().nth(1);
            if !o.json {
                eprintln!(
                    "評価: {} 件中、1 番目の候補が承認と一致 {} 件({:.0}%)、いずれかの候補に含む {} 件、壊れた応答 {} 件",
                    cases,
                    top1,
                    e.top1_rate() * 100.0,
                    any,
                    broken
                );
                if let Some(p) = &prev {
                    eprintln!("  前回({} {}): {:.0}%", p.at, p.model, p.top1_rate() * 100.0);
                }
                for x in misses.iter().take(10) {
                    eprintln!("  外れ: {x}");
                }
            }
            let mut diags = Diagnostics::new();
            if let Some(min) = min {
                if e.top1_rate() < min {
                    diags.push(Diagnostic::error(
                        Code::AMB001,
                        Span::default(),
                        format!(
                            "一致率 {:.0}% が基準 {:.0}% に届きません。このモデルは採用しないでください",
                            e.top1_rate() * 100.0,
                            min * 100.0
                        ),
                    ));
                }
            }
            Ok(report(o, &diags, json!({"eval": e, "misses": misses})))
        }
        other => Err(format!("未知のサブコマンド `{other}` です(init / status / collect / export / eval)")),
    }
}

const LEARN_EXPORT_README: &str = include_str!("../templates/learn_export_README.md");
