//! Backend 生成(Node.js 22.18+ / TypeScript / node:sqlite、npm 依存なし)。docs/architecture.md §5。

use crate::appir::{api_contract, check_ir, wire_type, AppIr, EndpointKind};
use crate::sql::{lower_ranking, pg_ddl_for, sqlite_columns, sqlite_ddl_for, Dialect, SqlParam};
use crate::ts::{js_str, mangle, TsGen};
use crate::GeneratedFile;
use kotonoha_ast::OpKind;
use kotonoha_diag::Diagnostic;
use kotonoha_types::Type;
use serde_json::json;

pub const RT_TS: &str = include_str!("../../../runtime/backend/rt.ts");
pub const DB_TS: &str = include_str!("../../../runtime/backend/db.ts");
pub const AUTH_TS: &str = include_str!("../../../runtime/backend/auth.ts");
/// PostgreSQL ドライバ(任意依存。KOTONOHA_DB=postgres://… のときだけ読み込む)。固定版(SEC-07)
pub const PG_VERSION: &str = "8.16.3";
pub const SERVER_TS: &str = include_str!("../../../runtime/backend/server.ts");

const MAIN_TS: &str = r#"// 生成物: KOTONOHA backend の起動点
import { openDb } from "./runtime/db.ts";
import { createApp } from "./runtime/server.ts";
import { app, tables } from "./app.generated.ts";

const dbPath = process.env.KOTONOHA_DB ?? "data/app.sqlite";
const db = await openDb(dbPath, tables);
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
import { openDb, q, toSql } from "../src/runtime/db.ts";
import { decodeInput } from "../src/runtime/rt.ts";
import { app, tables } from "../src/app.generated.ts";

const file = process.argv[2];
if (!file) {
  console.error("使い方: node tools/seed.ts seed.json");
  process.exit(2);
}
const data = JSON.parse(readFileSync(file, "utf8"));
const db = await openDb(process.env.KOTONOHA_DB ?? "data/app.sqlite", tables);
await db.transaction(async (c) => {
  for (const [table, rows] of Object.entries(data)) {
    const fields = app.schema.entities[table];
    if (!fields) throw new Error(`未知のデータ ${table}`);
    for (const [i, row] of (rows as Record<string, unknown>[]).entries()) {
      const vals = fields.map((f) => toSql(decodeInput(row[f.name], f.type, app.schema, `${table}[${i}].${f.name}`), f.type));
      await c.run(`INSERT INTO ${q(table)} (${fields.map((f) => q(f.name)).join(", ")}) VALUES (${fields.map(() => "?").join(", ")})`, vals);
    }
  }
});
await db.close();
console.log("投入しました");
"#;

/// FR-019: API 契約から導いた契約テスト(生成物に同梱。`koto test` が実行する)
const CONTRACT_TEST_TS: &str = r#"// 生成物: API 契約(api.json)から導いた契約テスト。node --test tests/contract.test.ts
// 公開範囲・認証・権限(デフォルト拒否)が契約どおりかを、実際にサーバーを起動して確かめる。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { randomBytes } from "node:crypto";
import { createServer } from "node:net";
import { signToken } from "../src/runtime/auth.ts";

const api = JSON.parse(readFileSync(new URL("../api.json", import.meta.url), "utf8"));
const secret = randomBytes(32).toString("hex");
let url = "";
let proc: ChildProcess;

const port = () => new Promise<number>((ok) => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const p = (s.address() as { port: number }).port; s.close(() => ok(p)); }); });
const tok = (caps: string[]) => signToken({ sub: "contract-test", caps, exp: Math.floor(Date.now() / 1000) + 600 }, secret);
const path = (e: any, id?: string) => "/" + e.segments.map(encodeURIComponent).join("/") + (e.has_id ? "/" + encodeURIComponent(id ?? "none") : "");
const call = async (e: any, t?: string, body?: unknown) => {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (t) headers.Authorization = `Bearer ${t}`;
  return (await fetch(url + path(e), { method: e.method, headers, body: e.method === "POST" ? JSON.stringify(body ?? {}) : undefined })).status;
};

