// koto studio を空のフォルダで開く: 画面で「新しいアプリを作る」→ 動かす・初期データを表で直す・Excel から貼り付け・公開の準備。
// 黒い画面でコマンドを打たずに最後まで行けること(第 6 回の分析)。操作の記録(KOTONOHA_USAGE_LOG)も確かめる
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";
import { chromium, type Browser } from "playwright";

let dir = "";
let url = "";
let log = "";
let proc: ChildProcess;
let browser: Browser;

before(async () => {
  dir = join(tmp("studio-home"), "料理教室");
  mkdirSync(dir, { recursive: true });
  log = join(dir, "..", "usage.jsonl");
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, KOTONOHA_USAGE_LOG: log } });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/", 60000);
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  proc?.kill();
});

test("空のフォルダで開くと「新しいアプリを作る」。ひな形を選んで作り、動かす・初期データ・公開の準備まで画面だけでできる", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP", viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/");
  await page.waitForSelector("#welcome:not([hidden])");
  assert.equal(await page.inputValue("#welcome-name"), "料理教室");
  await page.check('input[name="tpl"][value="予約受付"]');
  await page.click('#welcome-form button[type="submit"]');
  await page.waitForSelector("#status.ok", { timeout: 60000 });
  assert.ok(existsSync(join(dir, "kotonoha.json")));
  assert.match(readFileSync(join(dir, "src/app.koto"), "utf8"), /アプリ「料理教室」/);
  assert.ok(existsSync(join(dir, "Studio を開く.command")) && existsSync(join(dir, "Studio を開く.bat")), "ダブルクリックで開くファイル");

  // 動かす・初期データ
  await page.click("#tab-run");
  await page.waitForSelector("#seed-table table");
  const first = page.locator('#seed-table input[aria-label="1 行目の名前"]');
  await first.fill("朝の回");
  await page.click("#seed-save");
  await page.waitForSelector("#seed-msg.ok");
  assert.match(readFileSync(join(dir, "seed.json"), "utf8"), /"朝の回"/);
  // Excel から貼り付け(1 行目が項目名。数は整数に、日時は日本時間に)
  await page.fill("#seed-paste", "名前\t開始日時\t定員\t予約数\n夜の回\t2026-12-05 19:00\t8\t0\n昼の回\t2026-12-06 12:00\t4\t0");
  await page.click("#seed-paste-apply");
  for (let i = 0; i < 100 && !readFileSync(join(dir, "seed.json"), "utf8").includes("夜の回"); i++) await page.waitForTimeout(100);
  const seed = JSON.parse(readFileSync(join(dir, "seed.json"), "utf8"));
  assert.deepEqual(seed["枠"].map((r: any) => [r.名前, r.定員, r.開始日時]), [
    ["夜の回", 8, "2026-12-05T19:00:00+09:00"],
    ["昼の回", 4, "2026-12-06T12:00:00+09:00"],
  ]);
  // 整数でない値は分かる言葉で断る
  await page.fill("#seed-paste", "名前\t定員\nだめ\tたくさん");
  await page.click("#seed-paste-apply");
  await page.waitForSelector("#seed-msg.err");
  assert.match(await page.textContent("#seed-msg") ?? "", /整数/);
  // 別のデータの表(項目名が合わない)は入れずに、切り替えるよう案内する
  await page.fill("#seed-paste", "枠ID\t予約者\t人数\nx\ty\t1");
  await page.click("#seed-paste-apply");
  await page.waitForFunction(() => /には項目「枠ID」「予約者」「人数」がありません.*「予約」に切り替えて/.test(document.querySelector("#seed-msg")?.textContent ?? ""));
  assert.match(readFileSync(join(dir, "seed.json"), "utf8"), /"夜の回"/, "前の行は残る");
  // seed.json のような JSON をそのまま貼っても入る(表に書き直していた。第 13 回評価)
  await page.fill("#seed-paste", JSON.stringify({ 枠: [{ ID: "j1", 名前: "JSON の回", 開始日時: "2026-12-07T10:00:00+09:00", 定員: 5, 予約数: 0 }] }));
  await page.click("#seed-paste-apply");
  await page.waitForFunction(() => /JSON から 1 行入れました/.test(document.querySelector("#seed-msg")?.textContent ?? ""));
  assert.match(readFileSync(join(dir, "seed.json"), "utf8"), /"JSON の回"/);
  // 無いデータの名前の JSON は入れない
  await page.fill("#seed-paste", JSON.stringify({ 会員: [{ 名前: "x" }] }));
  await page.click("#seed-paste-apply");
  await page.waitForFunction(() => /データ「会員」はありません/.test(document.querySelector("#seed-msg")?.textContent ?? ""));
  // 列の数が合わない行は断る。タブ区切りなら「1,200」のカンマで列がずれない
  await page.fill("#seed-paste", "名前\t定員\nずれ\t3\t余分");
  await page.click("#seed-paste-apply");
  await page.waitForFunction(() => /列の数/.test(document.querySelector("#seed-msg")?.textContent ?? ""));

  // アプリを開く(動いていれば URL が入る)
  await page.waitForFunction(() => !!document.querySelector("#run-open")?.getAttribute("href"), null, { timeout: 120000 });
  // 公開の準備
  await page.click("#publish");
  await page.waitForSelector("#publish-msg.ok", { timeout: 180000 });
  assert.ok(existsSync(join(dir, "公開用", "公開の手順.md")) && existsSync(join(dir, "公開用", "start.sh")));
  assert.match(readFileSync(join(dir, "公開用", ".env"), "utf8"), /KOTONOHA_ENV=production\nKOTONOHA_JWT_SECRET=[0-9a-f]{96}/);
  assert.match(await page.textContent("#publish-check") ?? "", /公開前の点検/);
  assert.deepEqual(errors, []);
  await ctx.close();

  // 操作の記録: Studio の操作は残り、koto のコマンドは studio の 1 回だけ
  const rows = readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  assert.ok(rows.some((r) => r.kind === "studio" && r.api === "create" && r.ok));
  assert.ok(rows.some((r) => r.kind === "studio" && r.api === "seed"));
  assert.ok(rows.some((r) => r.kind === "studio" && r.api === "publish" && r.ok));
  assert.equal(rows.filter((r) => r.kind === "cli").length, 0, "Studio の中から呼んだ koto は記録しない(studio 自体は止めたときに記録される)");
});
