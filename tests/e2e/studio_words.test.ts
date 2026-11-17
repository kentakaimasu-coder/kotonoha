// Studio の日本語の指示: 評価で読めなかった言い方を、その言い方のまま通す(第 14 回評価)。
// 確かめるの見出しごと 1 行、並べ替え、画面に集計を出す、かぎかっこ無しの受け取る値、選択肢を消す、取り消しの 2 回目
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
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
  // 「レッスン順」のように、ほかのデータを指す項目は ID を付けずに書いてもよい
  await ok("予約のうち予約者が本人のものを枠順に一覧表示", { screen: "自分の予約" });
  assert.match(src(), /\n    予約のうち予約者が本人のものを枠ID順に一覧表示\n/);
  // 画面の名前を書いて集計を出す
  const r = await ok("分析「枠別」を作る(枠の定員を名前ごとに合計)");
  assert.match(r.done, /「枠一覧に枠別を表示」/);
  await ok("枠一覧に枠別を表示");
  assert.match(src(), /画面 枠一覧:\n(    .+\n)*    枠別を表示\n/);
  // 確かめるを見出しごと 1 行で
  await ok("確かめる 新しい流れ: 花子(利用者)が 予約する(枠1, 1) → 成功。花子から 予約が 1件見える");
  assert.match(src(), /確かめる 新しい流れ:\n    花子\(利用者\)が 予約する\(枠1, 1\) → 成功\n    花子から 予約が 1件見える\n/);
  // かぎかっこ無しの「…で」
  const rp = await ok("予約するで「目的」も入れられるようにしたい");
  assert.match(src(), /動作 予約する\(枠ID: 識別子, 人数: 整数, 目的: 文字列\)/);
  // ボタンでは入れられないので、ボタンのある画面に入力できるフォームも足す(ボタンは残す)
  assert.match(rp.done, /画面「枠詳細」に入力できるフォームも足しました/);
  assert.match(src(), /画面 枠詳細:\n(    .+\n)*    予約するのフォームを表示\n/);
  assert.match(src(), /「1名で予約」を押すと 予約する\(枠\.ID, 1, ""\)/);
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

test("画面を選ばずに「この画面は…だけ」と書くと、今作った画面に入る", async () => {
  await ok("画面「受付の一覧」を作る(予約を一覧表示)");
  const r = await ok("この画面は受付係だけ");
  assert.match(r.done, /受付の一覧/);
  assert.match(src(), /画面 受付の一覧:\n    予約を一覧表示\n    この画面は受付係だけ\n/);
});

test("「「取り消す」で予約の状態を「取消」にする」: 2 回目は断り、取り消した予約にはボタンを出さず、更新の権限が無い役割を知らせる", async () => {
  await ok("予約に「状態」を追加");
  const r = await ok("「取り消す」で予約の状態を「取消」にする");
  assert.match(r.done, /予約\.更新 が無いので/);
  assert.match(src(), /動作 取り消す\(予約ID: 識別子\) -> 結果<識別子, 予約失敗>:\n    件 = 予約を取得\(予約ID\)\n    件\.状態 != "取消" を確認 または 失敗\("取消済み"\)\n    件\.状態 = "取消"\n    親 = 枠を取得\(件\.枠ID\)\n    親\.予約数 = 親\.予約数 - 件\.人数\n/);
  assert.match(src(), /予約の状態が取消でないとき「取り消す」を押すと 取り消す\(予約\.ID\)/);
});

test("項目を選択肢の型にすると、登録の行の値が選択肢に無ければ最初の値にそろえる", async () => {
  // 登録で文字の値を入れている項目(自動で作った登録の行と同じ形)
  const s0 = src()
    .replace(/(データ 予約:\n(?:    .+\n)*?    予約日時: 日時\n)/, "$1    区分: 文字列\n")
    .replace(/予約を登録\((.*)\) を返す/, '予約を登録($1, 区分: "一般") を返す');
  assert.match(s0, /区分: "一般"/);
  writeFileSync(join(dir, "src/app.koto"), s0);
  await ok("列挙「予約区分」を作る(通常、団体)");
  const r = await ok("予約の「区分」を「予約区分」にする");
  assert.match(r.done, /「一般」は選択肢に無いので「通常」にしました/);
  assert.match(src(), /区分: "通常"\) を返す/);
});

