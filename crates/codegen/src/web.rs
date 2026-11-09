//! Web 生成(React 19 SSR + esbuild。backend の API を同一オリジンのプロキシ経由で使う)。docs/architecture.md §6。

use crate::appir::{api_contract, check_ir, wire_type, AppIr, Component};
use crate::ts::{js_str, mangle};
use crate::GeneratedFile;
use kotonoha_ast::{BinOp, Expr, ExprKind};
use kotonoha_diag::{Code, Diagnostic, Span};
use kotonoha_types::Type;
use serde_json::{json, Value};

pub const UI_TS: &str = include_str!("../../../runtime/web/ui.ts");
pub const SERVER_TS: &str = include_str!("../../../runtime/web/server.ts");
pub const APP_CSS: &str = include_str!("../../../runtime/web/app.css");

/// テーマ色の名前(ADR-0131)。どれも白い文字とのコントラストが 4.5 以上(ボタンの文字が読める)
pub const THEME_COLORS: &[(&str, &str)] = &[
    ("藍", "#1e50a2"),
    ("紺", "#223a70"),
    ("青", "#0b5cad"),
    ("緑", "#1d6b45"),
    ("深緑", "#14532d"),
    ("朱", "#b5341f"),
    ("紅", "#b0284a"),
    ("紫", "#6a3d9a"),
    ("橙", "#a64b00"),
    ("茶", "#7a4b2a"),
    ("墨", "#2f3237"),
];

fn channel(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 白とのコントラスト比(WCAG 2)
pub fn contrast_with_white(hex: &str) -> f64 {
    let v = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    let l = 0.2126 * channel(v(1)) + 0.7152 * channel(v(3)) + 0.0722 * channel(v(5));
    1.05 / (l + 0.05)
}

/// theme.color を #rrggbb にする。ボタンの白い文字が読めない明るい色は断る
pub fn theme_color(c: &str) -> Result<String, String> {
    let c = c.trim();
    if let Some((_, hex)) = THEME_COLORS.iter().find(|(n, _)| *n == c) {
        return Ok(hex.to_string());
    }
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit());
    if !ok {
        return Err(format!("「{c}」は色の名前か #RRGGBB(例: #1e50a2)で書きます"));
    }
    let hex = c.to_ascii_lowercase();
    let r = contrast_with_white(&hex);
    if r < 4.5 {
        return Err(format!(
            "{hex} は明るすぎて、ボタンの白い文字が読みにくくなります(コントラスト {r:.1}。4.5 以上が必要)"
        ));
    }
    Ok(hex)
}

/// 依存バージョンは固定(SEC-07)。更新は意図した変更としてのみ行う。
pub const REACT_VERSION: &str = "19.2.0";
pub const ESBUILD_VERSION: &str = "0.25.10";

const SERVER_MAIN_TS: &str = r#"// 生成物: KOTONOHA web の起動点
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createWebServer, oidcFromEnv } from "./runtime/server.ts";
import { webApp } from "./screens.generated.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const port = Number(process.env.PORT ?? 3000);
const host = process.env.KOTONOHA_HOST ?? "127.0.0.1";
// koto dev から起動されたときは、koto が終わったら一緒に終わる(サーバーが残り続けないように)
const parent = Number(process.env.KOTONOHA_EXIT_WITH_PARENT ?? 0);
if (parent > 0) {
  setInterval(() => {
    // 親が止まると、親は別のプロセスに付け替わる(ゾンビのままでも kill(0) は成功するため、親の番号も見る)
    if (process.ppid !== parent) process.exit(0);
    try {
      process.kill(parent, 0);
    } catch {
      process.exit(0);
    }
  }, 500).unref();
}
createWebServer(webApp, {
  backendUrl: process.env.KOTONOHA_BACKEND_URL ?? "http://127.0.0.1:8787",
  root,
  secureCookie: process.env.KOTONOHA_COOKIE_SECURE === "1",
  tz: process.env.KOTONOHA_DISPLAY_TZ ?? "Asia/Tokyo",
  oidc: oidcFromEnv(process.env),
  // 開発用のトークン貼り付けログイン。OIDC を設定したら既定で無効(KOTONOHA_DEV_LOGIN=1 で有効)
  devLogin: process.env.KOTONOHA_DEV_LOGIN ? process.env.KOTONOHA_DEV_LOGIN === "1" : !process.env.KOTONOHA_OIDC_ISSUER,
  // koto dev が設定する、開発用のワンクリックログイン(本番モードでは起動を拒否する)
  devQuick: process.env.KOTONOHA_DEV_JWT_SECRET ? { secret: process.env.KOTONOHA_DEV_JWT_SECRET, caps: (process.env.KOTONOHA_DEV_CAPS ?? "").split(",").filter((c) => c), roles: JSON.parse(process.env.KOTONOHA_DEV_ROLES ?? "{}") } : null,
}).listen(port, host, () => {
  process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), level: "info", msg: `web listening on http://${host}:${port}` }) + "\n");
});
"#;

