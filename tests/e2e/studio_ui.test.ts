// Studio をブラウザで操作して、ペルソナが通る道を確かめる(第 11 回評価: 画面の足す欄の例の文がそのまま通らず ✗ になった)。
// 欄に例として出している文は、その欄にそのまま入れて通らなければならない
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";
import { chromium, type Browser } from "playwright";

let dir = "";
let url = "";
let proc: ChildProcess;
let browser: Browser;

before(async () => {
  dir = join(tmp("studio-ui"), "イベント受付");
  execFileSync(KOTO, ["init", dir, "--template", "予約受付"], { stdio: "pipe" });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/", 60000);
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  proc?.kill();
});

const src = () => readFileSync(join(dir, "src/app.koto"), "utf8");

test("画面の足す欄: 例として出している文をそのまま入れると通る。読めない文はソースに入れない", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP", viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/");
  await page.click('#tree button:has-text("枠詳細")');
  const box = page.locator('#editor input[aria-label="足す文"]');
  await box.waitFor();
  // 欄の例(placeholder)の後ろの部分「「2名で予約」ボタンを足す」をそのまま入れる
  const ph = (await box.getAttribute("placeholder"))!;
  const example = ph.split(" / ").pop()!.trim();
  assert.equal(example, "「2名で予約」ボタンを足す");
  await box.fill(example);
  await box.press("Enter");
  await page.waitForFunction(() => /2名で予約/.test(document.querySelector("#editor")?.textContent ?? ""), null, { timeout: 15000 });
  assert.match(src(), /「2名で予約」を押すと 予約する\(枠\.ID, 2\)/);
  // 前の例(データの一覧)もそのまま通る
  const first = ph.replace(/^例:\s*/, "").split(" / ")[0].trim();
  await page.locator('#editor input[aria-label="足す文"]').fill(first);
  await page.locator('#editor input[aria-label="足す文"]').press("Enter");
  await page.waitForFunction(() => /枠を.*新しい順に一覧表示/.test(document.querySelector("#editor")?.textContent ?? ""), null, { timeout: 15000 });
  assert.equal(first, "枠を新しい順に一覧表示");
  // 読めない文は入れずに、理由と近い書き方を出す(ソースは変わらない)
  const before = src();
  await page.locator('#editor input[aria-label="足す文"]').fill("いい感じのボタン");
  await page.locator('#editor input[aria-label="足す文"]').press("Enter");
  await page.waitForSelector("#editor .msg.err");
  assert.match((await page.textContent("#editor .msg.err"))!, /読めませんでした/);
  assert.equal(src(), before);
  // 検査は ✗ にならない
  await page.waitForFunction(() => !/✗/.test(document.querySelector("#status")?.textContent ?? "✗"), null, { timeout: 30000 });
  assert.deepEqual(errors, []);
  await ctx.close();
});