test("既にある動作で、ほかのデータの状態を変える(借りる・返す)", async () => {
  await ok("列挙「備品状態」を作る(貸出可、貸出中)");
  await ok("列挙「貸出状況」を作る(貸出中、返却済)");
  await ok("データ「備品」を作る(名前: 文字列、状態: 備品状態)");
  await ok("データ「貸出」を作る(備品ID: 識別子、借りた人: 識別子、貸出日: 日付、状況: 貸出状況)");
  await ok("「借りる」で貸出を登録");
  const r = await ok("「借りる」で備品の状態を「貸出中」にする");
  assert.match(r.done, /動作「借りる」で、備品の状態を「貸出中」にするようにしました/);
  assert.match(src(), /動作 借りる\(備品ID: 識別子\) -> 結果<識別子, 借りるの失敗>:\n    対象 = 備品を取得\(備品ID\)\n    対象\.状態 != "貸出中" を確認 または 失敗\("すでに貸出中"\)\n    対象\.状態 = "貸出中"\n    貸出を登録\(/);
  // 返す(貸出ID) では、取り出した貸出の備品IDから備品を取り出す
  await ok("「返す」で貸出の状況を「返却済」にする");
  await ok("「返す」で備品の状態を「貸出可」にする");
  assert.match(src(), /    件 = 貸出を取得\(貸出ID\)\n    対象 = 備品を取得\(件\.備品ID\)\n    対象\.状態 != "貸出可" を確認 または 失敗\("すでに貸出可"\)\n/);
  assert.match(src(), /    対象\.状態 = "貸出可"\n    貸出ID を返す/);
  // 選択肢に無い値は入れない
  const j = await say("「借りる」で備品の状態を「修理中」にする");
  assert.notEqual(j.result?.ok, true);
  assert.match(JSON.stringify(j), /備品状態に「修理中」を追加/);
});

test("「「作品のメモ」を空でもよいにする」: かぎかっこの中の「の」で区切らない", async () => {
  await ok("予約に「作品のメモ」を追加");
  const r = await ok("「作品のメモ」を空でもよいにする");
  assert.match(r.done, /作品のメモ/);
  assert.match(src(), /    作品のメモ: 選択<文字列>\n/);
});

test("裏返しの言い方の数の確認: 「…が1以上でなければ失敗」「…は0より大きくないとだめにする」", async () => {
  await ok("予約するで人数が1以上でなければ失敗");
  assert.match(src(), /    人数 >= 1 を確認 または 失敗\("人数不正"\)\n/);
  await ok("定員は0より大きくないとだめにする");
  assert.match(src(), /    定員 > 0 を確認 または 失敗\("定員不正"\)\n/);
});

test("「予約したら回数券を1減らす」: 動作と数の項目を決め、足りないときの確認は回数券を取り出した後に入る", async () => {
  await ok("データ「回数券」を作る(残り回数: 整数)");
  await ok("「予約する」で回数券を選べるようにしたい");
  const r = await ok("予約したら回数券を1減らす");
  assert.match(r.done, /回数券の残り回数を1減らす/);
  const body = /動作 予約する\([^\n]*\n((?:    .+\n)+)/.exec(src())![1];
  const def = body.indexOf("= 回数券を取得(回数券ID)");
  const chk = body.indexOf(".残り回数 >= 1 を確認");
  assert.ok(def >= 0 && chk > def, body);
});

test("「日報に「現場」を追加」: ほかのデータの名前の項目は、そのデータを選ぶ「現場ID」にする", async () => {
  await ok("データ「現場」を作る(名前: 文字列)");
  const r = await ok("予約に「現場」を追加");
  assert.match(r.done, /項目「現場ID」\((選択<)?識別子>?\)/);
  assert.match(src(), /    現場ID: (選択<)?識別子>?\n/);
  await ok("データ「日報」を作る(現場、内容: 文字列)");
  assert.match(src(), /データ 日報:\n    ID: 識別子\n    現場ID: 識別子\n    内容: 文字列\n/);
});

test("「「3個で予約」…」: 数え方や「で」の有無が違うボタンもまねて足す", async () => {
  const r = await ok("「3個予約」ボタンを足す", { screen: "枠詳細" });
  assert.match(r.done, /「1名で予約」をまねて 3 にしました/);
  assert.match(src(), /「3個予約」を押すと 予約する\(枠\.ID, 3/);
});
