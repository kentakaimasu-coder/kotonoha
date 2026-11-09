// koto studio(ADR-0135): 画面で作る・日本語で指示する・機種ごとの見た目の確認。
// 変更はソースに文として入り、読めない文は元に戻して近い書き方を返す。他のサイトからは操作できない
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";
import { chromium, type Browser } from "playwright";
import { request } from "node:http";

let dir = "";
let url = "";
let token = "";
let proc: ChildProcess;
let browser: Browser;

before(async () => {
  dir = join(tmp("studio"), "教室予約");
  execFileSync(KOTO, ["init", dir, "--template", "予約受付"], { stdio: "pipe" });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/", 60000);
  token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(url + "/")).text())![1];
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  proc?.kill();
});

const api = (path: string, body: unknown, headers: Record<string, string> = {}) =>
  fetch(url + path, { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token, ...headers }, body: JSON.stringify(body) });
const src = () => readFileSync(join(dir, "src/app.koto"), "utf8");

test("日本語の指示: データを作る・項目を足す・画面と約束に文を入れる。読めない文は戻して近い書き方を返す", async () => {
  let j: any = await (await api("/api/instruct", { text: "データ「教室」を作る(名前: 文字列、定員: 数)" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /データ 教室:\n    ID: 識別子\n    名前: 文字列\n    定員: 整数/);
  j = await (await api("/api/instruct", { text: "教室に「場所」を追加" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    場所: 文字列/);
  // 画面の文は入れ先(選んでいる画面)が要る
  j = await (await api("/api/instruct", { text: "教室を名前順に一覧表示" })).json();
  assert.equal(j.result.need, "screen");
  j = await (await api("/api/instruct", { text: "教室を名前順に一覧表示", screen: "枠一覧" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.equal(j.outline.ok, true);
  const before = src();
  j = await (await api("/api/instruct", { text: "教室をいい感じに並べて", screen: "枠一覧" })).json();
  assert.equal(j.result.ok, false);
  assert.ok(j.result.suggestions.includes("教室を一覧表示"), JSON.stringify(j.result));
  assert.equal(src(), before, "読めない文はソースに残さない");
  j = await (await api("/api/instruct", { text: "教室は誰でも閲覧可" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /約束:[\s\S]*教室は誰でも閲覧可/);
  // 取り消す
  j = await (await api("/api/undo", {})).json();
  assert.equal(j.ok, true);
  assert.doesNotMatch(src(), /教室は誰でも閲覧可/);
});

test("画面の操作: 役割を作る・権限を選ぶ・行を消す。名前と型を確かめる", async () => {
  let j: any = await (await api("/api/op", { op: "add_role", name: "先生", caps: ["枠.読取", "枠.作成", "動作.枠を追加"] })).json();
  assert.equal(j.ok, true, j.error);
  assert.match(src(), /役割 先生:\n    権限 = \[枠\.読取, 枠\.作成, 動作\.枠を追加\]/);
  assert.equal(j.outline.ok, true);
  j = await (await api("/api/op", { op: "add_field", data: "枠", name: "場所 です", type: "文字列" })).json();
  assert.match(j.error, /使えません/);
  j = await (await api("/api/op", { op: "add_field", data: "枠", name: "部屋", type: "なにか" })).json();
  assert.match(j.error, /型「なにか」は分かりません/);
  // ソースが変わっていれば消さない(古い画面からの操作を防ぐ)
  j = await (await api("/api/op", { op: "remove_line", file: "src/app.koto", line: 1, old: "違う行" })).json();
  assert.match(j.error, /ソースが変わっています/);
  // プロジェクトの外は書けない
  j = await (await api("/api/op", { op: "save_source", file: "../outside.koto", text: "x" })).json();
  assert.match(j.error, /プロジェクトの外/);
});

test("安全: 合言葉の無い書き込み・別の Host・JSON でない要求は拒否する", async () => {
  assert.equal((await api("/api/undo", {}, { "X-Studio-Token": "wrong" })).status, 403);
  assert.equal((await fetch(url + "/api/undo", { method: "POST", headers: { "Content-Type": "text/plain", "X-Studio-Token": token }, body: "{}" })).status, 403);
  // 別の Host(DNS リバインディング)は拒否する。fetch は Host を変えられないので http.request で送る
  const status = await new Promise<number>((ok, ng) => {
    const u = new URL(url);
    const r = request({ host: u.hostname, port: u.port, path: "/api/state", headers: { Host: "evil.example" } }, (res) => {
      res.resume();
      ok(res.statusCode ?? 0);
    });
    r.on("error", ng);
    r.end();
  });
  assert.equal(status, 421);
  const h = await fetch(url + "/");
  assert.match(h.headers.get("content-security-policy") ?? "", /frame-ancestors 'none'/);
});

test("見た目の確認: iPhone・Android の各機種の大きさで、生成したアプリを並べて表示する", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP", viewport: { width: 1440, height: 1000 } });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/");
  await page.waitForSelector("#status.ok");
  await page.click("#tab-look");
  await page.waitForSelector("#devices iframe", { timeout: 120000 });
  const titles = await page.$$eval("#devices iframe", (fs) => fs.map((f) => (f as HTMLIFrameElement).title));
  assert.ok(titles.some((t) => t.startsWith("iPhone")) && titles.some((t) => t.startsWith("Galaxy") || t.startsWith("Pixel")), titles.join(","));
  // 生成したアプリが枠の中に表示される(開発中だけ Studio からの表示を許す)
  const st: any = await (await fetch(url + "/api/state")).json();
  await waitHttp(`http://127.0.0.1:${st.previewPort}/`, 240000);
  const pr = await fetch(`http://127.0.0.1:${st.previewPort}/`);
  assert.match(pr.headers.get("content-security-policy") ?? "", new RegExp(`frame-ancestors http://127\\.0\\.0\\.1:${new URL(url).port}`));
  await page.click("#look-reload");
  let frame;
  for (let i = 0; i < 120 && !frame; i++) {
    frame = page.frames().find((f) => f.url().includes(`:${st.previewPort}/`));
    if (!frame) await page.waitForTimeout(500);
  }
  assert.ok(frame, "機種の枠の中に生成したアプリ");
  await frame!.waitForSelector("h1", { timeout: 60000 });
  assert.equal(await frame!.textContent("h1"), "枠一覧");
  assert.deepEqual(errors, []);
  await ctx.close();
});
