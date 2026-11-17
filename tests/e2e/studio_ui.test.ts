// Studio をブラウザで操作して、ペルソナが通る道を確かめる(第 11 回評価: 画面の足す欄の例の文がそのまま通らず ✗ になった)。
// 欄に例として出している文は、その欄にそのまま入れて通らなければならない
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
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

test("ひな形「予約受付」のまま「予約者」を「参加者」に変えられる(課題の項目名。第 12 回評価で通らなかった)", async () => {
  const token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(url + "/")).text())![1];
  const r: any = await (await fetch(url + "/api/instruct", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ text: "予約の「予約者」を「参加者」に変える" }) })).json();
  assert.equal(r.result?.ok, true, JSON.stringify(r));
  assert.match(src(), /予約のうち参加者が本人のものを一覧表示/);
  assert.doesNotMatch(src().replace(/#.*$/gm, ""), /予約者/);
  assert.equal(r.outline.ok, true, JSON.stringify(r.outline.diagnostics));
});

test("課題 A の道筋(ペルソナがする順): 名前を変える・項目を足す・ボタン・集計・項目の順番。どの一歩でも検査が ✗ にならない", async () => {
  const d2 = join(tmp("studio-ui-a"), "イベント受付");
  execFileSync(KOTO, ["init", d2, "--template", "予約受付"], { stdio: "pipe" });
  const port = await freePort();
  const u2 = `http://127.0.0.1:${port}`;
  const p2 = spawn(KOTO, ["-C", d2, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  try {
    await waitHttp(u2 + "/", 60000);
    const token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(u2 + "/")).text())![1];
    const say = async (text: string, screen?: string) => {
      const r: any = await (await fetch(u2 + "/api/instruct", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ text, screen }) })).json();
      assert.equal(r.result?.ok, true, `${text}: ${JSON.stringify(r.error ?? r.result)}`);
      assert.equal(r.outline.ok, true, `${text}: ${JSON.stringify(r.outline.diagnostics)}`);
      return r;
    };
    await say("「枠」を「イベント」に変える");
    await say("予約の「予約者」を「参加者」に変える");
    await say("「受付係」を「主催者」に変える");
    await say("「イベントを追加」を「イベント登録」に変える");
    await say("イベントに「会場」を追加");
    await say("イベントに「参加費」を追加");
    await say("「2名で予約」ボタンを足す", "イベント詳細");
    await say("「3つで予約」ボタンを足す", "イベント詳細");
    await say("イベントの項目を 名前・会場・開始日時・参加費・定員・予約数 の順にする");
    // 順番の約束があれば、状態を進めるボタンは前の状態のときだけ出す
    await say("列挙「進み」を作る(受付、作業中、完了)");
    await say("データ「作業」を作る(名前: 文字列、進み: 進み)");
    await say("作業の進みは 受付 → 作業中 → 完了 の順");
    await say("作業を一覧表示", "イベント一覧");
    await say("「始める」で作業の進みを「作業中」にする");
    assert.match(readFileSync(join(d2, "src/app.koto"), "utf8"), /作業の進みが受付のとき「始める」を押すと 始める\(作業\.ID\)/);
    // 役割を作った後に足したデータも、役割の人が読める
    await say("データ「回覧」を作る(題名: 文字列)");
    assert.match(readFileSync(join(d2, "src/app.koto"), "utf8"), /役割 利用者:\n    権限 = \[[^\]]*回覧\.読取/);
    // 最初に出る画面を変える
    await say("最初の画面を「自分の予約」にする");
    assert.ok(readFileSync(join(d2, "src/app.koto"), "utf8").indexOf("画面 自分の予約:") < readFileSync(join(d2, "src/app.koto"), "utf8").indexOf("画面 イベント一覧:"));
    // 既にある動作で、ほかのデータの数を変える(回数券を減らす など)
    await say("データ「回数券」を作る(残り回数: 整数)");
    await say("「予約する」で回数券を選べるようにしたい");
    await say("「予約する」で回数券の残り回数を1減らす");
    await say("分析「会場別の予約状況」を作る(イベントの予約数と定員を会場ごとに合計)");
    const s = readFileSync(join(d2, "src/app.koto"), "utf8");
    assert.match(s, /データ イベント:\n    ID: 識別子\n    名前: 文字列\n    会場: 文字列\n    開始日時: 日時\n    参加費: 金額/);
    assert.match(s, /動作 イベント登録\(名前: 文字列, 会場: 文字列, 開始日時: 日時, 参加費: 金額/);
    assert.match(s, /「2名で予約」を押すと 予約する\(イベント\.ID, 2(, [^)]*)?\)/);
    assert.match(s, /「3つで予約」を押すと 予約する\(イベント\.ID, 3(, [^)]*)?\)/);
    assert.match(s, /参加者: 本人/);
    assert.match(s, /    対象 = 回数券を取得\(回数券ID\)\n(?:.*\n)*?    対象\.残り回数 >= 1 を確認 または 失敗\("残り回数不足"\)\n(?:.*\n)*?    対象\.残り回数 = 対象\.残り回数 - 1\n(?:.*\n)*?.*を返す/);
    assert.doesNotMatch(s.replace(/#.*$/gm, ""), /予約者|受付係|枠/);
    // 説明書き(# の行)の役割の名前もそろう。名前の変え方の例の行はそのまま
    assert.match(s, /# 役割: [^\n]*「主催者として入る」/);
    assert.match(s, /「「枠」を「イベント」に変える」/);
  } finally {
    p2.kill();
  }
});

