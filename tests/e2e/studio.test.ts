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
  // 計算項目(ADR-0136)
  j = await (await api("/api/instruct", { text: "枠に「残席 = 定員 - 予約数」を追加" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    残席 = 定員 - 予約数/);
  assert.deepEqual(j.outline.decls.find((d: any) => d.kind === "data" && d.name === "枠").computed.map((c: any) => c.name), ["残席"]);
  j = await (await api("/api/instruct", { text: "枠に「だめ = 名前 - 1」を追加" })).json();
  assert.equal(j.result.ok, false, "文字列の項目は計算に使えないので入れない");
});

test("日本語の指示(第 5 回の改善): 動作のひな形・消す・列挙に値を足す・数字で始まる名前・登録できるようにしたい", async () => {
  // 状態を変える動作 + 詳細画面にボタン
  let j: any = await (await api("/api/instruct", { text: "「満席にする」で枠の予約数を99にする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /動作 満席にする\(枠ID: 識別子\)/);
  assert.match(src(), /「満席にする」を押すと 満席にする\(枠\.ID\)/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 増やす
  j = await (await api("/api/instruct", { text: "「定員を増やす」で枠の定員を1増やす" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /件\.定員 = 件\.定員 \+ 1/);
  // 登録できるようにしたい
  j = await (await api("/api/instruct", { text: "予約を登録できるようにしたい", screen: "管理" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /動作 予約を追加\(/);
  assert.match(src(), /予約を追加のフォームを表示/);
  // 列挙に値を足す
  j = await (await api("/api/instruct", { text: "予約失敗に「受付終了」を追加" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    受付終了\n/);
  // 数字で始まる名前は理由を出す
  j = await (await api("/api/instruct", { text: "枠に「1日の数」を追加" })).json();
  assert.match(j.error ?? "", /数字で始められません/);
  // 消す
  j = await (await api("/api/instruct", { text: "「定員を増やす」を消す" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.doesNotMatch(src(), /動作 定員を増やす/);
  // 呼んでいたボタンも一緒に消え、エラーが残らない
  assert.doesNotMatch(src(), /を押すと 定員を増やす/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
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

test("日本語の指示(第 6 回の改善): 名前を変える・空でもよい項目・項目を足すと登録もそろう・データをまとめて消す・取り消すで全部戻る", async () => {
  // 名前を変える(使っている所と初期データもそろえる)
  let j: any = await (await api("/api/instruct", { text: "「枠」を「レッスン」に変える" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /データ レッスン:/);
  assert.match(src(), /レッスンID: 識別子/);
  assert.doesNotMatch(src(), /枠/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  const seed = readFileSync(join(dir, "seed.json"), "utf8");
  assert.match(seed, /"レッスン"/);
  assert.doesNotMatch(seed, /"枠/);
  // 項目の名前を変える(区切りを見て、その項目だけ)
  j = await (await api("/api/instruct", { text: "レッスンの「定員」を「上限」に変える" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    上限: 整数/);
  assert.match(src(), /レッスン\.上限 - レッスン\.予約数/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 項目を足すと、その項目を登録する動作の引数もそろう
  j = await (await api("/api/instruct", { text: "レッスンに「料金」を追加" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /料金: 金額\n/);
  assert.match(src(), /動作 レッスンを追加\([^)]*料金: 金額\)/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // ボタンで呼ぶ動作は引数を増やせないので、空でもよい項目にすれば直る
  j = await (await api("/api/instruct", { text: "予約に「要望」を追加" })).json();
  assert.match(j.result.done, /誤りになりました/);
  j = await (await api("/api/instruct", { text: "「要望」を空でもよいにする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /要望: 選択<文字列>/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 動作「X」を作る(項目…): 項目を持つデータの登録になる。本人・日時は自動
  j = await (await api("/api/instruct", { text: "動作「受け付ける」を作る(レッスンID、人数)" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /予約を登録\([^)]*予約者: 本人[^)]*予約日時: 現在日時\(\)/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // データをまとめて消す → 取り消すで全部戻る
  const before = src();
  j = await (await api("/api/instruct", { text: "予約を消す" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.doesNotMatch(src(), /予約を登録|データ 予約:|予約\.作成/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  j = await (await api("/api/undo", {})).json();
  assert.equal(src(), before);
});
