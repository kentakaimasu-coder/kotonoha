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
  // 登録の動作は備品があるかを取り出して確かめている(備品 = 備品を取得(備品ID))。その備品の状態を変える
  assert.match(src(), /動作 借りる\(備品ID: 識別子\) -> 結果<識別子, 借りるの失敗>:\n    備品 = 備品を取得\(備品ID\)\n    備品\.状態 != "貸出中" を確認 または 失敗\("すでに貸出中"\)\n    備品\.状態 = "貸出中"\n    貸出を登録\(/);
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

test("「予約の人数は1以上にする」: 受け取る動作で、外れたら「人数不正」で止める", async () => {
  const before = (src().match(/人数 >= 1 を確認/g) ?? []).length;
  await ok("予約の人数は1以上にする");
  assert.equal((src().match(/人数 >= 1 を確認 または 失敗\("人数不正"\)/g) ?? []).length, before + 1);
});

test("「「借りる」で備品の貸出中を真にする」: もう貸出中なら「すでに貸出中」で断る", async () => {
  await ok("備品に「貸出中」(真偽)を追加");
  await ok("「持ち出す」で備品の貸出中を真にする");
  assert.match(src(), /動作 持ち出す\(備品ID: 識別子\) -> 結果<識別子, [^>]+>:\n    件 = 備品を取得\(備品ID\)\n    件\.貸出中 != 真 を確認 または 失敗\("すでに貸出中"\)\n    件\.貸出中 = 真\n/);
});

test("詳細の画面を後から作ると、その 1 件を扱う動作でボタンの無いもののボタンも出す", async () => {
  await ok("データ「伝票」を作る(品名: 文字列、済: 真偽)");
  // 画面が無いうちに動作を作る(ボタンを出せる画面が無い)
  await ok("「仕上げる」で伝票の済を真にする");
  if (/画面 伝票詳細:/.test(src())) return; // 動作を作ったときに詳細の画面もできる版ではここまで
  await ok("画面「伝票一覧」を作る(伝票を一覧表示)");
  await ok("画面「伝票の中身」を作る(伝票の品名と済を表示)");
  // 済にするボタンは、もう済のときは出さない(第 20 回評価)
  assert.match(src(), /画面 伝票の中身:\n    伝票の品名と済を表示\n    伝票の済が真でないとき「仕上げる」を押すと 仕上げる\(伝票\.ID\)\n/);
});

test("「枠一覧に、枠のうち開始日時が今日のものを一覧表示」: 画面の名前から書いてよく、同じデータの一覧は置き換える", async () => {
  const r = await ok("枠一覧に、枠のうち開始日時が今日のものを一覧表示");
  assert.match(r.done, /画面「枠一覧」の一覧を/);
  assert.match(src(), /画面 枠一覧:\n    枠のうち開始日時が今日のものを一覧表示\n/);
  assert.equal((src().match(/\n    枠(を|のうち)[^\n]*一覧表示\n/g) ?? []).length, 1);
});

test("項目の名前を変えると、失敗の名前「人数不正」もそろえる", async () => {
  const r = await ok("予約の「人数」を「使う棚」に変える");
  assert.match(r.done, /「人数不正」も「使う棚不正」にしました/);
  assert.doesNotMatch(src(), /人数不正/);
  assert.match(src(), /失敗\("使う棚不正"\)/);
});

test("名前に「の」があるデータ(「月謝の支払い」)の集計を作れる", async () => {
  await ok("データ「月謝の支払い」を作る(生徒: 文字列、金額: 金額)");
  const r = await ok("分析「生徒別の月謝」を作る(月謝の支払いの金額を生徒ごとに合計)");
  assert.match(r.done, /分析「生徒別の月謝」を作りました/);
  assert.match(src(), /集計 = "月謝の支払いの金額を生徒ごとに合計"/);
});

test("データの名前を変えると「データ名+する」の動作もそろえる(「予約」→「取り置き」で「予約する」→「取り置きする」)", async () => {
  const r = await ok("「予約」を「取り置き」に変える");
  assert.match(r.done, /動作「予約する」も「取り置きする」にしました/);
  assert.match(src(), /動作 取り置きする\(/);
  assert.doesNotMatch(src(), /予約する\(/);
});

test("アプリの名前と同じ名前のデータにしてよい", async () => {
  await ok("アプリの名前を「受付票」にする");
  await ok("「回数券」を「受付票」に変える");
  assert.match(src(), /データ 受付票:/);
});

test("登録の動作は、ほかのデータを指す値(メニューID)のデータがあるかを確かめる", async () => {
  await ok("データ「品書き」を作る(品名: 文字列)");
  await ok("データ「注文票」を作る(品書きID: 識別子、数量: 整数)");
  await ok("注文票を登録できるようにしたい");
  assert.match(src(), /動作 注文票を追加\(品書きID: 識別子, 数量: 整数\)[^\n]*:\n    品書き = 品書きを取得\(品書きID\)\n    注文票を登録\(/);
});

test("「予約」を「内見予約」に変えても、アプリの名前や、もう「内見予約」の所は二重にしない", async () => {
  await ok("アプリの名前を「不動産 内見予約」にする");
  await ok("「取り置き」を「内見取り置き」に変える");
  assert.match(src(), /アプリ「不動産 内見予約」/);
  assert.doesNotMatch(src(), /内見内見/);
  assert.match(src(), /データ 内見取り置き:/);
});

test("「受付係にすべてのデータの登録と更新を許可する」: データごとに書かなくてよい", async () => {
  await ok("受付係にすべてのデータの登録と更新を許可する");
  const line = /役割 受付係:\n    権限 = \[([^\]]*)\]/.exec(src())![1];
  for (const c of ["枠.作成", "枠.更新", "品書き.作成", "品書き.更新"]) assert.ok(line.includes(c), line);
});

test("担当者は登録した本人(入力欄にしない)", async () => {
  await ok("データ「商談」を作る(顧客名: 文字列、担当者)");
  assert.match(src(), /データ 商談:\n    ID: 識別子\n    顧客名: 文字列\n    担当者: 識別子\n/);
  await ok("商談を登録できるようにしたい");
  assert.match(src(), /動作 商談を追加\(顧客名: 文字列\)[^\n]*:\n    商談を登録\(顧客名: 顧客名, 担当者: 本人\)/);
});

test("「…するときに〜を1減らす」も読む(前のテストで「予約」は「内見取り置き」、「回数券」は「受付票」に変わっている)", async () => {
  const before = (src().match(/\.残り回数 = [^\n]*\.残り回数 - 1/g) ?? []).length;
  await ok("内見取り置きするときに受付票の残り回数を1減らす");
  assert.equal((src().match(/\.残り回数 = [^\n]*\.残り回数 - 1/g) ?? []).length, before + 1);
});

test("詳細の画面に表示する項目を増やすと、前の表示の行を置き換える(二重に出さない)", async () => {
  // 今の表示の行と同じ項目を並べ替えた行(前の行の項目を全部含む)を入れる
  const cur = /\n    (枠の(\S+)を表示)\n/.exec(src())!;
  const r = await ok(`枠の${cur[2].split("と").reverse().join("と")}を表示`, { screen: "枠詳細" });
  assert.match(r.done, /置き換えました/);
  assert.equal((src().match(/\n    枠の[^\n]*を表示\n/g) ?? []).length, 1);
});


test("「「満席」を「定員に達しました」に変える」: 失敗の言葉(選択肢の値)の名前も変えられる", async () => {
  const r = await ok("「満席」を「定員に達しました」に変える");
  assert.match(r.done, /「満席」を「定員に達しました」に変えました/);
  assert.match(src(), /失敗\("定員に達しました"\)/);
  assert.doesNotMatch(src(), /"満席"/);
});

test("第 17 回評価の言い方: 最初に出る画面・主催者だけのメニュー・データに無い項目での並べ替え", async () => {
  // 「アプリを開いたときに最初に出る画面を…」がアプリの名前の文と取り違えられ「アプリの名前に「」は使えません」になった
  const r1 = await ok("アプリを開いたときに最初に出る画面を「受付の一覧」にする");
  assert.match(r1.done, /受付の一覧/);
  assert.match(src(), /^アプリ「[^」]+」\n(?:(?!画面 ).*\n)*画面 受付の一覧:/m);
  // 「…の画面は受付係だけのメニューにしたい」
  const r2 = await ok("枠一覧の画面は受付係だけのメニューにしたい");
  assert.match(r2.done, /受付係 だけにしました/);
  // データに無い項目で並べようとしたときは、行を入れずに並べられる項目を案内する
  const before = src();
  const j = await say("枠を症状メモの順に並べる");
  assert.notEqual(j.result?.ok, true);
  assert.match(JSON.stringify(j), /項目「症状メモ」がありません.*並べられる項目/);
  assert.equal(src(), before);
});

test("集計を作るとき、選んでいた画面が 1 件を見る詳細の画面なら、そこには出さない", async () => {
  const r = await ok("分析「名前別の定員」を作る(枠の定員を名前ごとに合計)", { screen: "枠詳細" });
  assert.doesNotMatch(r.done, /画面「枠詳細」に出しました/);
  assert.doesNotMatch(src(), /\n    名前別の定員を表示\n/);
});

test("「枠一覧に枠の定員を名前ごとに合計を表示」: 画面の名前と集計の文を一度に書ける", async () => {
  await ok("枠一覧に枠の定員を名前ごとに合計を表示");
  assert.match(src(), /画面 枠一覧:\n(    .+\n)*    枠の定員を名前ごとに合計\n/);
});

test("必須で足した項目を空でもよいにすると、初期データの仮の値「未設定」を消す", async () => {
  const seed = () => JSON.parse(readFileSync(join(dir, "seed.json"), "utf8"));
  const dn = Object.keys(seed()).find((k) => Array.isArray(seed()[k]) && seed()[k].length && /枠/.test(k))!;
  await ok(`${dn}に「持ち物」を追加`);
  assert.ok(seed()[dn].every((r: any) => r.持ち物 === "未設定"), JSON.stringify(seed()[dn][0]));
  await ok("「持ち物」を空でもよいにする");
  assert.ok(seed()[dn].every((r: any) => r.持ち物 === undefined), JSON.stringify(seed()[dn][0]));
});

test("本人が自動で入る項目(担当)を文字列にすると、登録の動作では入力する欄にする(第 17・18 回評価)", async () => {
  await ok("データ「施術」を作る(内容: 文字列, 担当)");
  await ok("施術を登録できるようにしたい");
  const r = await ok("施術の「担当」を文字列にする");
  assert.match(r.done, /入力する欄にしました/);
  assert.match(src(), /動作 施術を追加\(内容: 文字列, 担当: 文字列\)[^\n]*:\n    施術を登録\(内容: 内容, 担当: 担当\)/);
});

test("「「予約する」で枠が売り切れなら断る」: 真偽・選択肢の項目で断る確認を入れる(第 18 回評価)", async () => {
  await ok("枠に「売り切れ」を追加(真偽)");
  // 前のテストで動作の名前が変わっているので、枠を受け取って失敗を返す動作を探す
  const an = /動作 (\S+)\(枠ID: 識別子[^\n]*結果</.exec(src())![1];
  const r = await ok(`「${an}」で枠が売り切れなら断る`);
  assert.match(r.done, /「売り切れ」で断るようにしました/);
  assert.match(src(), /\.売り切れ == 偽 を確認 または 失敗\("売り切れ"\)/);
});

test("項目の順番・ボタンを消す・ボタンの文字を変えるを日本語で(第 19 回評価)", async () => {
  // 「会場を名前の次にする」が型の名前と取り違えられ「型がありません」と出た
  await ok("データ「備品台帳」を作る(品名: 文字列, 数: 整数, 置き場所: 文字列)");
  let r = await ok("置き場所を品名の次にする");
  assert.match(r.done, /「置き場所」を「品名」の次にしました/);
  assert.match(src(), /データ 備品台帳:\n    ID: 識別子\n    品名: 文字列\n    置き場所: 文字列\n    数: 整数/);
  await ok("備品台帳の数を先頭にする");
  assert.match(src(), /データ 備品台帳:\n    ID: 識別子\n    数: 整数\n    品名: 文字列/);
  // 「「1名で予約」のボタンを…に変える」「「…」の行を消す」が通じず、ソースで直した
  const lbl = /「([^」]+)」を押すと/.exec(src())![1];
  r = await ok(`「${lbl}」のボタンを「お試しボタン」に変える`);
  assert.match(r.done, /文字を「お試しボタン」に変えました/);
  r = await ok("「お試しボタン」の行を消す");
  assert.match(r.done, /ボタン「お試しボタン」を消しました/);
  assert.doesNotMatch(src(), /お試しボタン/);
});

test("かぎかっこの無い「3名で予約のボタンを追加」、「4名で予約できるようにしたい」でもボタンを足す(第 20 回評価)", async () => {
  const lbl = /「(\D*)\d+(\D*)」を押すと/.exec(src());
  assert.ok(lbl, "数を含むボタンがある");
  let r = await ok(`${lbl![1]}7${lbl![2]}のボタンを追加`);
  assert.match(r.done, /のボタンを足しました/);
  r = await ok(`${lbl![1]}8${lbl![2]}できるようにしたい`);
  assert.match(r.done, /のボタンを足しました/);
  assert.match(src(), new RegExp(`「${lbl![1]}8${lbl![2]}」を押すと`));
});

test("詳細の画面を後から作ると、一覧の画面も作り、状態を変えるボタンはその状態にできるときだけ出す(第 20 回評価)", async () => {
  await ok("列挙「調理状況」を作る(受付、調理中、提供済み)");
  await ok("データ「卓注文」を作る(卓番号: 整数, 調理状況: 調理状況)");
  await ok("「調理開始」で卓注文の調理状況を「調理中」にする");
  await ok("「提供」で卓注文の調理状況を「提供済み」にする");
  await ok("卓注文の調理状況は 受付 → 調理中 → 提供済み の順");
  const r = await ok("画面「卓注文詳細」を作る(卓注文の卓番号と調理状況を表示)");
  assert.match(r.done, /画面「卓注文一覧」も作りました/);
  assert.match(src(), /卓注文の調理状況が受付のとき「調理開始」を押すと 調理開始\(卓注文\.ID\)/);
  assert.match(src(), /卓注文の調理状況が調理中のとき「提供」を押すと 提供\(卓注文\.ID\)/);
});

test("空でもよい ID の項目を必須にすると、確かめるには初期データの ID か仮の値を入れる(\"\" で ✗ になった。第 20 回評価)", async () => {
  await ok("データ「施術メニュー」を作る(名前: 文字列)");
  const an = /動作 (\S+)\(枠ID: 識別子[^\n]*結果</.exec(src())![1];
  const D = /^\s+(\S+)を登録\(/m.exec(src().split(`動作 ${an}(`)[1])![1];
  await ok(`${D}に施術メニューID(識別子)を足す`);
  await ok(`${an}で「施術メニューID」も入力できるようにしたい`);
  const r = await ok("「施術メニューID」を必須にする");
  assert.match(r.done, /必須/);
  assert.doesNotMatch(src(), new RegExp(`${an}\\([^)]*, ""\\)`));
});

test("「「借りる」で絵本の残り冊数を1減らす」: 足りなければ「…不足」で断る(第 20 回評価)", async () => {
  await ok("データ「紙芝居」を作る(題名: 文字列, 残り冊数: 整数)");
  await ok("画面「紙芝居一覧」を作る(紙芝居を一覧表示)");
  await ok("「紙芝居を借りる」で紙芝居の残り冊数を1減らす");
  assert.match(src(), /動作 紙芝居を借りる\(紙芝居ID: 識別子\) -> 結果<識別子, 紙芝居を借りるの失敗>:\n    件 = 紙芝居を取得\(紙芝居ID\)\n    件\.残り冊数 >= 1 を確認 または 失敗\("残り冊数不足"\)/);
});
