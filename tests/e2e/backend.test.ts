// Backend 生成物の結合テスト: TC-007 / TC-012 / AT-02 / AT-03 / AT-04 / AT-07 / SEC-04 / SEC-06
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import type { ChildProcess } from "node:child_process";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, shopSeed, SECRET, BASE } from "./helpers.ts";

let dir = "";
let url = "";
let proc: ChildProcess;
const env = () => ({ KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(dir, "data/app.sqlite") });

const token = (caps: string[], extra: Record<string, unknown> = {}) =>
  runNode(dir, ["tools/dev-token.ts", "--sub", "user1", "--caps", caps.join(","), ...Object.entries(extra).map(([k, v]) => `--${k}=${v}`)], env());

async function api(method: string, path: string, body?: unknown, tok?: string) {
  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (tok) headers.Authorization = `Bearer ${tok}`;
  const r = await fetch(url + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  return { status: r.status, body: await r.json().catch(() => null), headers: r.headers };
}
const p = (...segs: string[]) => "/" + segs.map(encodeURIComponent).join("/");

before(async () => {
  dir = tmp("backend");
  build("examples/shop", "backend", dir);
  runNode(dir, ["tools/seed.ts", seedFile(dir, shopSeed())], env());
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = start(dir, ["src/main.ts"], { ...env(), PORT: String(port) });
  await waitHttp(url + "/api/health");
});
after(() => proc?.kill());

test("AT-02 公開データの一覧・詳細", async () => {
  const l = await api("GET", p("api", "data", "商品"));
  assert.equal(l.status, 200);
  assert.equal(l.body.length, 3);
  assert.deepEqual(l.body[0], { ID: "P01", 名前: "緑茶", 価格: 300, 在庫: 10 });
  const d = await api("GET", p("api", "data", "商品", "P02"));
  assert.equal(d.body.名前, "ほうじ茶");
  assert.ok(d.headers.get("x-trace-id"), "NFR-008 トレース ID");
  assert.equal((await api("GET", p("api", "data", "商品", "none"))).status, 404);
});

test("AT-04 / TC-007 認可はサーバーで強制(401 / 403)", async () => {
  assert.equal((await api("POST", p("api", "actions", "注文する"), { 商品ID: "P01", 数量: 1 })).status, 401);
  const weak = token(["注文.create"]);
  const r = await api("POST", p("api", "actions", "注文する"), { 商品ID: "P01", 数量: 1 }, weak);
  assert.equal(r.status, 403);
  assert.match(r.body.message, /商品.読取/);
  // 非公開データ(注文明細)の集計
  assert.equal((await api("GET", p("api", "queries", "売れ筋商品"))).status, 401);
});

test("偽造トークン(alg=none・署名改ざん・期限切れ)は 401", async () => {
  const good = token(["商品.read", "商品.update", "注文.create"]);
  const [h, b] = good.split(".");
  const none = Buffer.from(JSON.stringify({ alg: "none", typ: "JWT" })).toString("base64url") + "." + b + ".";
  const elevated = Buffer.from(JSON.stringify({ sub: "x", caps: ["商品.create"], exp: 9999999999 })).toString("base64url");
  for (const t of [none, `${h}.${elevated}.${good.split(".")[2]}`, token(["注文.create"], { ttl: -100 })]) {
    assert.equal((await api("POST", p("api", "actions", "注文する"), { 商品ID: "P01", 数量: 1 }, t)).status, 401);
  }
});

test("TC-012 注文登録と在庫の原子的更新・失敗時のロールバック", async () => {
  const t = token(["商品.read", "商品.update", "注文.create"]);
  const ok = await api("POST", p("api", "actions", "注文する"), { 商品ID: "P01", 数量: 3 }, t);
  assert.equal(ok.status, 200, JSON.stringify(ok.body));
  assert.match(ok.body.ok, /^[0-9a-f-]{36}$/);
  assert.equal((await api("GET", p("api", "data", "商品", "P01"))).body.在庫, 7);
  // 在庫不足 → 422、在庫は変わらない
  const ng = await api("POST", p("api", "actions", "注文する"), { 商品ID: "P02", 数量: 5 }, t);
  assert.deepEqual([ng.status, ng.body.value], [422, "在庫不足"]);
  assert.equal((await api("GET", p("api", "data", "商品", "P02"))).body.在庫, 1);
  const zero = await api("POST", p("api", "actions", "注文する"), { 商品ID: "P02", 数量: 0 }, t);
  assert.deepEqual([zero.status, zero.body.value], [422, "数量不正"]);
  // 存在しない商品 → 404(取引はロールバック)
  assert.equal((await api("POST", p("api", "actions", "注文する"), { 商品ID: "NOPE", 数量: 1 }, t)).status, 404);
});

test("入力検証(型・未知項目・不正 JSON)は 400", async () => {
  const t = token(["商品.read", "商品.update", "注文.create"]);
  for (const body of [{ 商品ID: "P01", 数量: "1" }, { 商品ID: "P01", 数量: 1.5 }, { 商品ID: "P01" }, { 商品ID: "P01", 数量: 1, 余計: 1 }, [1]]) {
    assert.equal((await api("POST", p("api", "actions", "注文する"), body, t)).status, 400, JSON.stringify(body));
  }
  const r = await fetch(url + p("api", "actions", "注文する"), { method: "POST", headers: { "Content-Type": "application/json", Authorization: `Bearer ${t}` }, body: "{" });
  assert.equal(r.status, 400);
});

test("SEC-04 SQL インジェクションは成立しない", async () => {
  const r = await api("GET", p("api", "data", "商品", "P01' OR '1'='1"));
  assert.equal(r.status, 404);
  assert.equal((await api("GET", p("api", "data", "商品"))).body.length >= 3, true);
});

test("AT-03 商品を追加すると一覧に反映される", async () => {
  const t = token(["商品.create"]);
  const r = await api("POST", p("api", "actions", "商品登録"), { 名前: "玄米茶", 価格: 420, 在庫: 8 }, t);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  const l = await api("GET", p("api", "data", "商品"));
  assert.ok(l.body.some((x: { 名前: string }) => x.名前 === "玄米茶"));
  const bad = await api("POST", p("api", "actions", "商品登録"), { 名前: "x", 価格: 1, 在庫: -1 }, t);
  assert.deepEqual([bad.status, bad.body.value], [422, "在庫不正"]);
});

test("AT-07 売れ筋(直近30日・売上金額・確定のみ)", async () => {
  const t = token(["注文明細.read"]);
  const r = await api("GET", p("api", "queries", "売れ筋商品") + `?base=${encodeURIComponent(BASE)}`, undefined, t);
  assert.equal(r.status, 200, JSON.stringify(r.body));
  // L2 は区間の始点ちょうど(含む)、L3 取消・L4 期間外は除外
  assert.deepEqual(r.body, [
    { 商品ID: "P02", 売上金額: 9000 },
    { 商品ID: "P01", 売上金額: 3000 },
    { 商品ID: "P03", 売上金額: 500 },
  ]);
  assert.equal((await api("GET", p("api", "queries", "売れ筋商品") + "?limit=999", undefined, t)).status, 400);
});

test("SEC-06 / AT-10 生成物に秘密情報を含めない", () => {
  const walk = (d: string): string[] => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? (f === "data" ? [] : walk(join(d, f))) : [join(d, f)]));
  for (const f of walk(dir)) {
    if (f.endsWith("seed.json")) continue;
    assert.ok(!readFileSync(f, "utf8").includes(SECRET), f);
  }
});
