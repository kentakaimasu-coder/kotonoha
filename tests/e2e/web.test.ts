// Web 生成物の E2E(Playwright): AT-01 / TC-011 / FR-012 / TC-008 / AT-03 / AT-10 / CSRF
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { execFileSync, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, shopSeed, SECRET } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

let be = "", web = "", webUrl = "";
const procs: ChildProcess[] = [];
let browser: any;

before(async () => {
  be = tmp("be");
  web = tmp("web");
  build("examples/shop", "backend", be);
  build("examples/shop", "web", web);
  const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(be, "data/app.sqlite") };
  runNode(be, ["tools/seed.ts", seedFile(be, shopSeed())], env);
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: web, stdio: "inherit" });
  execFileSync("npm", ["run", "build", "--silent"], { cwd: web, stdio: "inherit" });
  const [bp, wp] = [await freePort(), await freePort()];
  procs.push(start(be, ["src/main.ts"], { ...env, PORT: String(bp) }));
  procs.push(start(web, ["src/server-main.ts"], { PORT: String(wp), KOTONOHA_BACKEND_URL: `http://127.0.0.1:${bp}` }));
  webUrl = `http://127.0.0.1:${wp}`;
  await waitHttp(`http://127.0.0.1:${bp}/api/health`);
  await waitHttp(webUrl + "/login");
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  procs.forEach((p) => p.kill());
});

const token = (caps: string[]) => runNode(be, ["tools/dev-token.ts", "--sub", "web-user", "--caps", caps.join(",")], { KOTONOHA_JWT_SECRET: SECRET });

async function newPage() {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  page.on("dialog", (d: any) => {
    errors.push("dialog: " + d.message());
    d.dismiss();
  });
  return { ctx, page, errors };
}

async function login(page: any, caps: string[]) {
  await page.goto(webUrl + "/login");
  await page.fill("#token", token(caps));
  await page.click("form[action='/login'] button");
}

test("AT-01 / TC-011 商品一覧の SSR 表示とクリック遷移", async () => {
  const { page, errors } = await newPage();
  const res = await page.goto(webUrl + "/");
  assert.equal(res.status(), 200);
  assert.equal(await page.textContent("h1"), "商品一覧");
  // SSR(JavaScript 無しでも本文がある)
  const html = await (await fetch(webUrl + "/s/" + encodeURIComponent("商品一覧"))).text();
  assert.ok(html.includes("緑茶") && html.includes("¥300"));
  await page.click("text=緑茶");
  await page.waitForURL(/%E5%95%86%E5%93%81%E8%A9%B3%E7%B4%B0/);
  assert.equal(await page.textContent("h1"), "商品詳細");
  assert.ok((await page.textContent("dl.detail"))?.includes("¥300"));
  assert.deepEqual(errors, []);
});

test("FR-012 / AT-04 ボタン操作 → 状態変更 → 表示更新(未ログインは拒否)", async () => {
  const { page, errors } = await newPage();
  await page.goto(webUrl + "/s/" + encodeURIComponent("商品詳細") + "/P01");
  await page.click("text=購入");
  await page.waitForSelector(".msg.error");
  assert.match(await page.textContent(".msg.error"), /ログインが必要/);
  await login(page, ["商品.read", "商品.update", "注文.create"]);
  await page.goto(webUrl + "/s/" + encodeURIComponent("商品詳細") + "/P01");
  const before = await page.textContent("dl.detail");
  assert.ok(before.includes("10"));
  await page.click("text=購入");
  await page.waitForSelector(".msg.ok");
  await page.waitForFunction(() => document.querySelector("dl.detail")?.textContent?.includes("9"));
  assert.deepEqual(errors, []);
});

test("TC-008 HTML への不正入力で XSS が成立しない", async () => {
  const { page, errors } = await newPage();
  await page.goto(webUrl + "/s/" + encodeURIComponent("商品一覧"));
  assert.ok((await page.textContent("table.list")).includes("<script>alert(1)</script>"), "文字列として表示");
  assert.equal(await page.locator("table.list script").count(), 0);
  assert.equal(await page.locator("table.list img").count(), 0);
  await page.waitForTimeout(300);
  assert.deepEqual(errors, [], "alert も JS エラーも起きない");
});

test("AT-03 フォームで商品を追加すると一覧に反映される", async () => {
  const { page, errors } = await newPage();
  await login(page, ["商品.create"]);
  await page.goto(webUrl + "/s/" + encodeURIComponent("商品管理"));
  await page.fill("#form-商品登録-名前", "玄米茶");
  // NR-01: 全角数字・桁区切りも受け付ける
  await page.fill("#form-商品登録-価格", "４２０");
  await page.fill("#form-商品登録-在庫", "1,000");
  await page.click("form.form button[type=submit]");
  await page.waitForSelector(".msg.ok");
  await page.waitForFunction(() => document.querySelector("table.list")?.textContent?.includes("玄米茶"));
  assert.ok((await page.textContent("table.list")).includes("¥420"), "全角の ４２０ が 420 円として登録される");
  // 権限なし(閲覧のみの利用者)にはフォームを出さない。API に直接送っても 403(認可は backend)
  const { page: p2 } = await newPage();
  await login(p2, ["商品.read"]);
  await p2.goto(webUrl + "/s/" + encodeURIComponent("商品管理"));
  assert.equal(await p2.locator("#form-商品登録-名前").count(), 0);
  const r403 = await p2.evaluate(async () => {
    const r = await fetch("/api/actions/" + encodeURIComponent("商品登録"), { method: "POST", headers: { "Content-Type": "application/json", "X-CSRF": "1" }, body: JSON.stringify({ 名前: "x", 価格: 1, 在庫: 1 }) });
    return { status: r.status, body: await r.json() };
  });
  assert.equal(r403.status, 403);
  assert.match(r403.body.message, /権限がありません/);
  assert.deepEqual(errors, []);
});

