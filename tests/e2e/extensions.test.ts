// 拡張の E2E: 関数・外部関数(TypeScript / Python)・分析(SQL)・拡張部品・実験(A/B テスト)
// ADR-0122〜0125 / NR-04〜08。examples/lab を生成して backend と web を起動し、ブラウザでも確かめる。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { execFileSync, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, SECRET } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

let be = "", web = "", api = "", webUrl = "";
const procs: ChildProcess[] = [];
let browser: any;
const enc = encodeURIComponent;

before(async () => {
  be = tmp("lab-be");
  web = tmp("lab-web");
  build("examples/lab", "backend", be);
  build("examples/lab", "web", web);
  const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(be, "data/app.sqlite") };
  runNode(be, ["tools/seed.ts", seedFile(be, { 商品: [{ ID: "p-1", 名前: "湯のみ", 価格: 2500, 評価: 4 }, { ID: "p-2", 名前: "急須", 価格: 12000, 評価: 5 }] })], env);
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: web, stdio: "inherit" });
  execFileSync("npm", ["run", "build", "--silent"], { cwd: web, stdio: "inherit" });
  const [bp, wp] = [await freePort(), await freePort()];
  procs.push(start(be, ["src/main.ts"], { ...env, PORT: String(bp), KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" }));
  procs.push(start(web, ["src/server-main.ts"], { PORT: String(wp), KOTONOHA_BACKEND_URL: `http://127.0.0.1:${bp}` }));
  api = `http://127.0.0.1:${bp}`;
  webUrl = `http://127.0.0.1:${wp}`;
  await waitHttp(api + "/api/health");
  await waitHttp(webUrl + "/login");
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  procs.forEach((p) => p.kill());
});

const token = (sub: string, caps: string[]) => runNode(be, ["tools/dev-token.ts", "--sub", sub, "--caps", caps.join(",")], { KOTONOHA_JWT_SECRET: SECRET });
const buyer = () => token("buyer", ["注文.create", "商品.read"]);
const post = (path: string, body: unknown, headers: Record<string, string> = {}) =>
  fetch(api + path, { method: "POST", headers: { "Content-Type": "application/json", ...headers }, body: JSON.stringify(body) });

test("NR-05 / NR-06 / NR-08 関数と外部関数(TypeScript と Python)を動作から呼ぶ", async () => {
  const r = await post(`/api/actions/${enc("購入する")}`, { 商品ID: "p-2" }, { Authorization: `Bearer ${buyer()}` });
  assert.equal(r.status, 200, await r.clone().text());
  // 割引額(TS): 12000 円は 1 割引き → 10800 円で登録される
  const rows = await (await fetch(`${api}/api/analyses/${enc("商品別売上")}`)).json();
  assert.deepEqual(rows, [{ 商品ID: "p-2", 売上: 10800, 件数: 1 }]);
});

test("NR-07 分析: 引数の検証・未知の引数の拒否・結果の型", async () => {
  const ok = await fetch(`${api}/api/analyses/${enc("期間売上")}?${enc("開始")}=${enc("2020-01-01T00:00:00Z")}`);
  assert.equal(ok.status, 200);
  const [row] = await ok.json();
  assert.equal(typeof row.売上, "number");
  assert.equal((await fetch(`${api}/api/analyses/${enc("期間売上")}`)).status, 400);
  assert.equal((await fetch(`${api}/api/analyses/${enc("期間売上")}?${enc("開始")}=x`)).status, 400);
  assert.equal((await fetch(`${api}/api/analyses/${enc("期間売上")}?${enc("開始")}=${enc("2020-01-01T00:00:00Z")}&limit=1`)).status, 400);
  // SQL を渡す口は無い(分析は宣言したものだけ)
  assert.equal((await fetch(`${api}/api/analyses/${enc("DROP TABLE")}`)).status, 404);
});

test("NR-04 実験: 割付は決定的でコンパイラと同じ計算・成果は露出した対象者だけ 1 回", async () => {
  const E = enc("購入ボタン文言");
  const a1 = await (await post(`/api/experiments/${E}/assign`, {}, { "X-Koto-Subject": "anon-0001" })).json();
  const a2 = await (await post(`/api/experiments/${E}/assign`, {}, { "X-Koto-Subject": "anon-0001" })).json();
  assert.deepEqual(a1, a2);
  // crates/ext の assign("購入ボタン文言", "anon-0001", [50, 50]) == 1 と一致
  assert.deepEqual(a1, { variant: "今すぐ購入", index: 1 });
  assert.equal((await post(`/api/experiments/${E}/assign`, { subject: "短い" })).status, 400);
  assert.equal((await post(`/api/experiments/${E}/assign`, {})).status, 400);
  // 成果: 露出済みの端末から 2 回購入しても 1 回、露出していない端末は数えない
  const t = buyer();
  for (const s of ["anon-0001", "anon-0001", "never-exposed-1"]) {
    assert.equal((await post(`/api/actions/${enc("購入する")}`, { 商品ID: "p-1" }, { Authorization: `Bearer ${t}`, "X-Koto-Subject": s })).status, 200);
  }
  // 結果は許可 実験.結果 が必要
  assert.equal((await fetch(`${api}/api/experiments/${E}/results`)).status, 401);
  assert.equal((await fetch(`${api}/api/experiments/${E}/results`, { headers: { Authorization: `Bearer ${t}` } })).status, 403);
  const res = await (await fetch(`${api}/api/experiments/${E}/results`, { headers: { Authorization: `Bearer ${token("analyst", ["実験.結果"])}` } })).json();
  const v1 = res.variants.find((v: any) => v.variant === "今すぐ購入");
  assert.equal(v1.exposures, 1);
  assert.equal(v1.conversions, 1);
  assert.equal(res.variants[0].control, true);
});

test("NR-04 Web: 変種の出し分け(ラベルと部品)・匿名 ID の Cookie・分析の表", async () => {
  // 同じ匿名 ID なら毎回同じ変種、Cookie は HttpOnly
  const r = await fetch(`${webUrl}/s/${enc("商品詳細")}/p-1`);
  assert.equal(r.status, 200);
  const cookie = r.headers.get("set-cookie") ?? "";
  assert.match(cookie, /^vid=[A-Za-z0-9_-]{16,64}; HttpOnly; SameSite=Lax/);
  const vid = cookie.split(";")[0];
  const html1 = await (await fetch(`${webUrl}/s/${enc("商品詳細")}/p-1`, { headers: { Cookie: vid } })).text();
  const html2 = await (await fetch(`${webUrl}/s/${enc("商品詳細")}/p-1`, { headers: { Cookie: vid } })).text();
  const pick = (h: string) => [h.includes("今すぐ購入") ? "今すぐ購入" : "購入", h.includes("★") ? "星" : "数字"];
  assert.deepEqual(pick(html1), pick(html2));
  assert.ok(html1.includes("★★★★☆") || html1.includes("評価 4 / 5"));
  // 多くの匿名 ID では両方の変種が現れる
  const seen = new Set<string>();
  for (let i = 0; i < 16; i++) {
    const h = await (await fetch(`${webUrl}/s/${enc("商品詳細")}/p-1`, { headers: { Cookie: `vid=device-${i}-abcdefghijkl` } })).text();
    seen.add(pick(h).join("/"));
  }
  assert.ok(seen.size >= 2, [...seen].join(","));
  // 分析の表(商品一覧)
  const list = await (await fetch(`${webUrl}/s/${enc("商品一覧")}`)).text();
  assert.ok(list.includes("商品別売上") && list.includes("¥10,800"), list);
  // 匿名 ID は backend に渡るが、レスポンスに痕跡(製品名)は出さない
  assert.ok(!/kotonoha/i.test(cookie));
});

test("NR-04 Web: ブラウザで変種のまま購入でき、ハイドレーション後も表示が変わらない", async () => {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  page.on("console", (m: any) => m.type() === "error" && errors.push(m.text()));
  await page.goto(webUrl + "/login");
  await page.fill("#token", buyer());
  await page.click("form[action='/login'] button");
  await page.goto(`${webUrl}/s/${enc("商品詳細")}/p-1`);
  const before = await page.textContent("main");
  await page.waitForLoadState("networkidle");
  assert.equal(await page.textContent("main"), before);
  const label = (await page.textContent("main"))!.includes("今すぐ購入") ? "今すぐ購入" : "購入";
  await page.click(`button:has-text("${label}")`);
  await page.waitForSelector(".msg");
  assert.match((await page.textContent(".msg"))!, /完了/);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test("NR-06 / NR-08 外部関数の失敗・型違いは 422(内部情報を出さない)", async () => {
  const cases: { name: string; ts?: string; py?: string; code: string }[] = [
    { name: "TS が型違いを返す", ts: 'export function 割引額(価格: number): unknown { return "たくさん"; }\n', code: "external_type" },
    { name: "Python が例外", py: "\n\ndef 需要予測(商品ID, 価格):\n    raise RuntimeError('secret detail')\n", code: "external_failed" },
    { name: "Python が型違い", py: "\n\ndef 需要予測(商品ID, 価格):\n    return 'many'\n", code: "external_type" },
  ];
  for (const c of cases) {
    const dir = tmp("lab-bad");
    build("examples/lab", "backend", dir);
    if (c.ts) writeFileSync(join(dir, "src/ext/ext/pricing.ts"), c.ts);
    if (c.py) {
      const py = join(dir, "ext/ml/forecast.py");
      writeFileSync(py, readFileSync(py, "utf8") + c.py);
    }
    const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(dir, "data/app.sqlite") };
    runNode(dir, ["tools/seed.ts", seedFile(dir, { 商品: [{ ID: "p-1", 名前: "湯のみ", 価格: 2500, 評価: 4 }] })], env);
    const port = await freePort();
    const p = start(dir, ["src/main.ts"], { ...env, PORT: String(port) });
    try {
      await waitHttp(`http://127.0.0.1:${port}/api/health`);
      const r = await fetch(`http://127.0.0.1:${port}/api/actions/${enc("購入する")}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${buyer()}` },
        body: JSON.stringify({ 商品ID: "p-1" }),
      });
      assert.equal(r.status, 422, c.name);
      const text = await r.text();
      assert.equal(JSON.parse(text).code, c.code, `${c.name}: ${text}`);
      assert.ok(!text.includes("たくさん") && !text.includes("secret") && !text.includes("many"), text);
    } finally {
      p.kill();
    }
  }
});

