// 生成 backend を PostgreSQL で動かす結合テスト(ADR-003)。KOTONOHA_TEST_PG_URL が無ければスキップ。
// 例: KOTONOHA_TEST_PG_URL=postgres://kotonoha@127.0.0.1:55432/postgres
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, type ChildProcess } from "node:child_process";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, shopSeed, SECRET, BASE } from "./helpers.ts";

const PG = process.env.KOTONOHA_TEST_PG_URL;
const skip = PG ? false : "KOTONOHA_TEST_PG_URL が未設定";
let dir = "", url = "";
let proc: ChildProcess;
const p = (...segs: string[]) => "/" + segs.map(encodeURIComponent).join("/");
const tok = (sub: string, caps: string[]) => runNode(dir, ["tools/dev-token.ts", "--sub", sub, "--caps", caps.join(",")], { KOTONOHA_JWT_SECRET: SECRET });
async function api(method: string, path: string, t?: string, body?: unknown) {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (t) headers.Authorization = `Bearer ${t}`;
  const r = await fetch(url + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  return { status: r.status, body: await r.json().catch(() => null) };
}

before(async () => {
  if (!PG) return;
  // テスト用にスキーマを作り直す
  execFileSync("psql", [PG, "-q", "-c", "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;"]);
  dir = tmp("pg");
  build("examples/shop", "backend", dir);
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: dir, stdio: "inherit" });
  const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: PG, KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" };
  runNode(dir, ["tools/seed.ts", seedFile(dir, shopSeed())], env);
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = start(dir, ["src/main.ts"], { ...env, PORT: String(port) });
  await waitHttp(url + "/api/health");
});
after(() => proc?.kill());

test("PostgreSQL: 一覧・詳細・ページング", { skip }, async () => {
  const l = await api("GET", p("api", "data", "商品") + "?limit=2");
  assert.equal(l.status, 200);
  assert.deepEqual(l.body[0], { ID: "P01", 名前: "緑茶", 価格: 300, 在庫: 10 });
  assert.equal(l.body.length, 2);
  assert.equal((await api("GET", p("api", "data", "商品", "P02"))).body.名前, "ほうじ茶");
});

test("PostgreSQL: TC-012 同時注文でも在庫は負にならず、成功数は在庫数と一致(SERIALIZABLE + 再試行)", { skip }, async () => {
  const t = tok("buyer", ["動作.注文する"]);
  const rs = await Promise.all(Array.from({ length: 20 }, () => api("POST", p("api", "actions", "注文する"), t, { 商品ID: "P01", 数量: 1 })));
  const ok = rs.filter((r) => r.status === 200).length;
  const short = rs.filter((r) => r.status === 422 && r.body.value === "在庫不足").length;
  assert.deepEqual([ok, short], [10, 10], JSON.stringify(rs.map((r) => r.status)));
  assert.equal((await api("GET", p("api", "data", "商品", "P01"))).body.在庫, 0);
  assert.equal((await api("GET", p("api", "data", "注文"), t)).body.length, 10, "注文は成功分だけ(行レベル認可で本人分)");
});

test("PostgreSQL: AT-07 売れ筋の集計(SUM は BigInt で受け取り JSON 数値へ)", { skip }, async () => {
  const r = await api("GET", p("api", "queries", "売れ筋商品") + `?base=${encodeURIComponent(BASE)}`, tok("a", ["注文明細.read"]));
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.deepEqual(r.body, [
    { 商品ID: "P02", 売上金額: 9000 },
    { 商品ID: "P01", 売上金額: 3000 },
    { 商品ID: "P03", 売上金額: 500 },
  ]);
});

test("PostgreSQL: 列挙の CHECK 制約と登録", { skip }, async () => {
  const r = await api("POST", p("api", "actions", "商品登録"), tok("admin", ["動作.商品登録"]), { 名前: "玄米茶", 価格: 420, 在庫: 8 });
  assert.equal(r.status, 200);
  assert.ok((await api("GET", p("api", "data", "商品"))).body.some((x: any) => x.名前 === "玄米茶"));
  // 列挙外の値は DB の CHECK 制約でも拒否される
  assert.throws(
    () => execFileSync("psql", [PG!, "-v", "ON_ERROR_STOP=1", "-qc", `INSERT INTO "注文明細" VALUES ('x', 'P01', 1, '保留', 0)`], { stdio: "pipe" }),
    /check constraint/,
  );
});

test("PostgreSQL: K-05 選択型の列追加はデータを保ったまま適用", { skip }, async () => {
  const { cpSync, readFileSync, writeFileSync } = await import("node:fs");
  const { join } = await import("node:path");
  const { ROOT, KOTO } = await import("./helpers.ts");
  const proj = tmp("pgproj");
  cpSync(join(ROOT, "examples/shop"), proj, { recursive: true, filter: (s) => !s.includes("/build") });
  const src = join(proj, "src/app.koto");
  writeFileSync(src, readFileSync(src, "utf8").replace("    在庫: 整数\n", "    在庫: 整数\n    説明: 選択<文字列>\n"));
  execFileSync(KOTO, ["-C", proj, "build", "--target", "backend", "--out", dir]);
  proc.kill();
  await new Promise((r) => setTimeout(r, 300));
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = start(dir, ["src/main.ts"], { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: PG!, PORT: String(port) });
  await waitHttp(url + "/api/health");
  const r = await api("GET", p("api", "data", "商品", "P02"));
  assert.deepEqual(r.body, { ID: "P02", 名前: "ほうじ茶", 価格: 350, 在庫: 1, 説明: null });
  const log = execFileSync("psql", [PG!, "-qtAc", `SELECT action FROM "_kotonoha_migrations" ORDER BY at`]).toString();
  assert.match(log, /add 商品\.説明/);
});
