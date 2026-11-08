// 実際に作ったアプリ(examples/event: 勉強会・セミナーの受付)を、利用者の操作どおりに通しで確かめる。
// PC とスマホ(iPhone 13 相当)の両方。日本人の利用者にとっての読みやすさ(日時・メッセージ)も確認する。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { execFileSync, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, SECRET } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium, devices } = require("playwright");
const enc = encodeURIComponent;

let be = "", web = "", api = "", webUrl = "";
const procs: ChildProcess[] = [];
let browser: any;

const SEED = {
  イベント: [
    { ID: "e1", 名前: "はじめての Rust 勉強会", 会場: "渋谷", 開始日時: "2026-12-05T19:00:00+09:00", 参加費: 1000, 定員: 3, 予約数: 0 },
    { ID: "e2", 名前: "データ分析入門セミナー", 会場: "大阪", 開始日時: "2026-12-12T14:00:00+09:00", 参加費: 0, 定員: 30, 予約数: 12 },
    { ID: "e3", 名前: "デザイン相談会", 会場: "渋谷", 開始日時: "2026-12-19T10:00:00+09:00", 参加費: 3000, 定員: 10, 予約数: 9 },
  ],
};

before(async () => {
  be = tmp("event-be");
  web = tmp("event-web");
  build("examples/event", "backend", be);
  build("examples/event", "web", web);
  const env = { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(be, "data/app.sqlite") };
  runNode(be, ["tools/seed.ts", seedFile(be, SEED)], env);
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
const attendee = (sub: string) => token(sub, ["予約.create", "イベント.read", "イベント.update"]);

async function session(kind: "pc" | "phone", tok?: string) {
  const opts = kind === "pc" ? { viewport: { width: 1280, height: 860 } } : { ...devices["iPhone 13"] };
  const ctx = await browser.newContext({ ...opts, locale: "ja-JP" });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  if (tok) {
    await page.goto(webUrl + "/login");
    await page.fill("#token", tok);
    await page.click("form[action='/login'] button");
  }
  return { ctx, page, errors };
}

for (const kind of ["pc", "phone"] as const) {
  test(`${kind}: 一覧 → 詳細 → 予約 → 満席 → 自分の予約(日本語で読める表示)`, async () => {
    // 毎回新しいイベントで確かめる(主催者 API で登録)
    const org = token(`org-${kind}`, ["イベント.create"]);
    const created = await fetch(`${api}/api/actions/${enc("イベント登録")}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${org}` },
      body: JSON.stringify({ 名前: `少人数ワークショップ(${kind})`, 会場: "名古屋", 開始日時: "2026-12-24T18:30:00+09:00", 参加費: 500, 定員: 3 }),
    });
    assert.equal(created.status, 200);
    const id = (await created.json()).ok as string;

    const { ctx, page, errors } = await session(kind);
    await page.goto(webUrl + "/");
    await page.click(`text=少人数ワークショップ(${kind})`);
    await page.waitForSelector("dl");
    // 日時は「2026年12月24日(木) 18:30」の形(秒を出さない)
    assert.match((await page.textContent("dl"))!, /2026年12月24日\(木\) 18:30/);
    assert.match((await page.textContent("dl"))!, /¥500/);
    const label = (await page.textContent("main"))!.includes("参加を申し込む") ? "参加を申し込む" : "1名で予約";
    await page.click(`button:has-text("${label}")`);
    await page.waitForFunction(() => /完了|失敗|必要|権限/.test(document.querySelector(".msg")?.textContent ?? ""), null, { timeout: 15000 });
    assert.match((await page.textContent(".msg"))!, /ログインが必要/);
    await ctx.close();

    const s = await session(kind, attendee(`user-${kind}`));
    await s.page.goto(`${webUrl}/s/${enc("イベント詳細")}/${id}`);
    assert.match((await s.page.textContent("header"))!, /ログイン中/);
    // 別のブラウザ(匿名 ID が別)なので、変種はこの画面で改めて読む
    const label2 = (await s.page.textContent("main"))!.includes("参加を申し込む") ? "参加を申し込む" : "1名で予約";
    await s.page.click(`button:has-text("${label2}")`);
    await s.page.waitForFunction(() => /完了|失敗|必要|権限/.test(document.querySelector(".msg")?.textContent ?? ""), null, { timeout: 15000 });
    const ok = (await s.page.textContent(".msg"))!;
    assert.match(ok, /完了しました/);
    // 自動採番の ID(UUID)を利用者に見せない
    assert.doesNotMatch(ok, /[0-9a-f]{8}-[0-9a-f]{4}-/);
    await s.page.click(`button:has-text("2名で予約")`);
    await s.page.waitForFunction(() => [...document.querySelectorAll(".msg")].some((m) => m.textContent?.startsWith("2名で予約")));
    assert.match((await s.page.locator(".msg", { hasText: "2名で予約" }).textContent())!, /完了しました/);
    // 残り 0 席: もう一度押すと「満席」
    await s.page.click(`button:has-text("2名で予約")`);
    await s.page.waitForFunction(() => document.querySelector(".msg.error")?.textContent?.includes("満席"));
    assert.match((await s.page.textContent("dl"))!, /予約数\s*3/);
    // 自分の予約には自分の 2 件だけ
    await s.page.goto(`${webUrl}/s/${enc("自分の予約")}`);
    assert.equal(await s.page.locator("tbody tr").count(), 2);
    // 関連データの名前表示: イベントID ではなくイベントの名前が出る
    assert.ok((await s.page.textContent("tbody"))!.includes(`少人数ワークショップ(${kind})`));
    assert.match((await s.page.textContent("thead"))!, /イベント/);
    assert.deepEqual(s.errors, []);
    assert.deepEqual(errors, []);
    await s.ctx.close();
  });
}