before(async () => {
  const p = await port();
  url = `http://127.0.0.1:${p}`;
  proc = spawn(process.execPath, ["--no-warnings", "src/main.ts"], { cwd: new URL("..", import.meta.url), env: { ...process.env, PORT: String(p), KOTONOHA_DB: ":memory:", KOTONOHA_JWT_SECRET: secret, KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" }, stdio: "ignore" });
  for (let i = 0; i < 200; i++) {
    try { if ((await fetch(url + "/api/health")).ok) return; } catch {}
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error("server did not start");
});
after(() => proc?.kill());

test("health", async () => assert.equal((await fetch(url + "/api/health")).status, 200));

for (const e of api.endpoints) {
  if (e.access.public) {
    test(`${e.id}: 公開(認証なしで 401/403 にならない)`, async () => {
      const s = await call(e);
      assert.ok(s !== 401 && s !== 403, `status ${s}`);
    });
    continue;
  }
  test(`${e.id}: 認証なしは 401`, async () => assert.equal(await call(e), 401));
  if (e.access.capabilities.length > 0 || e.access.grant) {
    test(`${e.id}: 権限なしは 403`, async () => assert.equal(await call(e, tok([])), 403));
  }
  if (e.method === "POST") {
    const full = e.access.grant ? [e.access.grant] : e.access.capabilities;
    test(`${e.id}: 権限があり入力が不正なら 400`, async () => assert.equal(await call(e, tok(full), { "__不明な項目__": 1 }), 400));
  }
  if (e.query?.limit) {
    test(`${e.id}: 許可されない limit は 400`, async () => {
      const r = await fetch(url + path(e) + "?limit=99999", { headers: { Authorization: `Bearer ${tok(e.access.capabilities)}` } });
      assert.equal(r.status, 400);
    });
  }
}
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
    let mut tables = Vec::new();
    for (e, d) in ir.entities.iter().zip(&ddl) {
        if let (Ok(cols), Ok(pg)) = (sqlite_columns(&e.fields, &ir.enums), pg_ddl_for(&e.name, &e.fields, &ir.enums)) {
            tables.push(json!({"name": e.name, "ddl": d, "pg_ddl": pg, "columns": cols}));
        }
    }
    g.push_str(&format!(
        "export const tables: db.TableDef[] = {};\n\n",
        serde_json::to_string_pretty(&tables).unwrap()
    ));

    // アダプタ
    for a in &ir.adapters {
        let ps: Vec<String> = a.params.iter().map(|(n, _)| mangle("v_", n)).collect();
        let sig = format!(
            "async function {}(ctx: Ctx{}{}): Promise<any>",
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
        let access = ep.access_json();
        let handler = match &ep.kind {
            EndpointKind::ListData { data } => format!(
                "async (ctx, req) => ({{ status: 200, body: (await db.adList(ctx, {d}, db.pageOf(req.query))).map((e) => rt.encode(e, {{ kind: \"entity\", name: {d} }}, schema)), headers: {{ \"X-Total-Count\": String(await db.adCount(ctx, {d})) }} }})",
                d = js_str(data)
            ),
            EndpointKind::GetData { data } => format!(
                "async (ctx, req) => ({{ status: 200, body: rt.encode(await db.adGet(ctx, {d}, rt.decodeInput(req.id, {{ kind: \"id\" }}, schema, \"id\"), false), {{ kind: \"entity\", name: {d} }}, schema) }})",
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
                            "async (ctx, req) => {{\n      const allowed = {limits:?};\n      const limit = req.query.has(\"limit\") ? Number(req.query.get(\"limit\")) : allowed[allowed.length - 1];\n      if (!allowed.includes(limit)) throw new rt.HttpError(400, \"invalid_input\", `limit は ${{allowed.join(\" / \")}} のいずれかです`);\n      const base = req.query.has(\"base\") ? (rt.decodeInput(req.query.get(\"base\"), {{ kind: \"datetime\" }}, schema, \"base\") as bigint) : BigInt(Date.now());\n      const days = {days};\n      const rows = await db.all(ctx, {sql}, [{params}]);\n      const w: any = {wire};\n      return {{ status: 200, body: rows.map((r) => rt.encode(r, w, schema)) }};\n    }}",
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
                    "async (ctx, req) => {{\n      const b = req.body as Record<string, unknown>;\n      if (b === null || typeof b !== \"object\" || Array.isArray(b)) throw new rt.HttpError(400, \"invalid_input\", \"本文はオブジェクトです\");\n      const extra = Object.keys(b).filter((k) => ![{keys}].includes(k));\n      if (extra.length > 0) throw new rt.HttpError(400, \"invalid_input\", `未知の項目: ${{extra.join(\", \")}}`);\n      const v = await {f}(ctx{sep}{args});\n      return {{ status: 200, body: {{ ok: rt.encode(v, {w}, schema) }} }};\n    }}",
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
    g.push_str(&format!(
        "export const app: App = {{ name: {}, schema, routes, owners: {} }};\n",
        js_str(&ir.name),
        serde_json::to_string(&ir.policy.owner_read).unwrap()
    ));

    if !errs.is_empty() {
        return Err(errs);
    }
    let pkg = json!({
        "name": "kotonoha-backend",
        "private": true,
        "type": "module",
        "engines": {"node": ">=22.18.0"},
        "optionalDependencies": {"pg": PG_VERSION},
        "scripts": {"start": "node --no-warnings src/main.ts", "test": "node --no-warnings --test tests/contract.test.ts tests/actions.todo.test.ts", "token": "node --no-warnings tools/dev-token.ts", "seed": "node --no-warnings tools/seed.ts"},
    });
    let readme = format!(
        "# {name} backend(KOTONOHA 生成物)\n\nNode.js 22.18 以上。SQLite だけなら依存パッケージなし。PostgreSQL を使う場合は `npm install`(任意依存の pg)。\n\n```bash\nexport KOTONOHA_JWT_SECRET=...(32 文字以上。秘密管理から注入)\nnpm run seed -- seed.json   # 任意\nnpm start                   # http://127.0.0.1:8787\n```\n\n環境変数: PORT, KOTONOHA_HOST, KOTONOHA_DB(SQLite のパス または postgres://…), KOTONOHA_JWT_SECRET, KOTONOHA_JWKS_FILE / KOTONOHA_JWKS_JSON / KOTONOHA_JWKS_URL, KOTONOHA_JWT_ISSUER, KOTONOHA_JWT_AUDIENCE, KOTONOHA_CAPS_CLAIM, KOTONOHA_ROLES_CLAIM, KOTONOHA_ROLE_MAP, KOTONOHA_RATE_LIMIT_PER_MINUTE, KOTONOHA_PG_POOL\n\nAPI 契約: api.json。意味グラフ: vag.json。\n",
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
        GeneratedFile::text("tests/contract.test.ts", CONTRACT_TEST_TS.to_string()),
        GeneratedFile::text("tests/actions.todo.test.ts", action_todos(ir)),
    ])
}

/// FR-019: 動作の `を確認 または 失敗(…)` から、業務規則のテスト雛形(test.todo)を作る。
fn action_todos(ir: &AppIr) -> String {
    use kotonoha_ast::{ExprKind, Stmt};
    fn walk(b: &kotonoha_ast::Block, out: &mut Vec<String>) {
        for s in &b.stmts {
            match s {
                Stmt::Check { cond, fail, .. } => {
                    let v = match &fail.kind {
                        ExprKind::Call { args, .. } => args.first().map(kotonoha_parser_expr).unwrap_or_default(),
                        _ => String::new(),
                    };
                    out.push(format!("{} を満たさないとき 失敗({v}) になる", kotonoha_parser_expr(cond)));
                }
                Stmt::If { branches, else_block, .. } => {
                    for (_, b) in branches {
                        walk(b, out);
                    }
                    if let Some(b) = else_block {
                        walk(b, out);
                    }
                }
                Stmt::Loop { body, .. } | Stmt::Transaction { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut s =
        String::from("// 生成物: 業務規則のテスト雛形(FR-019)。各 todo を具体的な入力と期待値で実装してください。\nimport { test } from \"node:test\";\n\n");
    for a in &ir.actions {
        let mut rules = Vec::new();
        if let Some(b) = &a.body {
            walk(b, &mut rules);
        }
        s.push_str(&format!(
            "// 動作 {}\ntest.todo({});\n",
            a.name,
            js_str(&format!("{}: 正常系の結果を確認する", a.name))
        ));
        for r in rules {
            s.push_str(&format!("test.todo({});\n", js_str(&format!("{}: {r}", a.name))));
        }
        s.push('\n');
    }
    s
}

fn kotonoha_parser_expr(e: &kotonoha_ast::Expr) -> String {
    kotonoha_parser::printer::expr_str(e)
}