const CLIENT_TS: &str = r#"// 生成物: ブラウザでの hydration
import { hydrateRoot } from "react-dom/client";
import { App, h, type Init } from "./runtime/ui.ts";
import { webApp } from "./screens.generated.ts";

const el = document.getElementById("__init__");
if (el) {
  const init = JSON.parse(el.textContent ?? "{}") as Init;
  hydrateRoot(document.getElementById("root") as HTMLElement, h(App, { init, screens: webApp.screens }));
}
"#;

const BUILD_TS: &str = r#"// 生成物: クライアントバンドルの作成
import { build } from "esbuild";
await build({
  entryPoints: ["src/client.ts"],
  bundle: true,
  format: "esm",
  minify: true,
  outfile: "dist/client.js",
  define: { "process.env.NODE_ENV": '"production"' },
  logLevel: "warning",
});
"#;

/// フィールド記述(UI 部品が使う)。列挙は選択肢を埋め込む。
fn field_desc(ir: &AppIr, n: &str, t: &Type) -> Value {
    let mut w = wire_type(t).unwrap_or(Value::Null);
    fill_variants(ir, &mut w);
    json!({"name": n, "type": w})
}

fn fill_variants(ir: &AppIr, w: &mut Value) {
    if w["kind"] == "enum" {
        let name = w["name"].as_str().unwrap_or("").to_string();
        w["variants"] = json!(ir.enums.get(&name).cloned().unwrap_or_default());
    }
    if let Some(of) = w.get_mut("of") {
        fill_variants(ir, of);
    }
}

/// 画面上の引数式(現在のエンティティ・リテラル・四則の一部)→ JS。
pub fn screen_arg_js(e: &Expr, input: Option<&str>, errs: &mut Vec<Diagnostic>) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Str(s) => js_str(s),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::None => "null".into(),
        ExprKind::Ident(n) if Some(n.as_str()) == input => "cur".into(),
        ExprKind::Field(b, f) => format!("{}[{}]", screen_arg_js(b, input, errs), js_str(&f.name)),
        ExprKind::Binary {
            op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul),
            lhs,
            rhs,
        } => {
            format!("({} {} {})", screen_arg_js(lhs, input, errs), op.symbol(), screen_arg_js(rhs, input, errs))
        }
        _ => {
            errs.push(Diagnostic::error(
                Code::TGT001,
                e.span,
                "この画面引数式はクライアント生成に未対応です(リテラル・現在のエンティティ・+ - * のみ)",
            ));
            "undefined".into()
        }
    }
}

/// 動作に必要な権限(表示の出し分け用。認可は backend)
fn need_js(ir: &AppIr, action: &str) -> String {
    match ir.endpoints.iter().find(|e| e.id == format!("action.{action}")) {
        Some(e) => {
            let a = e.access_json();
            format!(
                ", need: {}, caps: p.caps",
                serde_json::to_string(&json!({"caps": a["capabilities"], "grant": a["grant"]})).unwrap()
            )
        }
        None => String::new(),
    }
}

