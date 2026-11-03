//! コンパイラパイプライン(技術仕様書 §12)。
//!
//! UTF-8 source → Lexer → Parser → Surface AST → 宣言収集/型検査 → Knowledge Resolver
//! → Intent 解決(承認済み解釈の適用)→ Effect/Capability Validator → Typed VAG → (ロック検証) → Target 対応検査/Lowering
//!
//! 各段は前段の不変データだけを入力にし、AI もネットワークも使わない(NFR-002, TC-009)。

use kotonoha_ast::{Decl, SourceFile};
use kotonoha_diag::{Code, Diagnostic, Diagnostics, FileId, Span};
use kotonoha_graph::{build_vag, check_integrity, hash_value, sha256_hex, GraphInput, Status};
use kotonoha_intent::{ApprovedInterpretation, IntentModel, Unresolved};
use kotonoha_knowledge::{Knowledge, Registry};
use kotonoha_lock::{ApprovalReference, ApprovalVerifier, ApprovalsFile, Authenticity, LocalUnsigned, LockFile, LockedInterpretation, LOCK_VERSION};
use kotonoha_types::{Env, Facts};
use kotonoha_validator::Validation;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use kotonoha_lock::{APPROVALS_FILE, LOCK_FILE};
pub use kotonoha_resolver::CANDIDATES_FILE;

pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROJECT_FILE: &str = "kotonoha.json";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
    pub sources: Vec<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub config: ProjectConfig,
    /// FileId → (プロジェクト相対パス, 内容)
    pub files: BTreeMap<FileId, (String, String)>,
}

fn io_err(msg: String) -> Diagnostics {
    let mut d = Diagnostics::new();
    d.push(Diagnostic::error(Code::INT001, Span::default(), msg));
    d
}

pub fn load_project(root: &Path) -> Result<Project, Diagnostics> {
    let cfg_path = root.join(PROJECT_FILE);
    let text = std::fs::read_to_string(&cfg_path).map_err(|e| io_err(format!("{} を読めません: {e}", cfg_path.display())))?;
    let config: ProjectConfig = serde_json::from_str(&text).map_err(|e| io_err(format!("{PROJECT_FILE} の形式が不正です: {e}")))?;
    let mut srcs = config.sources.clone();
    srcs.sort();
    srcs.dedup();
    let mut files = BTreeMap::new();
    for (i, rel) in srcs.iter().enumerate() {
        if rel.contains("..") || Path::new(rel).is_absolute() {
            return Err(io_err(format!("ソースパス `{rel}` はプロジェクト内の相対パスで指定してください")));
        }
        let bytes = std::fs::read(root.join(rel)).map_err(|e| io_err(format!("{rel} を読めません: {e}")))?;
        let content = String::from_utf8(bytes).map_err(|_| {
            let mut d = Diagnostics::new();
            d.push(Diagnostic {
                file: rel.clone(),
                ..Diagnostic::error(Code::SYN001, Span::default(), "UTF-8 として不正なバイト列です")
            });
            d
        })?;
        files.insert(FileId(i as u32), (rel.clone(), content));
    }
    for t in &config.targets {
        if kotonoha_codegen::TargetKind::parse(t).is_none() {
            return Err(io_err(format!("未知のターゲット `{t}` です(web / backend / ios / android)")));
        }
    }
    Ok(Project {
        root: root.to_path_buf(),
        config,
        files,
    })
}

/// テストやツール用: メモリ上のソースからプロジェクトを作る。
pub fn project_from_sources(name: &str, sources: &[(&str, &str)], targets: &[&str]) -> Project {
    let mut files = BTreeMap::new();
    let mut sorted: Vec<_> = sources.to_vec();
    sorted.sort();
    for (i, (p, c)) in sorted.iter().enumerate() {
        files.insert(FileId(i as u32), (p.to_string(), c.to_string()));
    }
    Project {
        root: PathBuf::from("."),
        config: ProjectConfig {
            name: name.into(),
            sources: sorted.iter().map(|s| s.0.to_string()).collect(),
            targets: targets.iter().map(|s| s.to_string()).collect(),
        },
        files,
    }
}

