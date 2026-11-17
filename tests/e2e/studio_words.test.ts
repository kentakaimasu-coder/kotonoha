// Studio の日本語の指示: 評価で読めなかった言い方を、その言い方のまま通す(第 14 回評価)。
// 確かめるの見出しごと 1 行、並べ替え、画面に集計を出す、かぎかっこ無しの受け取る値、選択肢を消す、取り消しの 2 回目
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, waitHttp } from "./helpers.ts";

let dir = "";
let url = "";
let token = "";
let proc: ChildProcess;

before(async () => {
  dir = join(tmp("studio-words"), "受付");
  execFileSync(KOTO, ["init", dir, "--template", "予約受付"], { stdio: "pipe" });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/", 60000);
  token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(url + "/")).text())![1];
});
after(() => {
  proc?.kill();
});

const say = async (text: string, extra: Record<string, string> = {}) => {
  const j: any = await (
    await fetch(url + "/api/instruct", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ text, ...extra }) })
  ).json();
  return j;
};
const ok = async (text: string, extra: Record<string, string> = {}) => {
  const j = await say(text, extra);
  assert.equal(j.result?.ok, true, `${text}: ${JSON.stringify(j.result ?? j).slice(0, 400)}`);
  assert.equal(j.outline?.ok, true, `${text}: 検査が ✗ になった`);
  return j.result;
};
const src = () => readFileSync(join(dir, "src/app.koto"), "utf8");

test("評価で読めなかった言い方が通り、検査が ✗ にならない", async () => {
  // 並べ替え: 一覧のある画面が 1 つなら聞かずにそこで置き換える
  await ok("枠を開始日時の順に並べる");
  assert.match(src(), /\n    枠を開始日時順に一覧表示\n/);
  await ok("枠を定員が多い順に並べたい");
  assert.match(src(), /\n    枠を定員の新しい順に一覧表示\n/);
  assert.doesNotMatch(src(), /\n    枠を開始日時順に一覧表示\n/);
  // 画面の名前を書いて集計を出す
  const r = await ok("分析「枠別」を作る(枠の定員を名前ごとに合計)");
  assert.match(r.done, /「枠一覧に枠別を表示」/);
  await ok("枠一覧に枠別を表示");
  assert.match(src(), /画面 枠一覧:\n(    .+\n)*    枠別を表示\n/);
  // 確かめるを見出しごと 1 行で
  await ok("確かめる 新しい流れ: 花子(利用者)が 予約する(枠1, 1) → 成功。花子から 予約が 1件見える");
  assert.match(src(), /確かめる 新しい流れ:\n    花子\(利用者\)が 予約する\(枠1, 1\) → 成功\n    花子から 予約が 1件見える\n/);
  // かぎかっこ無しの「…で」
  await ok("予約するで「目的」も入れられるようにしたい");
  assert.match(src(), /動作 予約する\(枠ID: 識別子, 人数: 整数, 目的: 文字列\)/);
  // 選択肢を消す。使っている値は消さずに場所を知らせる
  await ok("列挙「状況」を作る(受付、対応中、完了)");
  await ok("選択肢の「対応中」を消す");
  assert.match(src(), /列挙 状況:\n    受付\n    完了\n/);
  const before = src();
  const j = await say("予約失敗から「満席」を消す");
  assert.notEqual(j.result?.ok, true);
  assert.match(JSON.stringify(j), /使っているので消せません/);
  assert.equal(src(), before);
});

test("「「取り消す」で予約の状態を「取消」にする」: 2 回目は断り、取り消した予約にはボタンを出さず、更新の権限が無い役割を知らせる", async () => {
  await ok("予約に「状態」を追加");
  const r = await ok("「取り消す」で予約の状態を「取消」にする");
  assert.match(r.done, /予約\.更新 が無いので/);
  assert.match(src(), /動作 取り消す\(予約ID: 識別子\) -> 結果<識別子, 予約失敗>:\n    件 = 予約を取得\(予約ID\)\n    件\.状態 != "取消" を確認 または 失敗\("取消済み"\)\n    件\.状態 = "取消"\n    親 = 枠を取得\(件\.枠ID\)\n    親\.予約数 = 親\.予約数 - 件\.人数\n/);
  assert.match(src(), /予約の状態が取消でないとき「取り消す」を押すと 取り消す\(予約\.ID\)/);
});
