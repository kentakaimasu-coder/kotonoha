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
  // 注釈(# …)は変えない
  assert.doesNotMatch(src().split("\n").filter((l) => !l.trim().startsWith("#")).join("\n"), /枠/);
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
  // ボタンで登録している動作がある(1名で予約)ので、空でもよい項目として足す(誤りを残さない)
  j = await (await api("/api/instruct", { text: "予約に「要望」を追加" })).json();
  assert.match(j.result.done, /空でもよい項目にしました/);
  assert.match(src(), /要望: 選択<文字列>/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 空でもよい項目にする(登録の動作の引数もそろう)
  j = await (await api("/api/instruct", { text: "レッスンに「講師」を追加" })).json();
  j = await (await api("/api/instruct", { text: "「講師」を空でもよいにする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /講師: 選択<文字列>/);
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

test("日本語の指示(第 7 回の改善): 権限・役割・型・項目を消す・列挙の値・アプリ名・読めない文の案内", async () => {
  // 前のテストで「枠」は「レッスン」、「予約」は残っている(取り消したので)
  let j: any = await (await api("/api/instruct", { text: "受付係に予約の更新を許可する" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /役割 受付係:\n    権限 = \[[^\]]*予約\.更新/);
  j = await (await api("/api/instruct", { text: "役割「店長」を作る" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /役割 店長:\n    権限 = \[[^\]]*読取/);
  // 型を変える(登録の動作の引数もそろう)。同じ型なら名前の変更
  // 「…にする」は型の変更だけ。型でない言葉なら、名前の変え方を案内する(第 7 回評価: 型のつもりが名前が変わった)
  j = await (await api("/api/instruct", { text: "予約の「人数」を「数」にする" })).json();
  assert.match(j.error ?? "", /「数」という型はありません.*に変える/);
  j = await (await api("/api/instruct", { text: "予約の「人数」を「数」に変える" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    数: 整数/);
  // 項目を消す(登録の行からも外す。「人数」の引数は壊さない)
  j = await (await api("/api/instruct", { text: "予約の数を消す" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.doesNotMatch(src(), /    数: 整数/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 列挙の値の名前を変える(失敗("満席") もそろう)
  j = await (await api("/api/instruct", { text: "列挙「予約失敗」の「満席」を「満員」に変える" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /失敗\("満員"\)/);
  assert.doesNotMatch(src(), /"満席"/);
  // アプリの名前
  j = await (await api("/api/instruct", { text: "アプリの名前を「料理教室」にする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /アプリ「料理教室」/);
  // 失敗しない動作は、使わない失敗の列挙を作らない
  j = await (await api("/api/instruct", { text: "「満員にする」でレッスンの予約数を9にする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /動作 満員にする\(レッスンID: 識別子\) -> 識別子:/);
  assert.doesNotMatch(src(), /満員にするの失敗/);
  // 見られる人の約束は、前の約束を置き換える(矛盾させない)
  j = await (await api("/api/instruct", { text: "レッスンをログインした人だけにする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /レッスンはログインした人だけ閲覧可/);
  assert.doesNotMatch(src(), /レッスンは誰でも閲覧可/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 読めない文は「どの画面?」と聞かず、言える言い方を示す
  j = await (await api("/api/instruct", { text: "いい感じにして" })).json();
  assert.equal(j.result.ok, false);
  assert.ok(j.result.suggestions.length >= 3, JSON.stringify(j.result));
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
});

test("日本語の指示(第 8 回の改善): 決まった名前の集計・権限の名前をそのまま・項目を消すと確かめるもそろう", async () => {
  let j: any = await (await api("/api/instruct", { text: "レッスンに「会場」を追加" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  // 必須の項目を足すと、初期データの行にも仮の値が入る(入れないとアプリが起動できなかった。第 7 回評価)
  const seed = JSON.parse(readFileSync(join(dir, "seed.json"), "utf8"));
  assert.ok(seed["レッスン"].length > 0 && seed["レッスン"].every((r: any) => r.会場 === "未設定"), JSON.stringify(seed));
  // 名前から型を決める(単価は金額、納期は日付)
  j = await (await api("/api/instruct", { text: "レッスンに「単価」を追加" })).json();
  assert.match(src(), /    単価: 金額/);
  j = await (await api("/api/instruct", { text: "レッスンの単価を消す" })).json();
  // 決まった名前・列の集計(分析)を日本語で
  j = await (await api("/api/instruct", { text: "分析「会場別の状況」を作る(レッスンの予約数を会場ごとに合計、列: 会場・回数・予約数)" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /分析 会場別の状況\(\) -> 一覧<記録\{会場: 文字列, 回数: 整数, 予約数: 整数\}>:\n    集計 = "レッスンの予約数を会場ごとに合計"/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  j = await (await api("/api/instruct", { text: "分析「だめ」を作る(レッスンの予約数を会場ごとに合計、列: 会場・回数)" })).json();
  assert.match(j.error ?? "", /列は 3 個です/);
  // 権限の名前をそのまま書く
  j = await (await api("/api/instruct", { text: "店長に全件閲覧.予約を許可する" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /役割 店長:\n    権限 = \[[^\]]*全件閲覧\.予約/);
  // 集計に使っていない項目を消すと、確かめるの呼び出しの値も外れる
  const call = () => /レッスンを追加\(("確かめる回"[^)]*)\)/.exec(src())![1].split(",").length;
  const n0 = call();
  j = await (await api("/api/instruct", { text: "レッスンに「備考」を追加" })).json();
  assert.equal(call(), n0 + 1, "確かめるの呼び出しに値が足される");
  j = await (await api("/api/instruct", { text: "レッスンの備考を消す" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.equal(call(), n0, "消すと値も外れる");
  // 詳細画面を作ると、一覧の画面に「選ぶと 詳細へ」が自動で入る(誤りを残さない)
  j = await (await api("/api/instruct", { text: "画面「予約詳細」を作る(予約の予約者と予約日時を表示)" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /予約を選ぶと 予約詳細 へ/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 画面の行を上へ動かす
  const lsBefore = src().split("\n");
  const k = lsBefore.findIndex((l) => l.trim() === "予約を選ぶと 予約詳細 へ");
  const prev = lsBefore[k - 1];
  j = await (await api("/api/op", { op: "move_line", file: "src/app.koto", line: k + 1, old: lsBefore[k], dir: "up" })).json();
  assert.equal(j.ok, true, j.error);
  const lsAfter = src().split("\n");
  assert.equal(lsAfter[k - 1].trim(), "予約を選ぶと 予約詳細 へ");
  assert.equal(lsAfter[k], prev);
  // 画面の最後の 1 行は消さない(画面が壊れて足し直す欄も無くなった)
  j = await (await api("/api/instruct", { text: "画面「お知らせ」を作る(レッスンを一覧表示)" })).json();
  const ls2 = src().split("\n");
  const k2 = ls2.findIndex((l) => l.trim() === "画面 お知らせ:") + 1;
  j = await (await api("/api/op", { op: "remove_line", file: "src/app.koto", line: k2 + 1, old: ls2[k2] })).json();
  assert.match(j.error ?? "", /最後の 1 行なので消せません/);
  // 引数に「文字列(任意)」がある動作に項目を足しても、カッコの中に差し込まない
  j = await (await api("/api/instruct", { text: "データ「申込」を作る(名前: 文字列、要望: 文字列(任意))" })).json();
  j = await (await api("/api/instruct", { text: "「申し込む」で申込を登録する" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  j = await (await api("/api/instruct", { text: "申込に「人数」を追加" })).json();
  assert.match(src(), /動作 申し込む\(名前: 文字列, 要望: 選択<文字列>, 人数: 整数\)|動作 申し込む\(名前: 文字列, 要望: 文字列\(任意\), 人数: 整数\)/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 同じデータの一覧に並び順を足すと、一覧を置き換える(2 つにしない)。画面を選んでいなくても、出している画面が 1 つなら入れる
  j = await (await api("/api/instruct", { text: "画面「申込一覧」を作る(申込を一覧表示)" })).json();
  j = await (await api("/api/instruct", { text: "申込を名前順に一覧表示" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /画面 申込一覧:\n    申込を名前順に一覧表示\n/);
  assert.doesNotMatch(src(), /    申込を一覧表示\n/);
  // 宣言を何行か貼り付けると、そのままソースに入る
  j = await (await api("/api/instruct", { text: "分析 申込の数() -> 一覧<記録{名前: 文字列, 件数: 整数, 人数: 整数}>:\n    集計 = \"申込を名前ごとに集計\"" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /分析 申込の数\(\) -> 一覧<記録\{名前: 文字列, 件数: 整数, 人数: 整数\}>:\n    集計 = "申込を名前ごとに集計"/);
  // 日本語の「…ごとに集計」も、件数と整数の合計の列を作る
  j = await (await api("/api/instruct", { text: "分析「申込の集計」を作る(申込を名前ごとに集計)" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /分析 申込の集計\(\) -> 一覧<記録\{名前: 文字列, 申込数: 整数, 人数の合計: 整数\}>/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
});

test("日本語の指示(第 7 回評価の残り): 全件閲覧の言い方・新しい順・型を列挙にすると確かめるの値もそろう・表示はボタンの上", async () => {
  // 全件閲覧を日本語で
  let j: any = await (await api("/api/instruct", { text: "店長にレッスンの全件閲覧を許可する" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /役割 店長:\n    権限 = \[[^\]]*全件閲覧\.レッスン/);
  j = await (await api("/api/instruct", { text: "店長は全員の申込を見られる" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /役割 店長:\n    権限 = \[[^\]]*全件閲覧\.申込/);
  // 「新しい順」だけでも、日時の項目で並べる
  j = await (await api("/api/instruct", { text: "予約を新しい順に一覧表示", screen: "自分の予約" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /予約を予約日時の新しい順に一覧表示/);
  // 型を列挙にすると、確かめるの "" も列挙の値になる
  j = await (await api("/api/instruct", { text: "レッスンに「天気」を追加" })).json();
  assert.match(src(), /レッスンを追加\("確かめる回"[^)]*""\)/);
  j = await (await api("/api/instruct", { text: "列挙「天気の種類」を作る(晴れ、曇り、雨)" })).json();
  j = await (await api("/api/instruct", { text: "レッスンの「天気」を「天気の種類」にする" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /    天気: 天気の種類/);
  assert.match(src(), /レッスンを追加\("確かめる回"[^)]*"晴れ"\)/);
  assert.equal(j.outline.ok, true, JSON.stringify(j.outline.diagnostics));
  // 同じ型にする指示は、そのままと答える
  j = await (await api("/api/instruct", { text: "レッスンの「天気」を「天気の種類」にする" })).json();
  assert.match(j.result.done, /既に 天気の種類/);
  // 詳細画面の「…を表示」は、ボタンの上に入る
  j = await (await api("/api/instruct", { text: "レッスンの天気を表示", screen: "レッスン詳細" })).json();
  assert.equal(j.result.ok, true, JSON.stringify(j.result));
  assert.match(src(), /画面 レッスン詳細:\n(    .*\n)*    レッスンの天気を表示\n    「1名で予約」を押すと/);
});

test("集計を画面のフォームで作る(記号を書かない)・項目の順番を変える", async () => {
  const ctx = await browser.newContext({ locale: "ja-JP", viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/");
  await page.click("text=+ 新しい集計");
  await page.fill('input[aria-label="集計の名前"]', "会場別の予約状況");
  await page.selectOption('select[aria-label="集計するデータ"]', "レッスン");
  await page.selectOption('select[aria-label="まとめる項目"]', "会場");
  // 合計は予約数だけにする
  for (const box of await page.locator('#editor input[type="checkbox"]').all()) if ((await box.getAttribute("value")) !== "予約数") await box.uncheck();
  await page.fill('input[aria-label="2 列目の名前"]', "イベント数");
  await page.click('#editor button:has-text("作る")');
  await page.waitForSelector('h2:has-text("集計「会場別の予約状況」")');
  assert.match(src(), /分析 会場別の予約状況\(\) -> 一覧<記録\{会場: 文字列, イベント数: 整数, 予約数: 整数\}>/);
  // 項目の順番(一覧の列の順番)を変える
  await page.click('#tree button:text-is("レッスン")');
  const order = () => /データ レッスン:\n([\s\S]*?)\n\n/.exec(src())![1].split("\n").map((l) => l.trim().split(":")[0]);
  const o0 = order();
  const k = o0.indexOf("会場");
  await page.click(`button[aria-label="会場を上へ"]`);
  for (let i = 0; i < 50 && order().indexOf("会場") === k; i++) await page.waitForTimeout(100);
  assert.equal(order().indexOf("会場"), k - 1);
  assert.deepEqual(errors, []);
  await ctx.close();
});