test("最後の 1 席を 2 人が同時に押しても、予約できるのは 1 人だけ", async () => {
  const org = token("org-race", ["イベント.create"]);
  const id = (await (await fetch(`${api}/api/actions/${enc("イベント登録")}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${org}` },
    body: JSON.stringify({ 名前: "残り1席", 会場: "福岡", 開始日時: "2026-12-25T10:00:00+09:00", 参加費: 0, 定員: 1 }),
  })).json()).ok;
  const rs = await Promise.all(
    ["a", "b", "c", "d"].map((u) =>
      fetch(`${api}/api/actions/${enc("予約する")}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${attendee("race-" + u)}` },
        body: JSON.stringify({ イベントID: id, 人数: 1 }),
      }).then(async (r) => ({ status: r.status, body: await r.json() })),
    ),
  );
  assert.equal(rs.filter((r) => r.status === 200).length, 1, JSON.stringify(rs));
  assert.ok(rs.filter((r) => r.status === 422).every((r) => r.body.value === "満席"));
});

test("主催者: 日時入力のフォームで登録 → 一覧に出る → 会場別の集計に反映", async () => {
  const { ctx, page, errors } = await session("pc", token("org-form", ["イベント.create", "イベント.read"]));
  await page.goto(`${webUrl}/s/${enc("主催者")}`);
  await page.fill(`[id="form-イベント登録-名前"]`, "年末もくもく会");
  await page.fill(`[id="form-イベント登録-会場"]`, "札幌");
  // 入力欄の日時は日本時間として扱う(ブラウザの時刻帯が違っても同じ)
  assert.match((await page.textContent(`label[for="form-イベント登録-開始日時"]`))!, /日本時間/);
  await page.fill(`[id="form-イベント登録-開始日時"]`, "2026-12-28T13:00");
  // 全角数字でも入力できる(NR-01)
  await page.fill(`[id="form-イベント登録-参加費"]`, "１，５００");
  await page.fill(`[id="form-イベント登録-定員"]`, "２０");
  await page.click("form.form button[type=submit]");
  // 送信前から空のメッセージ欄があるため、文言が入るまで待つ(負荷が高い CI でも安定させる)
  await page.waitForFunction(() => /完了|失敗|エラー|必要/.test(document.querySelector("form.form .msg")?.textContent ?? ""), null, { timeout: 15000 });
  assert.match((await page.textContent("form.form .msg"))!, /完了しました/);
  await page.waitForFunction(() => document.querySelector("main")?.textContent?.includes("札幌"), null, { timeout: 15000 });
  await page.goto(webUrl + "/");
  assert.ok((await page.textContent("main"))!.includes("年末もくもく会"));
  assert.match((await page.textContent("main"))!, /2026年12月28日\(月\) 13:00/);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test("アクセシビリティ自動監査(axe-core: WCAG 2.2 A/AA)で違反なし(PC・スマホ)", async () => {
  const axe = readFileSync(require.resolve("axe-core/axe.min.js"), "utf8");
  const results: string[] = [];
  for (const kind of ["pc", "phone"] as const) {
    const { ctx, page } = await session(kind, token("a11y", ["イベント.create", "イベント.read", "予約.create"]));
    for (const path of ["/s/" + enc("イベント一覧"), "/s/" + enc("イベント詳細") + "/e2", "/s/" + enc("自分の予約"), "/s/" + enc("主催者")]) {
      await page.goto(webUrl + path);
      await page.evaluate(axe);
      const v = await page.evaluate(async () => {
        const r = await (window as any).axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] } });
        return r.violations.map((x: any) => `${x.id}: ${x.nodes.map((n: any) => n.target.join(" ")).join(", ")}`);
      });
      results.push(...v.map((x: string) => `${kind} ${decodeURIComponent(path)} ${x}`));
    }
    await ctx.close();
  }
  assert.deepEqual(results, []);
});