// PostgreSQL でも分析(読み取り専用の取引・時間制限)と実験が動くこと。KOTONOHA_TEST_PG_URL が無ければスキップ
const PG = process.env.KOTONOHA_TEST_PG_URL;
test("NR-04 / NR-07 PostgreSQL: 分析と実験", { skip: PG ? false : "KOTONOHA_TEST_PG_URL が未設定" }, async () => {
  // postgres.test.ts と並行して動くため、専用のデータベースを使う
  const u = new URL(PG!);
  execFileSync("psql", [PG!, "-q", "-c", "DROP DATABASE IF EXISTS kotonoha_lab"]);
  execFileSync("psql", [PG!, "-q", "-c", "CREATE DATABASE kotonoha_lab"]);
  u.pathname = "/kotonoha_lab";
  const dir = tmp("lab-pg");
  build("examples/lab", "backend", dir);
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: dir, stdio: "inherit" });
  const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: u.toString(), KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" };
  runNode(dir, ["tools/seed.ts", seedFile(dir, { 商品: [{ ID: "p-2", 名前: "急須", 価格: 12000, 評価: 5 }] })], env);
  const port = await freePort();
  const p = start(dir, ["src/main.ts"], { ...env, PORT: String(port) });
  const base = `http://127.0.0.1:${port}`;
  try {
    await waitHttp(base + "/api/health");
    const E = enc("購入ボタン文言");
    const a = await (await fetch(`${base}/api/experiments/${E}/assign`, { method: "POST", headers: { "Content-Type": "application/json", "X-Koto-Subject": "anon-0001" }, body: "{}" })).json();
    assert.deepEqual(a, { variant: "今すぐ購入", index: 1 });
    const buy = await fetch(`${base}/api/actions/${enc("購入する")}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${buyer()}`, "X-Koto-Subject": "anon-0001" },
      body: JSON.stringify({ 商品ID: "p-2" }),
    });
    assert.equal(buy.status, 200, await buy.clone().text());
    assert.deepEqual(await (await fetch(`${base}/api/analyses/${enc("商品別売上")}`)).json(), [{ 商品ID: "p-2", 売上: 10800, 件数: 1 }]);
    const since = await (await fetch(`${base}/api/analyses/${enc("期間売上")}?${enc("開始")}=${enc("2020-01-01T00:00:00+09:00")}`)).json();
    assert.deepEqual(since, [{ 売上: 10800, 件数: 1 }]);
    const res = await (await fetch(`${base}/api/experiments/${E}/results`, { headers: { Authorization: `Bearer ${token("analyst", ["実験.結果"])}` } })).json();
    assert.equal(res.variants[1].conversions, 1);
  } finally {
    p.kill();
  }
});