test("売れ筋画面は権限が無ければ案内を表示し、権限があれば表示する", async () => {
  const { page } = await newPage();
  await page.goto(webUrl + "/s/" + encodeURIComponent("売れ筋"));
  assert.match(await page.textContent(".notice"), /ログインが必要/);
  await login(page, ["注文明細.read"]);
  await page.goto(webUrl + "/s/" + encodeURIComponent("売れ筋"));
  assert.ok((await page.textContent("caption")).includes("売れ筋商品"));
});

test("CSRF: X-CSRF なし・別オリジンの API 呼出は拒否", async () => {
  // CSRF の本体: ログイン Cookie が自動で付く要求に X-CSRF が無ければ拒否
  const cookie = `sid=${token(["商品.create"])}`;
  const r1 = await fetch(webUrl + "/api/actions/" + encodeURIComponent("商品登録"), { method: "POST", headers: { "Content-Type": "application/json", Cookie: cookie }, body: "{}" });
  assert.equal(r1.status, 403);
  // Cookie も Bearer も無い要求は CSRF の対象外(backend が 401 を返す)
  const r0 = await fetch(webUrl + "/api/actions/" + encodeURIComponent("商品登録"), { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
  assert.equal(r0.status, 401);
  // 他サイトからのログイン POST(ログイン CSRF)は拒否
  const rl = await fetch(webUrl + "/login", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded", Origin: "http://evil.example" }, body: "token=" + token([]), redirect: "manual" });
  assert.equal(rl.status, 403);
  const r2 = await fetch(webUrl + "/api/actions/" + encodeURIComponent("商品登録"), {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-CSRF": "1", Origin: "http://evil.example" },
    body: "{}",
  });
  assert.equal(r2.status, 403);
  const page = await fetch(webUrl + "/s/" + encodeURIComponent("商品一覧"));
  assert.match(page.headers.get("content-security-policy") ?? "", /script-src 'self'/);
});

test("AT-10 生成フロントエンドに認証情報を含めない", () => {
  for (const f of ["dist/client.js", "src/screens.generated.ts", "src/runtime/server.ts"]) {
    const s = readFileSync(join(web, f), "utf8");
    assert.ok(!s.includes(SECRET), f);
    assert.ok(!/KOTONOHA_JWT_SECRET/.test(s), f);
  }
});

test("ADR-0118 注文履歴は本人の注文だけを表示する", async () => {
  const { page: a } = await newPage();
  await login(a, ["動作.注文する"]);
  await a.goto(webUrl + "/s/" + encodeURIComponent("商品詳細") + "/P01");
  await a.click("text=購入");
  await a.waitForSelector(".msg.ok");
  // K-13(改訂): 完了を伝え、自動採番の ID(UUID)は利用者に見せない
  assert.match(await a.textContent(".msg.ok"), /完了しました/, "K-13 結果の表示");
  assert.doesNotMatch(await a.textContent(".msg.ok"), /[0-9a-f]{8}-[0-9a-f]{4}-/, "K-13 UUID を出さない");
  await a.goto(webUrl + "/s/" + encodeURIComponent("注文履歴"));
  const mine = await a.locator("table.list tbody tr").count();
  assert.ok(mine >= 1);
  assert.ok(!(await a.textContent("table.list")).includes("other-user"));
  // 別の利用者には見えない
  const { page: b } = await newPage();
  await b.goto(webUrl + "/login");
  await b.fill("#token", runNode(be, ["tools/dev-token.ts", "--sub", "other-user", "--caps", ""], { KOTONOHA_JWT_SECRET: SECRET }));
  await b.click("form[action='/login'] button");
  await b.goto(webUrl + "/s/" + encodeURIComponent("注文履歴"));
  assert.match(await b.textContent("main"), /データがありません/);
});

test("K-06 アクセシビリティ自動監査(axe-core: WCAG 2.2 A/AA)で違反なし", async () => {
  const axe = readFileSync(require.resolve("axe-core/axe.min.js"), "utf8");
  const { page } = await newPage();
  await login(page, ["商品.create", "注文明細.read", "動作.注文する"]);
  const results: string[] = [];
  for (const path of ["/s/" + encodeURIComponent("商品一覧"), "/s/" + encodeURIComponent("商品詳細") + "/P01", "/s/" + encodeURIComponent("商品管理"), "/s/" + encodeURIComponent("売れ筋"), "/login"]) {
    await page.goto(webUrl + path);
    // CSP(script-src 'self')下でも動くよう、Playwright の評価経路で注入する
    await page.evaluate(axe);
    const v = await page.evaluate(async () => {
      const r = await (window as any).axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] } });
      return r.violations.map((x: any) => `${x.id}: ${x.nodes.map((n: any) => n.target.join(" ")).join(", ")}`);
    });
    results.push(...v.map((x: string) => `${decodeURIComponent(path)} ${x}`));
  }
  assert.deepEqual(results, []);
});