pub fn load_approvals(root: &Path) -> Result<ApprovalsFile, Diagnostics> {
    let p = root.join(APPROVALS_FILE);
    if !p.exists() {
        return Ok(ApprovalsFile::empty());
    }
    let t = std::fs::read_to_string(&p).map_err(|e| io_err(format!("{APPROVALS_FILE} を読めません: {e}")))?;
    serde_json::from_str(&t).map_err(|e| {
        let mut d = Diagnostics::new();
        d.push(Diagnostic::error(
            Code::APP001,
            Span::default(),
            format!("{APPROVALS_FILE} の形式が不正です: {e}"),
        ));
        d
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashes {
    pub source: String,
    pub knowledge: String,
    pub schema: String,
    pub graph: String,
}

pub struct Analysis {
    pub diags: Diagnostics,
    pub asts: Vec<SourceFile>,
    pub env: Env,
    pub registry: Registry,
    pub intents: IntentModel,
    pub validation: Validation,
    pub facts: BTreeMap<String, Facts>,
    /// エラーがあれば status=draft
    pub vag: Value,
    pub hashes: Hashes,
}

impl Analysis {
    pub fn ok(&self) -> bool {
        !self.diags.has_errors()
    }
    pub fn unresolved(&self) -> &[Unresolved] {
        &self.intents.unresolved
    }
}

/// ソースハッシュ: [{path, sha256(content)}](パス順)の正規 JSON の SHA-256。
pub fn source_hash(p: &Project) -> String {
    let mut entries: Vec<(String, String)> = p.files.values().map(|(path, c)| (path.clone(), sha256_hex(c))).collect();
    entries.sort();
    let v = Value::Array(entries.into_iter().map(|(p, h)| serde_json::json!({"path": p, "sha256": h})).collect());
    hash_value(&v)
}

/// 解析パイプライン全体。AI・ネットワーク・時刻に依存しない純粋関数。
pub fn analyze(p: &Project, approvals: &ApprovalsFile) -> Analysis {
    let mut diags = Diagnostics::new();
    let file_names: BTreeMap<FileId, String> = p.files.iter().map(|(id, (path, _))| (*id, path.clone())).collect();

    // 1. 字句・構文
    let mut asts = Vec::new();
    for (id, (_, content)) in &p.files {
        let (ast, d) = kotonoha_parser::parse_file(content, *id);
        diags.extend(d);
        asts.push(ast);
    }
    let refs: Vec<&SourceFile> = asts.iter().collect();
    // 2. 宣言収集・型
    let (env, d) = kotonoha_types::collect_env(&refs);
    diags.extend(d);
    let mut facts = BTreeMap::new();
    for f in &refs {
        for it in &f.items {
            if let Decl::Action(a) = &it.decl {
                let (fa, d) = kotonoha_types::check_action(&env, a);
                diags.extend(d);
                facts.insert(a.name.name.clone(), fa);
            }
        }
    }
    // 3. 知識
    let (registry, d) = kotonoha_knowledge::build_registry(&refs, &env);
    diags.extend(d);
    // 4. 承認記録 → 意図
    diags.extend(approvals.verify_integrity());
    let approved: Vec<ApprovedInterpretation> = approvals
        .records
        .iter()
        .filter(|r| r.compute_hash() == r.record_hash)
        .map(|r| ApprovedInterpretation {
            intent_id: r.intent_id.clone(),
            sentence_hash: r.sentence_hash.clone(),
            canonical: r.canonical.clone(),
            approval_hash: r.record_hash.clone(),
        })
        .collect();
    let (intents, d) = kotonoha_intent::resolve_intents(&refs, &env, &registry, &approved);
    diags.extend(d);
    for r in &approvals.records {
        if !intents.used_approvals.contains(&r.intent_id) {
            diags.push(
                Diagnostic::warning(
                    Code::APP001,
                    Span::default(),
                    format!("承認記録 `{}`「{}」は現在のソースで使われていません", r.intent_id, r.sentence),
                )
                .hint("文が変更・削除された可能性があります。不要なら記録を削除してください"),
            );
        }
    }
    // 5. 効果・権限
    let (validation, d) = kotonoha_validator::validate(&env, &facts, &intents);
    diags.extend(d);

    // ファイル名を診断に付与
    for d in diags.items.iter_mut() {
        if d.file.is_empty() {
            if let Some(n) = file_names.get(&d.span.file) {
                if d.span.line > 0 {
                    d.file = n.clone();
                }
            }
        }
    }
    // 6. VAG
    let status = if diags.has_errors() { Status::Draft } else { Status::Validated };
    let mut vag = build_vag(
        &GraphInput {
            env: &env,
            registry: &registry,
            intents: &intents,
            validation: &validation,
            files: &file_names,
        },
        status,
    );
    let problems = check_integrity(&vag);
    if !problems.is_empty() {
        for pr in problems {
            diags.push(Diagnostic::error(Code::INT001, Span::default(), format!("VAG の参照整合性エラー: {pr}")));
        }
        vag["status"] = Value::String("draft".into());
    }
    diags.sort();
    let hashes = Hashes {
        source: source_hash(p),
        knowledge: hash_value(&vag["knowledge"]),
        schema: hash_value(&vag["schema"]),
        graph: hash_value(&vag),
    };
    Analysis {
        diags,
        asts,
        env,
        registry,
        intents,
        validation,
        facts,
        vag,
        hashes,
    }
}

/// 現在の入力から期待されるロック内容を計算する。
pub fn compute_lock(p: &Project, a: &Analysis, approvals: &ApprovalsFile) -> LockFile {
    let mut interp: Vec<LockedInterpretation> = approvals
        .records
        .iter()
        .filter(|r| a.intents.used_approvals.contains(&r.intent_id))
        .map(|r| LockedInterpretation {
            intent_id: r.intent_id.clone(),
            sentence_hash: r.sentence_hash.clone(),
            canonical: r.canonical.clone(),
            record_hash: r.record_hash.clone(),
        })
        .collect();
    interp.sort_by(|x, y| x.intent_id.cmp(&y.intent_id));
    let mut targets = p.config.targets.clone();
    targets.sort();
    LockFile {
        lock_version: LOCK_VERSION.into(),
        compiler_version: COMPILER_VERSION.into(),
        source_hash: a.hashes.source.clone(),
        knowledge_hash: a.hashes.knowledge.clone(),
        schema_hash: a.hashes.schema.clone(),
        graph_hash: a.hashes.graph.clone(),
        targets,
        approved_interpretations: interp,
        approval_reference: ApprovalReference {
            kind: LocalUnsigned.kind().into(),
            records_hash: approvals.records_hash(),
        },
    }
}

/// `--locked` の検証: ロックの存在・一致・承認の真正性。
pub fn verify_lock(p: &Project, a: &Analysis, approvals: &ApprovalsFile, lock_text: Option<&str>) -> Vec<Diagnostic> {
    let Some(text) = lock_text else {
        return vec![
            Diagnostic::error(Code::LOCK001, Span::default(), format!("{LOCK_FILE} がありません")).hint("koto approve --write-lock でロックを作成してください")
        ];
    };
    let lock = match LockFile::parse(text) {
        Ok(l) => l,
        Err(d) => return vec![d],
    };
    let current = compute_lock(p, a, approvals);
    let mut d = lock.compare(&current);
    if d.is_empty() {
        if let Authenticity::Unverified { reason } = LocalUnsigned.verify(approvals) {
            d.push(
                Diagnostic::error(Code::APP001, Span::default(), "承認済み解釈を含むため本番(--locked)ビルドできません")
                    .hint(reason)
                    .hint("署名基盤(ADR-007)の決定後に検証器を実装します"),
            );
        }
    }
    d
}

/// ターゲット生成の結果。
pub struct BuildOutput {
    pub files: Vec<(String, String)>,
}

/// ターゲット対応検査と(対応済み部分の)生成。
pub fn build_target(p: &Project, a: &Analysis, target: &str) -> Result<BuildOutput, Vec<Diagnostic>> {
    let Some(kind) = kotonoha_codegen::TargetKind::parse(target) else {
        return Err(vec![Diagnostic::error(
            Code::TGT001,
            Span::default(),
            format!("未知のターゲット `{target}` です"),
        )]);
    };
    if !p.config.targets.iter().any(|t| t == target) {
        return Err(vec![Diagnostic::error(
            Code::TGT001,
            Span::default(),
            format!("ターゲット `{target}` は {PROJECT_FILE} の targets にありません"),
        )]);
    }
    let t = kotonoha_codegen::target_for(kind);
    let unsupported = kotonoha_codegen::check_support(&t, &a.vag);
    if !unsupported.is_empty() {
        return Err(unsupported);
    }
    let ir = kotonoha_codegen::appir::build_ir(&kotonoha_codegen::appir::IrInput {
        env: &a.env,
        registry: &a.registry,
        intents: &a.intents,
        validation: &a.validation,
        asts: &a.asts,
    });
    let generated = match kind {
        kotonoha_codegen::TargetKind::Backend => kotonoha_codegen::backend::generate(&ir, &a.vag)?,
        kotonoha_codegen::TargetKind::Web => kotonoha_codegen::web::generate(&ir)?,
        kotonoha_codegen::TargetKind::Ios => kotonoha_codegen::ios::generate(&ir)?,
        kotonoha_codegen::TargetKind::Android => kotonoha_codegen::android::generate(&ir)?,
    };
    let mut files: Vec<(String, String)> = generated.into_iter().map(|f| (f.path, f.content)).collect();
    if kind == kotonoha_codegen::TargetKind::Backend {
        for (name, k) in &a.registry.entries {
            let Knowledge::Ranking(spec) = k;
            for (dialect, ext) in [
                (kotonoha_codegen::sql::Dialect::Sqlite, "sqlite"),
                (kotonoha_codegen::sql::Dialect::Postgres, "postgres"),
            ] {
                match kotonoha_codegen::sql::lower_ranking(spec, dialect) {
                    Ok(q) => files.push((format!("queries/{name}.{ext}.sql"), q.sql + "\n")),
                    Err(d) => return Err(vec![d]),
                }
            }
        }
    }
    Ok(BuildOutput { files })
}

/// 候補の各解釈を、承認されたと仮定して再解析し、意図が解決し新たなエラーが出ないものだけ残す。
pub fn filter_candidate(p: &Project, approvals: &ApprovalsFile, cand: &mut kotonoha_resolver::Candidate, u: &Unresolved) -> Vec<String> {
    let mut rejected = Vec::new();
    cand.interpretations.retain(|it| {
        let mut trial = approvals.clone();
        trial.upsert(kotonoha_lock::ApprovalRecord::new(
            &u.intent_id,
            &u.sentence,
            &u.sentence_hash,
            &it.canonical,
            "(検証用)",
            "1970-01-01T00:00:00Z",
            "",
        ));
        // 自動モードでも検証は対象の intent に限定する
        let a = analyze(p, &trial);
        let still = a.unresolved().iter().any(|x| x.intent_id == u.intent_id);
        let app_err = a.diags.errors().any(|d| d.code == Code::APP001);
        if still || app_err {
            rejected.push(format!("「{}」は既知の文型として成立しないため除外しました", it.canonical));
            false
        } else {
            true
        }
    });
    rejected
}

/// テキスト正規化(koto fmt): BOM 除去・CRLF→LF・NFC・行末空白除去・末尾改行。
pub fn normalize_text(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    let s = s.replace("\r\n", "\n");
    let s: String = s.nfc().collect();
    let mut out: String = s.lines().map(|l| l.trim_end_matches(' ')).collect::<Vec<_>>().join("\n");
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// RFC 3339(UTC)の現在時刻。承認記録にだけ使う。
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // civil_from_days(Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests;

/// `koto ast` 用: 1 ファイルを解析して AST を JSON にする。
pub fn parse_for_cli(src: &str) -> (Value, Vec<Diagnostic>) {
    let (ast, d) = kotonoha_parser::parse_file(src, FileId(0));
    (serde_json::to_value(&ast).expect("AST は直列化できる"), d)
}

// ---------------------------------------------------------------------------
// 影響分析(FR-016 / AT-08)
// ---------------------------------------------------------------------------

/// 影響を受ける VAG ノードと理由。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Impact {
    pub node: String,
    pub reason: String,
}

fn node_mentions(n: &Value, target: &str, field: Option<&str>) -> Option<String> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let caps: Vec<String> = n["capabilities"].as_array().into_iter().flatten().map(s).collect();
    let d = &n["detail"];
    if let Some(f) = field {
        let fields: Vec<String> = d["fields"].as_array().into_iter().flatten().map(s).collect();
        if d["data"] == target && (fields.iter().any(|x| x == f) || d["ui"] == "list") {
            return Some(format!("{target}.{f} を表示"));
        }
        // 動作の本体のフィールド参照は VAG に記録しないため、資源を使う動作は保守的に含める
        if n["kind"] == "action" && caps.iter().any(|c| c.split('.').next() == Some(target)) {
            return Some(format!("{target} を使う(フィールド単位の参照は保守的に判定)"));
        }
        return None;
    }
    if caps.iter().any(|c| c.split('.').next() == Some(target)) {
        return Some(format!("{target} の Capability を使う"));
    }
    if d["data"] == target {
        return Some(format!("{target} を表示・選択"));
    }
    if n["knowledge_refs"].as_array().into_iter().flatten().any(|k| k == target) {
        return Some(format!("知識 {target} を参照"));
    }
    if n["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(n["outputs"].as_array().into_iter().flatten())
        .any(|t| t.as_str().is_some_and(|t| t.contains(target)))
    {
        return Some(format!("入出力の型に {target} を含む"));
    }
    if n["id"] == format!("action.{target}") || n["id"] == format!("screen.{target}") {
        return Some("変更対象そのもの".into());
    }
    None
}

/// 名前(データ / データ.フィールド / 知識 / 動作 / 画面 / 列挙 / 型)の変更が影響するノードを列挙する。
/// 直接参照するノードに加え、それを含む画面・呼び出す/遷移してくるノードを辿る。
pub fn impact_of(vag: &Value, name: &str) -> Vec<Impact> {
    let (target, field) = match name.split_once('.') {
        Some((d, f)) => (d, Some(f)),
        None => (name, None),
    };
    let nodes = vag["nodes"].as_array().cloned().unwrap_or_default();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    // 知識の対象データが変わると知識も変わる
    let mut targets = vec![target.to_string()];
    for k in vag["knowledge"].as_array().into_iter().flatten() {
        let uses_field = field.is_none_or(|f| {
            [&k["unit"], &k["time_field"], &k["metric"]["field"]].iter().any(|x| *x == f) || k["filter"].as_str().is_some_and(|x| x.contains(f))
        });
        if k["source"] == target && uses_field {
            targets.push(k["name"].as_str().unwrap_or("").to_string());
        }
    }
    // 動作がアダプタ経由でデータを使う場合は capabilities に現れる
    for n in &nodes {
        for t in &targets {
            let f = if t == target { field } else { None };
            if let Some(r) = node_mentions(n, t, f) {
                out.entry(n["id"].as_str().unwrap_or("").to_string()).or_insert(r);
            }
        }
    }
    // 上流へ辿る: contains(画面 ← 部品)/ invokes(呼出元 ← 動作)/ navigates(遷移元 ← 画面)
    loop {
        let mut added = false;
        for e in vag["edges"].as_array().into_iter().flatten() {
            let (from, to) = (e["from"].as_str().unwrap_or(""), e["to"].as_str().unwrap_or(""));
            if out.contains_key(to) && !out.contains_key(from) {
                out.insert(
                    from.to_string(),
                    format!(
                        "{to} を{}",
                        match e["kind"].as_str() {
                            Some("contains") => "含む",
                            Some("invokes") => "呼び出す",
                            _ => "遷移先に持つ",
                        }
                    ),
                );
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    out.into_iter().map(|(node, reason)| Impact { node, reason }).collect()
}

/// 2 つの VAG の差分から影響を列挙する(意味定義の変更 → 影響箇所)。
pub fn impact_diff(base: &Value, current: &Value) -> Vec<Impact> {
    let strip = |n: &Value| {
        let mut n = n.clone();
        if let Some(o) = n.as_object_mut() {
            o.remove("source_span");
        }
        kotonoha_graph::canonical_json(&n)
    };
    let index = |v: &Value| -> BTreeMap<String, String> {
        v["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| (n["id"].as_str().unwrap_or("").to_string(), strip(n)))
            .collect()
    };
    let (a, b) = (index(base), index(current));
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (id, n) in &b {
        match a.get(id) {
            None => {
                out.insert(id.clone(), "追加されたノード".into());
            }
            Some(m) if m != n => {
                out.insert(id.clone(), "定義が変わったノード".into());
            }
            _ => {}
        }
    }
    for id in a.keys().filter(|k| !b.contains_key(*k)) {
        out.insert(id.clone(), "削除されたノード".into());
    }
    // 知識・データ定義の変更 → その名前の影響
    let named = |v: &Value, sect: &str, key: &str| -> BTreeMap<String, String> {
        let arr = if sect.is_empty() { &v[key] } else { &v[sect][key] };
        arr.as_array()
            .into_iter()
            .flatten()
            .map(|x| (x["name"].as_str().unwrap_or("").to_string(), kotonoha_graph::canonical_json(x)))
            .collect()
    };
    for (sect, key) in [
        ("", "knowledge"),
        ("schema", "data"),
        ("schema", "enums"),
        ("schema", "aliases"),
        ("schema", "adapters"),
    ] {
        let (x, y) = (named(base, sect, key), named(current, sect, key));
        for name in x.keys().chain(y.keys()).collect::<std::collections::BTreeSet<_>>() {
            if x.get(name) != y.get(name) {
                for i in impact_of(current, name).into_iter().chain(impact_of(base, name)) {
                    out.entry(i.node).or_insert(format!("{name} の定義変更: {}", i.reason));
                }
            }
        }
    }
    out.into_iter().map(|(node, reason)| Impact { node, reason }).collect()
}
