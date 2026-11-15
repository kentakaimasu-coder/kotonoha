// 「予約のうち枠IDがこの枠のものを一覧表示」: 詳細の画面で、開いている枠の予約だけを出す。
// 文字どおりの「この枠」で絞り込み、いつも「データがありません」になっていた(第 8 回評価)
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

let dir = "", url = "", proc: ChildProcess, browser: any;

before(async () => {
  dir = join(tmp("this-filter"), "教室予約");
  execFileSync(KOTO, ["init", dir, "--template", "予約受付"], { stdio: "pipe" });
  const app = join(dir, "src/app.koto");
  writeFileSync(app, readFileSync(app, "utf8").replace("    枠の名前と開始日時と定員と予約数を表示\n", "    枠の名前と開始日時と定員と予約数を表示\n    予約のうち枠IDがこの枠のものを一覧表示\n"));
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

test("詳細の画面の「この枠」の一覧は、開いている枠の予約だけを出す", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP" });
  const page = await ctx.newPage();
  await page.goto(url + "/login");
  await page.fill("#dev-name", "花子");
  await page.click("text=利用者として入る");
  await page.waitForURL(/\/s\//);
  await page.goto(url + "/");
  await page.click("text=午前の回");
  await page.waitForSelector("dl");
  const done = page.waitForResponse((r: any) => r.url().includes(encodeURIComponent("予約する")));
  await page.click(`button:has-text("1名で予約")`);
  await done;
  await page.reload();
  await page.waitForSelector("dl");
  assert.equal(await page.locator("tbody tr").count(), 1, "予約した枠では 1 件");
  await page.goto(url + "/");
  await page.click("text=午後の回");
  await page.waitForSelector("dl");
  assert.equal(await page.locator("tbody tr").count(), 0, "ほかの枠では 0 件");
  await ctx.close();
});
