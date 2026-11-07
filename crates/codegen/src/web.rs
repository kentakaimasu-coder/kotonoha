//! Web 生成(React 19 SSR + esbuild。backend の API を同一オリジンのプロキシ経由で使う)。docs/architecture.md §6。

use crate::appir::{api_contract, check_ir, wire_type, AppIr, Component};
use crate::ts::{js_str, mangle};
use crate::GeneratedFile;
use kotonoha_ast::{BinOp, Expr, ExprKind};
use kotonoha_diag::{Code, Diagnostic};
use kotonoha_types::Type;
use serde_json::{json, Value};

pub const UI_TS: &str = include_str!("../../../runtime/web/ui.ts");
pub const SERVER_TS: &str = include_str!("../../../runtime/web/server.ts");
pub const APP_CSS: &str = include_str!("../../../runtime/web/app.css");

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
createWebServer(webApp, {
  backendUrl: process.env.KOTONOHA_BACKEND_URL ?? "http://127.0.0.1:8787",
  root,
  secureCookie: process.env.KOTONOHA_COOKIE_SECURE === "1",
  tz: process.env.KOTONOHA_DISPLAY_TZ ?? "Asia/Tokyo",
  oidc: oidcFromEnv(process.env),
  // 開発用のトークン貼り付けログイン。OIDC を設定したら既定で無効(KOTONOHA_DEV_LOGIN=1 で有効)
  devLogin: process.env.KOTONOHA_DEV_LOGIN ? process.env.KOTONOHA_DEV_LOGIN === "1" : !process.env.KOTONOHA_OIDC_ISSUER,
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

pub fn generate(ir: &AppIr) -> Result<Vec<GeneratedFile>, Vec<Diagnostic>> {
    let mut errs = check_ir(ir);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut g = String::new();
    g.push_str("// このファイルは koto build --target web が生成しました。手で編集しないでください(正本は KOTONOHA ソース)。\n");
    g.push_str("import { h, Page, DataList, Detail, ActionButton, ActionForm, screenHref, type ScreenDef, type ScreenProps } from \"./runtime/ui.ts\";\n");
    g.push_str("import type { WebApp } from \"./runtime/server.ts\";\n\n");

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
                Component::List { data, fields, link_to } => {
                    let key = format!("list:{data}");
                    if !loaders.iter().any(|l| l["key"] == key) {
                        loaders.push(json!({"key": key, "segments": ["api", "data", data], "withId": false}));
                    }
                    let fs: Vec<Value> = fields.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    let link = match link_to {
                        Some(t) => format!(", link: (row: any) => screenHref({}, row[\"ID\"])", js_str(t)),
                        None => String::new(),
                    };
                    body.push(format!(
                        "h(DataList, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz, caption: {}{link} }})",
                        js_str(&key),
                        serde_json::to_string(&fs).unwrap(),
                        js_str(&format!("{data}の一覧"))
                    ));
                }
                Component::Detail { data, fields } => {
                    let fs: Vec<Value> = fields.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    body.push(format!(
                        "h(Detail, {{ key: {ci}, r: p.data[{}], fields: {}, tz: p.tz }})",
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
                Component::Button { label, action, args } => {
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
                    body.push(format!(
                        "h(ActionButton, {{ key: {ci}, label: {}, action: {}, reload: p.reload, args: () => {{ {cur}return {{ {} }}; }} }})",
                        js_str(label),
                        js_str(action),
                        pairs.join(", ")
                    ));
                }
                Component::Form { action, params } => {
                    let fs: Vec<Value> = params.iter().map(|(n, t)| field_desc(ir, n, t)).collect();
                    body.push(format!(
                        "h(ActionForm, {{ key: {ci}, action: {}, fields: {}, reload: p.reload }})",
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
        "export const screens: Record<string, ScreenDef> = {{\n{}\n}};\n\nexport const webApp: WebApp = {{ appName: {}, entry: {}, nav: {}, screens }};\n",
        defs.join("\n"),
        js_str(&ir.name),
        ir.entry_screen.as_ref().map(|e| js_str(e)).unwrap_or_else(|| "null".into()),
        serde_json::to_string(&nav).unwrap()
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
    Ok(vec![
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
        GeneratedFile::text("assets/app.css", APP_CSS.to_string()),
        GeneratedFile::text("src/screens.generated.ts", g),
        GeneratedFile::text("src/server-main.ts", SERVER_MAIN_TS.to_string()),
        GeneratedFile::text("src/client.ts", CLIENT_TS.to_string()),
        GeneratedFile::text("src/runtime/ui.ts", UI_TS.to_string()),
        GeneratedFile::text("src/runtime/server.ts", SERVER_TS.to_string()),
    ])
}
