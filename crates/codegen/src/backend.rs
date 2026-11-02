//! Backend 生成(Node.js 22.18+ / TypeScript / node:sqlite、npm 依存なし)。docs/architecture.md §5。

use crate::appir::{api_contract, check_ir, wire_type, Access, AppIr, EndpointKind};
use crate::sql::{lower_ranking, sqlite_ddl_for, Dialect, SqlParam};
use crate::ts::{js_str, mangle, TsGen};
use crate::GeneratedFile;
use kotonoha_ast::OpKind;
use kotonoha_diag::Diagnostic;
use kotonoha_types::Type;
use serde_json::json;

pub const RT_TS: &str = include_str!("../../../runtime/backend/rt.ts");
pub const DB_TS: &str = include_str!("../../../runtime/backend/db.ts");
pub const AUTH_TS: &str = include_str!("../../../runtime/backend/auth.ts");
pub const SERVER_TS: &str = include_str!("../../../runtime/backend/server.ts");

const MAIN_TS: &str = r#"// 生成物: KOTONOHA backend の起動点
import { openDb } from "./runtime/db.ts";
import { createApp } from "./runtime/server.ts";
import { app, ddl } from "./app.generated.ts";

const dbPath = process.env.KOTONOHA_DB ?? "data/app.sqlite";
const db = openDb(dbPath, ddl);
const port = Number(process.env.PORT ?? 8787);
const host = process.env.KOTONOHA_HOST ?? "127.0.0.1";
createApp(app, db).listen(port, host, () => {
  process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), level: "info", msg: `listening on http://${host}:${port}` }) + "\n");
});
"#;

const DEV_TOKEN_TS: &str = r#"// 開発用トークン発行: node tools/dev-token.ts --sub alice --caps 注文.create,商品.read --ttl 3600
// 秘密は KOTONOHA_JWT_SECRET 環境変数から読む(生成物に秘密を含めない)。本番では外部 IdP を使う(ADR-002)。
import { parseArgs } from "node:util";
import { signToken } from "../src/runtime/auth.ts";

const { values } = parseArgs({ options: { sub: { type: "string" }, caps: { type: "string", default: "" }, ttl: { type: "string", default: "3600" } } });
const secret = process.env.KOTONOHA_JWT_SECRET ?? "";
if (secret.length < 32) {
  console.error("KOTONOHA_JWT_SECRET(32 文字以上)を設定してください");
  process.exit(2);
}
if (!values.sub) {
  console.error("--sub が必要です");
  process.exit(2);
}
const now = Math.floor(Date.now() / 1000);
const caps = values.caps.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
process.stdout.write(signToken({ sub: values.sub, caps, iat: now, exp: now + Number(values.ttl) }, secret) + "\n");
"#;

const SEED_TS: &str = r#"// 初期データ投入: node tools/seed.ts seed.json   (seed.json = { "データ名": [ { フィールド: 値, ... } ] })
// 値はワイヤ形式(API と同じ)で検証してから INSERT する。
import { readFileSync } from "node:fs";
import { openDb, q } from "../src/runtime/db.ts";
import { decodeInput } from "../src/runtime/rt.ts";
import { app, ddl } from "../src/app.generated.ts";

const file = process.argv[2];
if (!file) {
  console.error("使い方: node tools/seed.ts seed.json");
  process.exit(2);
}
const data = JSON.parse(readFileSync(file, "utf8"));
const db = openDb(process.env.KOTONOHA_DB ?? "data/app.sqlite", ddl);
db.exec("BEGIN");
for (const [table, rows] of Object.entries(data)) {
  const fields = app.schema.entities[table];
  if (!fields) throw new Error(`未知のデータ ${table}`);
  for (const [i, row] of (rows as Record<string, unknown>[]).entries()) {
    const vals = fields.map((f) => {
      const v = decodeInput(row[f.name], f.type, app.schema, `${table}[${i}].${f.name}`);
      return typeof v === "boolean" ? (v ? 1n : 0n) : v;
    });
    db.prepare(`INSERT INTO ${q(table)} (${fields.map((f) => q(f.name)).join(", ")}) VALUES (${fields.map(() => "?").join(", ")})`).run(...(vals as never[]));
  }
}
db.exec("COMMIT");
console.log("投入しました");
"#;