test("ADR-0140: 役割のあるアプリでは、確かめるで役割を書かない人は、どの役割にも書いていない権限だけを持つ(本番と同じ)", async () => {
  const d3 = join(tmp("adr0140"), "受付");
  execFileSync(KOTO, ["init", d3, "--template", "予約受付"], { stdio: "pipe" });
  const p = join(d3, "src/app.koto");
  const s = readFileSync(p, "utf8").replace(" → 成功(回)\n", " → 成功(回)\n    次郎が 予約する(回, 1) → 成功\n");
  writeFileSync(p, s);
  let out = "";
  try {
    out = execFileSync(KOTO, ["-C", d3, "test"], { stdio: "pipe", encoding: "utf8" });
  } catch (e: any) {
    out = String(e.stdout ?? "") + String(e.stderr ?? "");
  }
  assert.match(out, /✗ 次郎が 予約する\(回, 1\) → 成功.*役割を書かない人は、どの役割にも書いていない権限だけを持ちます/);
});

test("空でもよくすると、外した確認の失敗を確かめていた行は「→ 成功」にそろう(第 13 回評価)", async () => {
  const d4 = join(tmp("studio-optional"), "受付");
  execFileSync(KOTO, ["init", d4], { stdio: "pipe" });
  writeFileSync(
    join(d4, "src/app.koto"),
    [
      "アプリ「受付」",
      "",
      "列挙 要望の失敗:",
      "    内容が空",
      "",
      "データ 要望:",
      "    ID: 識別子",
      "    内容: 文字列",
      "",
      "動作 要望を出す(内容: 文字列) -> 結果<識別子, 要望の失敗>:",
      '    内容 != "" を確認 または 失敗("内容が空")',
      "    要望を登録(内容: 内容) を返す",
      "",
      "画面 要望一覧:",
      "    要望を一覧表示",
      "",
      "約束:",
      "    要望は誰でも閲覧可",
      "",
      "確かめる 要望の確かめ:",
      '    花子が 要望を出す("") → 失敗("内容が空")',
      "",
    ].join("\n"),
  );
  writeFileSync(join(d4, "seed.json"), "{}");
  const port = await freePort();
  const u4 = `http://127.0.0.1:${port}`;
  const p4 = spawn(KOTO, ["-C", d4, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  try {
    await waitHttp(u4 + "/", 60000);
    const token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(u4 + "/")).text())![1];
    const r: any = await (await fetch(u4 + "/api/instruct", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ text: "「内容」を空でもよいにする" }) })).json();
    assert.equal(r.result?.ok, true, JSON.stringify(r));
    const s4 = readFileSync(join(d4, "src/app.koto"), "utf8");
    assert.doesNotMatch(s4, /内容 != ""/);
    assert.match(s4, /花子が 要望を出す\(""\) → 成功/);
    assert.equal(r.outline.ok, true, JSON.stringify(r.outline.diagnostics));
  } finally {
    p4.kill();
  }
});