/// 関連データの名前表示: フィールド「XID」(識別子)で、データ X に一覧 API があり文字列の項目を持つなら、
/// X の一覧を読み込んで最初の文字列項目(名前など)で表示する。権限が無ければ ID のまま(API が判断する)
fn refs_js(ir: &AppIr, fields: &[(String, Type)], loaders: &mut Vec<Value>) -> String {
    let mut parts = Vec::new();
    for (n, t) in fields {
        let Some(x) = n.strip_suffix("ID") else { continue };
        if !matches!(t, Type::Id) || x.is_empty() {
            continue;
        }
        let Some(ent) = ir.entities.iter().find(|e| e.name == x) else { continue };
        // 選ぶ欄の表示: 最初の文字列の項目。日付・日時の項目があれば添える(同じ文字の行を見分けられるように)。
        // 文字列の項目が無ければ日付・日時だけで出す
        let base = |t: &Type| match t {
            Type::Option(x) => x.as_ref().clone(),
            t => t.clone(),
        };
        let text = ent
            .fields
            .iter()
            .find(|(f, ft)| f != "ID" && matches!(base(ft), Type::Str))
            .map(|(f, _)| f.clone());
        let when = ent
            .fields
            .iter()
            .find(|(_, ft)| matches!(base(ft), Type::Date | Type::DateTime))
            .map(|(f, _)| f.clone());
        let (label, extra) = match (text, when) {
            (Some(t), w) => (t, w),
            (None, Some(w)) => (w, None),
            (None, None) => continue,
        };
        let label = &label;
        if !ir.endpoints.iter().any(|e| e.id == format!("data.list.{x}")) {
            continue;
        }
        let key = format!("list:{x}");
        if !loaders.iter().any(|l| l["key"] == key) {
            loaders.push(json!({"key": key, "segments": ["api", "data", x], "withId": false}));
        }
        parts.push(format!(
            "{}: {{ r: p.data[{}], label: {}, title: {}{} }}",
            js_str(n),
            js_str(&key),
            js_str(label),
            js_str(x),
            extra.map(|e| format!(", extra: {}", js_str(&e))).unwrap_or_default()
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(", refs: {{ {} }}", parts.join(", "))
    }
}

pub fn generate(ir: &AppIr) -> Result<Vec<GeneratedFile>, Vec<Diagnostic>> {
    let mut errs = check_ir(ir);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut g = String::new();
    g.push_str("// このファイルは koto build --target web が生成しました。手で編集しないでください(正本は KOTONOHA ソース)。\n");
    g.push_str("import { h, Page, DataList, Detail, ActionButton, ActionForm, AnalysisQuery, ExtBoundary, variantOf, screenHref, type ScreenDef, type ScreenProps } from \"./runtime/ui.ts\";\n");
    g.push_str("import type { WebApp } from \"./runtime/server.ts\";\n");
    // 拡張部品(画面で使うものだけ。web 実装が無ければ TGT001)
    let mut used: Vec<String> = Vec::new();
    for s in &ir.screens {
        for c in &s.components {
            if let Component::Ext { component, experiment, .. } = c {
                let names = experiment.as_ref().map(|e| e.variants.clone()).unwrap_or_else(|| vec![component.clone()]);
                for n in names {
                    if !used.contains(&n) {
                        used.push(n);
                    }
                }
            }
        }
    }
    let mut ext_files: Vec<GeneratedFile> = Vec::new();
    for n in &used {
        let Some(c) = ir.ext.component(n) else { continue };
        let Some(f) = &c.web else {
            errs.push(Diagnostic::error(
                Code::TGT001,
                Span::default(),
                format!("部品 `{n}` に web の実装(web = \"….ts\")がありません"),
            ));
            continue;
        };
        g.push_str(&format!("import * as {} from {};\n", mangle("ce_", n), js_str(&format!("./ext/{}", f.path))));
        if !ext_files.iter().any(|x| x.path == format!("src/ext/{}", f.path)) {
            ext_files.push(GeneratedFile::text(&format!("src/ext/{}", f.path), f.content.clone()));
        }
    }
    g.push('\n');

    let mut defs = Vec::new();
    for s in &ir.screens {
        let fname = mangle("Screen_", &s.name);
        let mut loaders: Vec<Value> = Vec::new();
        let mut body: Vec<String> = Vec::new();
        if let Some(d) = &s.input {
            loaders.push(json!({"key": format!("get:{d}"), "segments": ["api", "data", d], "withId": true}));
        }
        for (ci, c) in s.components.iter().enumerate() {
            match c {
                Component::List {
                    data,
                    fields,
                    link_to,
                    query,
                    caption,
                } => {
                    let key = if query.is_empty() {
                        format!("list:{data}")
                    } else {
                        format!("list:{data}?{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"))
                    };
                    if !loaders.iter().any(|l| l["key"] == key) {
                        let mut l = json!({"key": key, "segments": ["api", "data", data], "withId": false});
                        if !query.is_empty() {
                            l["query"] = Value::Object(query.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect());
                        }
                        loaders.push(l);
                    }
                    let fs: Vec<Value> = fields.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let link = match link_to {
                        Some(t) => format!(", link: (row: any) => screenHref({}, row[\"ID\"])", js_str(t)),
                        None => String::new(),
                    };
                    let refs = refs_js(ir, fields, &mut loaders);
                    body.push(format!(
                        "h(DataList, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz, caption: {}{link}{refs} }})",
                        js_str(&key),
                        serde_json::to_string(&fs).unwrap(),
                        js_str(caption)
                    ));
                }
                Component::Detail { data, fields } => {
                    let fs: Vec<Value> = fields.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let refs = refs_js(ir, fields, &mut loaders);
                    body.push(format!(
                        "h(Detail, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz{refs} }})",
                        js_str(&format!("get:{data}")),
                        serde_json::to_string(&fs).unwrap()
                    ));
                }
                Component::Query { knowledge, limit, columns } => {
                    let key = format!("query:{knowledge}:{limit}");
                    loaders.push(json!({"key": key, "segments": ["api", "queries", knowledge], "withId": false, "query": {"limit": limit.to_string()}}));
                    let fs: Vec<Value> = columns.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    body.push(format!(
                        "h(DataList, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz, caption: {} }})",
                        js_str(&key),
                        serde_json::to_string(&fs).unwrap(),
                        js_str(&format!("{knowledge}(上位{limit}件)"))
                    ));
                }
                Component::Button {
                    label,
                    action,
                    args,
                    experiment,
                } => {
                    let Some(a) = ir.actions.iter().find(|x| &x.name == action) else { continue };
                    let pairs: Vec<String> = a
                        .params
                        .iter()
                        .zip(args)
                        .map(|((n, _), e)| format!("{}: {}", js_str(n), screen_arg_js(e, s.input.as_deref(), &mut errs)))
                        .collect();
                    let cur = match &s.input {
                        Some(d) => format!(
                            "const r = p.data[{}]; if (!r?.ok) return null; const cur: any = r.body; ",
                            js_str(&format!("get:{d}"))
                        ),
                        None => String::new(),
                    };
                    let label_js = match experiment {
                        Some(x) => {
                            let key = format!("exp:{}", x.name);
                            if !loaders.iter().any(|l| l["key"] == key) {
                                loaders.push(json!({"key": key, "segments": ["api", "experiments", x.name, "assign"], "withId": false, "method": "POST"}));
                            }
                            format!("variantOf(p.data[{}], {})", js_str(&key), serde_json::to_string(&x.variants).unwrap())
                        }
                        None => js_str(label),
                    };
                    let need = need_js(ir, action);
                    body.push(format!(
                        "h(ActionButton, {{ key: {ci}, label: {}, action: {}, reload: p.reload{need}, args: () => {{ {cur}return {{ {} }}; }} }})",
                        label_js,
                        js_str(action),
                        pairs.join(", ")
                    ));
                }
                Component::Ext { component, args, experiment } => {
                    let Some(c) = ir.ext.component(component) else { continue };
                    let props: Vec<String> = c
                        .params
                        .iter()
                        .zip(args)
                        .map(|((n, _), e)| format!("{}: {}", js_str(n), screen_arg_js(e, s.input.as_deref(), &mut errs)))
                        .collect();
                    let cur = match &s.input {
                        Some(d) => format!(
                            "const r = p.data[{}]; if (!r?.ok) return null; const cur: any = r.body; ",
                            js_str(&format!("get:{d}"))
                        ),
                        None => String::new(),
                    };
                    let (comp, name_js) = match experiment {
                        Some(x) => {
                            let key = format!("exp:{}", x.name);
                            if !loaders.iter().any(|l| l["key"] == key) {
                                loaders.push(json!({"key": key, "segments": ["api", "experiments", x.name, "assign"], "withId": false, "method": "POST"}));
                            }
                            let table: Vec<String> = x
                                .variants
                                .iter()
                                .map(|v| format!("{}: ({} as any)[{}]", js_str(v), mangle("ce_", v), js_str(v)))
                                .collect();
                            (
                                format!(
                                    "({{ {} }} as Record<string, any>)[variantOf(p.data[{}], {})]",
                                    table.join(", "),
                                    js_str(&key),
                                    serde_json::to_string(&x.variants).unwrap()
                                ),
                                js_str(component),
                            )
                        }
                        None => (format!("({} as any)[{}]", mangle("ce_", component), js_str(component)), js_str(component)),
                    };
                    body.push(format!(
                        "((): any => {{ {cur}return h(ExtBoundary, {{ key: {ci}, name: {name_js} }}, h({comp}, {{ {} }})); }})()",
                        props.join(", ")
                    ));
                }
                Component::Aggregate { name, columns, caption } => {
                    let key = format!("aggregate:{name}");
                    if !loaders.iter().any(|l| l["key"] == key) {
                        loaders.push(json!({"key": key, "segments": ["api", "aggregates", name], "withId": false}));
                    }
                    let fs: Vec<Value> = columns.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let refs = refs_js(ir, columns, &mut loaders);
                    body.push(format!(
                        "h(DataList, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz, caption: {}{refs} }})",
                        js_str(&key),
                        serde_json::to_string(&fs).unwrap(),
                        js_str(caption)
                    ));
                }
                Component::Analysis { analysis, columns, params } if !params.is_empty() => {
                    // 引数のある分析: 条件を入れて「表示」を押すと結果の表を出す
                    let fs: Vec<Value> = columns.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let ps: Vec<Value> = params.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let refs = refs_js(ir, columns, &mut loaders);
                    body.push(format!(
                        "h(AnalysisQuery, {{ key: {ci}, analysis: {}, params: {}, fields: {}, tz: p.tz{refs} }})",
                        js_str(analysis),
                        serde_json::to_string(&ps).unwrap(),
                        serde_json::to_string(&fs).unwrap()
                    ));
                }
                Component::Analysis { analysis, columns, .. } => {
                    let key = format!("analysis:{analysis}");
                    if !loaders.iter().any(|l| l["key"] == key) {
                        loaders.push(json!({"key": key, "segments": ["api", "analyses", analysis], "withId": false}));
                    }
                    let fs: Vec<Value> = columns.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    body.push(format!(
                        "h(DataList, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz, caption: {} }})",
                        js_str(&key),
                        serde_json::to_string(&fs).unwrap(),
                        js_str(analysis)
                    ));
                }
                Component::Form { action, params, submit } => {
                    let fs: Vec<Value> = params.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    // 詳細画面(入力データ D がある画面)のフォームで、引数「DID」(識別子)は表示中の D の ID を自動で入れる
                    let fixed = match &s.input {
                        Some(d) => params
                            .iter()
                            .find(|(n, t)| *n == format!("{d}ID") && matches!(t, Type::Id))
                            .map(|(n, _)| {
                                format!(
                                    "(p.data[{}]?.ok ? {{ {}: p.data[{}].body[\"ID\"] }} : null)",
                                    js_str(&format!("get:{d}")),
                                    js_str(n),
                                    js_str(&format!("get:{d}"))
                                )
                            })
                            .unwrap_or_else(|| "null".into()),
                        None => "null".into(),
                    };
                    let need = need_js(ir, action);
                    // 引数「データ名ID」は、そのデータの一覧から選ぶ(名前で表示)。自動で入れる ID は除く
                    let auto = s.input.as_ref().map(|d| format!("{d}ID"));
                    let pickable: Vec<(String, Type)> = params.iter().filter(|(n, _)| Some(n) != auto.as_ref()).cloned().collect();
                    let refs = refs_js(ir, &pickable, &mut loaders);
                    let label = if submit == "送信" {
                        String::new()
                    } else {
                        format!(", submit: {}", js_str(submit))
                    };
                    body.push(format!(
                        "h(ActionForm, {{ key: {ci}, action: {}, fields: {}, reload: p.reload, tz: p.tz, fixed: {fixed}{need}{refs}{label} }})",
                        js_str(action),
                        serde_json::to_string(&fs).unwrap()
                    ));
                }
            }
        }
        g.push_str(&format!(
            "function {fname}(p: ScreenProps) {{\n  return h(Page, {{ title: {} }},\n    {}\n  );\n}}\n\n",
            js_str(&s.name),
            body.join(",\n    ")
        ));
        defs.push(format!(
            "  {}: {{ name: {}, input: {}, loaders: {}, View: {fname} }},",
            js_str(&s.name),
            js_str(&s.name),
            s.input.as_ref().map(|d| js_str(d)).unwrap_or_else(|| "null".into()),
            serde_json::to_string(&loaders).unwrap()
        ));
    }
    let nav: Vec<&str> = ir.screens.iter().filter(|s| s.input.is_none()).map(|s| s.name.as_str()).collect();
    g.push_str(&format!(
        "export const screens: Record<string, ScreenDef> = {{\n{}\n}};\n\nexport const webApp: WebApp = {{ appName: {}, entry: {}, nav: {}, screens, roles: {} }};\n",
        defs.join("\n"),
        js_str(&ir.name),
        ir.entry_screen.as_ref().map(|e| js_str(e)).unwrap_or_else(|| "null".into()),
        serde_json::to_string(&nav).unwrap(),
        serde_json::to_string(&ir.roles).unwrap()
    ));
    if !errs.is_empty() {
        return Err(errs);
    }
    let pkg = json!({
        "name": "kotonoha-web",
        "private": true,
        "type": "module",
        "engines": {"node": ">=22.18.0"},
        "scripts": {"build": "node --no-warnings build.ts", "start": "node --no-warnings src/server-main.ts"},
        "dependencies": {"react": REACT_VERSION, "react-dom": REACT_VERSION},
        "devDependencies": {"esbuild": ESBUILD_VERSION},
    });
    let readme = format!(
        "# {name} web(KOTONOHA 生成物)\n\nReact {REACT_VERSION} の SSR。API は backend をこのサーバーの `/api` 経由で使う。\n\n```bash\nnpm install\nnpm run build\nKOTONOHA_BACKEND_URL=http://127.0.0.1:8787 npm start   # http://127.0.0.1:3000\n```\n",
        name = ir.name
    );
    let mut out = vec![
        GeneratedFile::text("package.json", serde_json::to_string_pretty(&pkg).unwrap() + "\n"),
        GeneratedFile::text(
            "sbom.cdx.json",
            crate::sbom(
                &ir.name,
                "web",
                &[
                    ("npm", "react", REACT_VERSION),
                    ("npm", "react-dom", REACT_VERSION),
                    ("npm", "esbuild", ESBUILD_VERSION),
                ],
            ),
        ),
        GeneratedFile::text("README.md", readme),
        GeneratedFile::text("api.json", serde_json::to_string_pretty(&api_contract(ir)).unwrap() + "\n"),
        GeneratedFile::text("build.ts", BUILD_TS.to_string()),
        GeneratedFile::text(
            "assets/app.css",
            match &ir.theme_color {
                Some(c) => format!("{APP_CSS}\n/* kotonoha.json の theme.color */\n:root {{ --accent: {c}; }}\n"),
                None => APP_CSS.to_string(),
            },
        ),
        GeneratedFile::text("src/screens.generated.ts", g),
        GeneratedFile::text("src/server-main.ts", SERVER_MAIN_TS.to_string()),
        GeneratedFile::text("src/client.ts", CLIENT_TS.to_string()),
        GeneratedFile::text("src/runtime/ui.ts", UI_TS.to_string()),
        GeneratedFile::text("src/runtime/server.ts", SERVER_TS.to_string()),
    ];
    out.extend(ext_files);
    Ok(out)
}

#[cfg(test)]
mod theme_tests {
    use super::*;

    #[test]
    fn named_theme_colors_are_readable_with_white_text() {
        for (n, hex) in THEME_COLORS {
            assert!(contrast_with_white(hex) >= 4.5, "{n} {hex}");
            assert_eq!(theme_color(n).unwrap(), *hex);
        }
        assert_eq!(theme_color("#0B5CAD").unwrap(), "#0b5cad");
        assert!(theme_color("#ffeb3b").unwrap_err().contains("明るすぎ"));
        assert!(theme_color("ピンク").is_err());
        assert!(theme_color("#12345").is_err());
    }
}