pub fn generate(ir: &AppIr, vag: &serde_json::Value) -> Result<Vec<GeneratedFile>, Vec<Diagnostic>> {
    let mut errs = check_ir(ir);
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut g = String::new();
    g.push_str("// このファイルは koto build --target backend が生成しました。手で編集しないでください(正本は KOTONOHA ソース)。\n");
    g.push_str("import * as rt from \"./runtime/rt.ts\";\nimport * as db from \"./runtime/db.ts\";\nimport type { Ctx } from \"./runtime/db.ts\";\nimport type { App, Route } from \"./runtime/server.ts\";\n\n");

    // スキーマ
    let mut ents = serde_json::Map::new();
    for e in &ir.entities {
        let fs: Vec<serde_json::Value> = e
            .fields
            .iter()
            .map(|(n, t)| json!({"name": n, "type": wire_type(t).unwrap_or(serde_json::Value::Null)}))
            .collect();
        ents.insert(e.name.clone(), serde_json::Value::Array(fs));
    }
    let schema = json!({"entities": ents, "enums": ir.enums});
    g.push_str(&format!(
        "export const schema: rt.Schema = {} as any;\n\n",
        serde_json::to_string_pretty(&schema).unwrap()
    ));

    // DDL
    let mut ddl = Vec::new();
    for e in &ir.entities {
        match sqlite_ddl_for(&e.name, &e.fields, &ir.enums) {
            Ok(s) => ddl.push(s),
            Err(d) => errs.push(d),
        }
    }
    g.push_str(&format!("export const ddl: string[] = {};\n\n", serde_json::to_string_pretty(&ddl).unwrap()));

    // アダプタ
    for a in &ir.adapters {
        let ps: Vec<String> = a.params.iter().map(|(n, _)| mangle("v_", n)).collect();
        let sig = format!(
            "function {}(ctx: Ctx{}{}): any",
            mangle("ad_", &a.name),
            if ps.is_empty() { "" } else { ", " },
            ps.iter().map(|p| format!("{p}: any")).collect::<Vec<_>>().join(", ")
        );
        let t = js_str(&a.target);
        let obj = || {
            format!(
                "{{ {} }}",
                a.params
                    .iter()
                    .map(|(n, _)| format!("{}: {}", js_str(n), mangle("v_", n)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let body = match a.op {
            OpKind::Get => format!("return db.adGet(ctx, {t}, {}, {});", ps[0], matches!(a.ret, Type::Option(_))),
            OpKind::List => format!("return db.adList(ctx, {t});"),
            OpKind::Create => format!(
                "return db.adCreate(ctx, {t}, {}, {});",
                obj(),
                if matches!(a.ret, Type::Data(_)) { "\"entity\"" } else { "\"id\"" }
            ),
            OpKind::Update => format!("return db.adUpdate(ctx, {t}, {});", obj()),
            OpKind::Delete => format!("return db.adDelete(ctx, {t}, {});", ps[0]),
        };
        g.push_str(&format!(
            "// アダプタ {}(操作 {}({}))\n{sig} {{\n  {body}\n}}\n\n",
            a.name,
            a.op.word(),
            a.target
        ));
    }

    // 動作
    let mut tg = TsGen::new(ir);
    for a in &ir.actions {
        g.push_str(&format!(
            "// 動作 {}  必要: [{}]\n",
            a.name,
            a.caps.iter().map(|c| c.vag()).collect::<Vec<_>>().join(", ")
        ));
        g.push_str(&tg.action(&a.name, &a.params, a.body.as_ref()));
        g.push('\n');
    }
    errs.append(&mut tg.errors);

    // ルート
    g.push_str("export const routes: Route[] = [\n");
    for ep in &ir.endpoints {
        let access = match &ep.access {
            Access::Public => json!({"public": true, "capabilities": []}),
            Access::Caps(c) => json!({"public": false, "capabilities": c.iter().map(|x| x.vag()).collect::<Vec<_>>()}),
        };
        let handler = match &ep.kind {
            EndpointKind::ListData { data } => format!(
                "(ctx) => ({{ status: 200, body: db.adList(ctx, {d}).map((e) => rt.encode(e, {{ kind: \"entity\", name: {d} }}, schema)) }})",
                d = js_str(data)
            ),
            EndpointKind::GetData { data } => format!(
                "(ctx, req) => ({{ status: 200, body: rt.encode(db.adGet(ctx, {d}, rt.decodeInput(req.id, {{ kind: \"id\" }}, schema, \"id\"), false), {{ kind: \"entity\", name: {d} }}, schema) }})",
                d = js_str(data)
            ),
            EndpointKind::Query { knowledge, limits } => {
                let spec = &ir.queries.iter().find(|q| &q.0.name == knowledge).unwrap().0;
                match lower_ranking(spec, Dialect::Sqlite) {
                    Err(d) => {
                        errs.push(d);
                        continue;
                    }
                    Ok(qr) => {
                        let n = qr.params.len();
                        let params: Vec<String> = qr
                            .params
                            .iter()
                            .enumerate()
                            .map(|(i, p)| match p {
                                SqlParam::Int(_) if i == n - 1 => "BigInt(limit)".to_string(),
                                SqlParam::Int(v) => format!("{v}n"),
                                SqlParam::Text(s) => js_str(s),
                                SqlParam::Bool(b) => if *b { "1n".into() } else { "0n".into() },
                                SqlParam::WindowStart => "base - BigInt(days) * 86400000n".into(),
                                SqlParam::WindowEnd => "base".into(),
                            })
                            .collect();
                        let row_wire = wire_type(&spec.row_type()).unwrap();
                        format!(
                            "(ctx, req) => {{\n      const allowed = {limits:?};\n      const limit = req.query.has(\"limit\") ? Number(req.query.get(\"limit\")) : allowed[allowed.length - 1];\n      if (!allowed.includes(limit)) throw new rt.HttpError(400, \"invalid_input\", `limit は ${{allowed.join(\" / \")}} のいずれかです`);\n      const base = req.query.has(\"base\") ? (rt.decodeInput(req.query.get(\"base\"), {{ kind: \"datetime\" }}, schema, \"base\") as bigint) : BigInt(Date.now());\n      const days = {days};\n      const rows = db.all(ctx, {sql}, [{params}]);\n      const w: any = {wire};\n      return {{ status: 200, body: rows.map((r) => rt.encode(r, w, schema)) }};\n    }}",
                            days = spec.window_days,
                            sql = js_str(&qr.sql),
                            params = params.join(", "),
                            wire = serde_json::to_string(&row_wire).unwrap(),
                        )
                    }
                }
            }
            EndpointKind::Action { action } => {
                let a = ir.actions.iter().find(|x| &x.name == action).unwrap();
                let args: Vec<String> = a
                    .params
                    .iter()
                    .map(|(n, t)| format!("rt.decodeInput(b[{k}], {w}, schema, {k})", k = js_str(n), w = serde_json::to_string(&wire_type(t).unwrap_or(serde_json::Value::Null)).unwrap()))
                    .collect();
                let ok_wire = match &a.ret {
                    None => json!({"kind": "unit"}),
                    Some(Type::Result(t, _)) => wire_type(t).unwrap_or(serde_json::Value::Null),
                    Some(t) => wire_type(t).unwrap_or(serde_json::Value::Null),
                };
                let keys: Vec<String> = a.params.iter().map(|(n, _)| js_str(n)).collect();
                format!(
                    "(ctx, req) => {{\n      const b = req.body as Record<string, unknown>;\n      if (b === null || typeof b !== \"object\" || Array.isArray(b)) throw new rt.HttpError(400, \"invalid_input\", \"本文はオブジェクトです\");\n      const extra = Object.keys(b).filter((k) => ![{keys}].includes(k));\n      if (extra.length > 0) throw new rt.HttpError(400, \"invalid_input\", `未知の項目: ${{extra.join(\", \")}}`);\n      const v = {f}(ctx{sep}{args});\n      return {{ status: 200, body: {{ ok: rt.encode(v, {w}, schema) }} }};\n    }}",
                    keys = keys.join(", "),
                    f = mangle("act_", action),
                    sep = if args.is_empty() { "" } else { ", " },
                    args = args.join(", "),
                    w = serde_json::to_string(&ok_wire).unwrap(),
                )
            }
        };
        g.push_str(&format!(
            "  {{\n    id: {},\n    method: {},\n    segments: {},\n    hasId: {},\n    access: {},\n    handle: {handler},\n  }},\n",
            js_str(&ep.id),
            js_str(ep.method),
            serde_json::to_string(&ep.segments).unwrap(),
            ep.has_id,
            serde_json::to_string(&access).unwrap()
        ));
    }
    g.push_str("];\n\n");
    g.push_str(&format!("export const app: App = {{ name: {}, schema, routes }};\n", js_str(&ir.name)));

    if !errs.is_empty() {
        return Err(errs);
    }
    let pkg = json!({
        "name": "kotonoha-backend",
        "private": true,
        "type": "module",
        "engines": {"node": ">=22.18.0"},
        "scripts": {"start": "node --no-warnings src/main.ts", "token": "node --no-warnings tools/dev-token.ts", "seed": "node --no-warnings tools/seed.ts"},
    });
    let readme = format!(
        "# {name} backend(KOTONOHA 生成物)\n\n依存パッケージなし。Node.js 22.18 以上。\n\n```bash\nexport KOTONOHA_JWT_SECRET=...(32 文字以上。秘密管理から注入)\nnpm run seed -- seed.json   # 任意\nnpm start                   # http://127.0.0.1:8787\n```\n\n環境変数: PORT, KOTONOHA_HOST, KOTONOHA_DB, KOTONOHA_JWT_SECRET, KOTONOHA_JWT_ISSUER, KOTONOHA_JWT_AUDIENCE\n\nAPI 契約: api.json。意味グラフ: vag.json。\n",
        name = ir.name
    );
    Ok(vec![
        GeneratedFile::text("package.json", serde_json::to_string_pretty(&pkg).unwrap() + "\n"),
        GeneratedFile::text("README.md", readme),
        GeneratedFile::text("api.json", serde_json::to_string_pretty(&api_contract(ir)).unwrap() + "\n"),
        GeneratedFile::text("vag.json", serde_json::to_string_pretty(vag).unwrap() + "\n"),
        GeneratedFile::text("schema.sql", ddl.join("\n\n") + "\n"),
        GeneratedFile::text("src/app.generated.ts", g),
        GeneratedFile::text("src/main.ts", MAIN_TS.to_string()),
        GeneratedFile::text("src/runtime/rt.ts", RT_TS.to_string()),
        GeneratedFile::text("src/runtime/db.ts", DB_TS.to_string()),
        GeneratedFile::text("src/runtime/auth.ts", AUTH_TS.to_string()),
        GeneratedFile::text("src/runtime/server.ts", SERVER_TS.to_string()),
        GeneratedFile::text("tools/dev-token.ts", DEV_TOKEN_TS.to_string()),
        GeneratedFile::text("tools/seed.ts", SEED_TS.to_string()),
    ])
}
