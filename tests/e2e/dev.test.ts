// はじめての人の流れ: koto init --template → koto dev → ブラウザで「名前を入れてすぐ入る」→ 予約。
// 1 つのポートで画面と API が使えること、開発用ログインが本番モードでは使えないことも確かめる。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const enc = encodeURIComponent;

let dir = "", url = "", proc: ChildProcess, browser: any;

before(async () => {
  dir = join(tmp("dev"), "教室予約");
  execFileSync(KOTO, ["init", dir, "--template", "予約受付"], { stdio: "pipe" });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "dev", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/login", 240000);
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  proc?.kill();
});

test("ひな形 → koto dev → 名前を入れてすぐ入る → 予約 → 満席 → 自分の予約", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP" });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/");
  assert.equal(await page.textContent("h1"), "枠一覧");
  await page.click("text=ログイン");
  await page.fill("#dev-name", "花子");
  await page.click("text=利用者として入る");
  await page.waitForURL(/\/s\//);
  assert.match((await page.textContent("header"))!, /ログイン中/);
  await page.click("text=午後の回");
  await page.waitForSelector("dl");
  for (const want of [/完了しました/, /完了しました/, /満席/]) {
    // 予約 API の応答と、その後の再読み込みを待ってから表示を確かめる
    const done = page.waitForResponse((r: any) => r.url().includes(enc("予約する")));
    await page.click(`button:has-text("1名で予約")`);
    await done;
    await page.waitForFunction((re: string) => new RegExp(re).test(document.querySelector(".msg")?.textContent ?? ""), want.source);
  }
  await page.goto(`${url}/s/${enc("自分の予約")}`);
  assert.equal(await page.locator("tbody tr").count(), 2);
  assert.deepEqual(errors, []);
  await ctx.close();
});

test("1 つのポートで API も使える(誰でも見られるデータ)・開発ログインは他サイトから使えない", async () => {
  const r = await fetch(`${url}/api/data/${enc("枠")}`);
  assert.equal(r.status, 200);
  assert.equal((await r.json()).length, 2);
  const x = await fetch(`${url}/login/dev`, { method: "POST", headers: { Origin: "http://evil.example", "Content-Type": "application/x-www-form-urlencoded" }, body: "name=x", redirect: "manual" });
  assert.equal(x.status, 403);
});

test("本番モードでは開発用ワンクリックログインを有効にして起動できない", async () => {
  const web = join(dir, ".koto/dev/web");
  const out = await new Promise<string>((ok) => {
    const p = spawn(process.execPath, ["--no-warnings", "src/server-main.ts"], {
      cwd: web,
      env: { ...process.env, PORT: "0", KOTONOHA_ENV: "production", KOTONOHA_DEV_JWT_SECRET: "x".repeat(64), KOTONOHA_COOKIE_SECURE: "1", KOTONOHA_DEV_LOGIN: "0" },
    });
    let s = "";
    p.stderr.on("data", (d) => (s += d));
    p.on("exit", () => ok(s));
    setTimeout(() => p.kill(), 10000);
  });
  assert.match(out, /ワンクリックログイン/);
});

test("日付型(時刻なし): 全角や / でも入力でき、存在しない日付は 400、画面は「2026年12月5日(土)」", async () => {
  const d = join(tmp("nippo"), "日報");
  execFileSync(KOTO, ["init", d, "--template", "日報"], { stdio: "pipe" });
  const port = await freePort();
  const p = spawn(KOTO, ["-C", d, "dev", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  try {
    const u = `http://127.0.0.1:${port}`;
    await waitHttp(u + "/login", 240000);
    const tok = execFileSync(KOTO, ["-C", d, "token", "--sub", "太郎"]).toString().trim();
    const post = (日付: string) =>
      fetch(`${u}/api/actions/${enc("日報を書く")}`, { method: "POST", headers: { Authorization: `Bearer ${tok}`, "Content-Type": "application/json" }, body: JSON.stringify({ 日付, 作業内容: "検品", 作業時間: 3 }) });
    assert.equal((await post("2026-12-05")).status, 200);
    assert.equal((await post("２０２６/１２/６")).status, 200);
    assert.equal((await post("2026-02-30")).status, 400);
    const rows = await (await fetch(`${u}/api/data/${enc("日報")}`, { headers: { Authorization: `Bearer ${tok}` } })).json();
    assert.deepEqual(rows.map((r: any) => r.日付).sort(), ["2026-12-05", "2026-12-06"]);
    const ctx = await browser.newContext({ locale: "ja-JP", timezoneId: "America/Los_Angeles" });
    const page = await ctx.newPage();
    await page.goto(u + "/login");
    await page.fill("#dev-name", "太郎");
    await page.click("text=一般の利用者として入る");
    await page.goto(`${u}/s/${enc("自分の日報")}`);
    // 時刻帯の違うブラウザでも暦日はずれない
    assert.match((await page.textContent("main"))!, /2026年12月5日\(土\)/);
    await ctx.close();
  } finally {
    p.kill();
  }
});
