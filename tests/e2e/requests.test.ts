// ペルソナ評価 第 2 回の残要望への対応を、生成した backend で確かめる(SQLite と、KOTONOHA_TEST_PG_URL があれば PostgreSQL)。
// examples/nippo(日報)を使う。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, type ChildProcess } from "node:child_process";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, runNode, SECRET } from "./helpers.ts";

const PG = process.env.KOTONOHA_TEST_PG_URL;
const enc = encodeURIComponent;
let dir = "";
const servers: { name: string; url: string }[] = [];
const procs: ChildProcess[] = [];

before(async () => {
  dir = tmp("requests");
  build("examples/nippo", "backend", dir);
  const dbs: [string, string][] = [["SQLite", join(dir, "data/app.sqlite")]];
  if (PG) {
    execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: dir, stdio: "inherit" });
    execFileSync("psql", [PG, "-q", "-c", "DROP DATABASE IF EXISTS koto_requests"]);
    execFileSync("psql", [PG, "-q", "-c", "CREATE DATABASE koto_requests"]);
    const u = new URL(PG);
    u.pathname = "/koto_requests";
    dbs.push(["PostgreSQL", u.toString()]);
  }
  for (const [name, db] of dbs) {
    const port = await freePort();
    procs.push(start(dir, ["src/main.ts"], { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: db, PORT: String(port), KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" }));
    servers.push({ name, url: `http://127.0.0.1:${port}` });
  }
  for (const s of servers) await waitHttp(s.url + "/api/health");
});
after(() => procs.forEach((p) => p.kill()));

const tok = (sub: string) => runNode(dir, ["tools/dev-token.ts", "--sub", sub, "--caps", "日報.作成,日報.読取"], { KOTONOHA_JWT_SECRET: SECRET });

async function write(url: string, t: string, 日付: string) {
  const r = await fetch(`${url}/api/actions/${enc("日報を書く")}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${t}`, "Content-Type": "application/json" },
    body: JSON.stringify({ 日付, 作業内容: "検品", 作業時間: 3 }),
  });
  return { status: r.status, body: await r.json() };
}

test("重複不可: 同じ人が同じ日に 2 通は書けない(409 と日本語の説明)・別の人は書ける・同時に送っても 1 通だけ", async () => {
  for (const s of servers) {
    const [taro, hanako, jiro] = [tok("太郎"), tok("花子"), tok("次郎")];
    assert.equal((await write(s.url, taro, "2026-12-05")).status, 200, s.name);
    const dup = await write(s.url, taro, "2026-12-05");
    assert.equal(dup.status, 409, `${s.name} ${JSON.stringify(dup)}`);
    assert.match(dup.body.message, /既に登録されています.*日付と記入者/);
    assert.equal((await write(s.url, hanako, "2026-12-05")).status, 200, s.name);
    assert.equal((await write(s.url, taro, "2026-12-06")).status, 200, s.name);
    const rs = await Promise.all(Array.from({ length: 6 }, () => write(s.url, jiro, "2026-12-07")));
    assert.equal(rs.filter((r) => r.status === 200).length, 1, `${s.name} ${JSON.stringify(rs.map((r) => r.status))}`);
    assert.ok(rs.every((r) => r.status === 200 || r.status === 409), s.name);
    const mine = await (await fetch(`${s.url}/api/data/${enc("日報")}`, { headers: { Authorization: `Bearer ${jiro}` } })).json();
    assert.equal(mine.length, 1, s.name);
  }
});

async function act(url: string, t: string, name: string, body: unknown) {
  const r = await fetch(`${url}/api/actions/${enc(name)}`, { method: "POST", headers: { Authorization: `Bearer ${t}`, "Content-Type": "application/json" }, body: JSON.stringify(body) });
  return { status: r.status, body: await r.json() };
}
const tokenOf = (sub: string, caps: string) => runNode(dir, ["tools/dev-token.ts", "--sub", sub, "--caps", caps], { KOTONOHA_JWT_SECRET: SECRET });

test("状態の順番: 下書き → 提出済み → 確認済み を飛ばせない・戻れない(409 と次の値の案内)、日時は日本時間で返る", async () => {
  for (const s of servers) {
    const t = tokenOf("三郎", "日報.作成,日報.読取,日報.更新");
    const id = (await act(s.url, t, "日報を書く", { 日付: "2026-12-10", 作業内容: "発送", 作業時間: 5 })).body.ok;
    const skip = await act(s.url, t, "確認する", { 日報ID: id });
    assert.equal(skip.status, 409, `${s.name} ${JSON.stringify(skip)}`);
    assert.match(skip.body.message, /「下書き」の次は「提出済み」/);
    assert.equal((await act(s.url, t, "提出する", { 日報ID: id })).status, 200, s.name);
    const got = await (await fetch(`${s.url}/api/data/${enc("日報")}/${id}`, { headers: { Authorization: `Bearer ${t}` } })).json();
    assert.equal(got.状態, "提出済み");
    assert.equal(got.提出期限, "2026-12-12", "日後(日付, 2)");
    assert.match(got.提出日時, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}\+09:00$/, "日時は日本時間(+09:00)で返る");
    assert.ok(Math.abs(Date.parse(got.提出日時) - Date.now()) < 60_000);
    assert.equal((await act(s.url, t, "確認する", { 日報ID: id })).status, 200, s.name);
    const back = await act(s.url, t, "提出する", { 日報ID: id });
    assert.equal(back.status, 409, s.name);
    assert.match(back.body.message, /「確認済み」で終わり/);
  }
});

test("日付の計算: 今日() と 日数差(アプリの時刻帯の暦日)", async () => {
  const today = new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(new Date());
  const expected = Math.round((Date.parse("2026-12-14T00:00:00Z") - Date.parse(today + "T00:00:00Z")) / 86_400_000);
  for (const s of servers) {
    const t = tokenOf("四郎", "日報.作成,日報.読取,日報.更新");
    const id = (await act(s.url, t, "日報を書く", { 日付: "2026-12-12", 作業内容: "棚卸し", 作業時間: 2 })).body.ok;
    const r = await act(s.url, t, "期限までの日数", { 日報ID: id });
    assert.equal(r.status, 200, `${s.name} ${JSON.stringify(r)}`);
    assert.equal(r.body.ok, expected, s.name);
  }
});

test("他の人の日報は 404 で、理由の候補が日本語で出る・月ごとの合計は本人の分だけ", async () => {
  for (const s of servers) {
    const a = tokenOf("五郎", "日報.作成,日報.読取");
    const b = tokenOf("六郎", "日報.作成,日報.読取");
    const id = (await act(s.url, a, "日報を書く", { 日付: "2026-11-20", 作業内容: "検品", 作業時間: 4 })).body.ok;
    await act(s.url, a, "日報を書く", { 日付: "2026-11-21", 作業内容: "検品", 作業時間: 6 });
    await act(s.url, a, "日報を書く", { 日付: "2026-12-01", 作業内容: "検品", 作業時間: 1 });
    const r = await fetch(`${s.url}/api/data/${enc("日報")}/${id}`, { headers: { Authorization: `Bearer ${b}` } });
    assert.equal(r.status, 404);
    assert.match((await r.json()).message, /無いか、ほかの人のもの/);
    const agg = await (await fetch(`${s.url}/api/aggregates/${enc("日報の日付の月ごとの集計")}`, { headers: { Authorization: `Bearer ${a}` } })).json();
    assert.deepEqual(agg, [
      { 日付の月: "2026年11月", 件数: 2, 作業時間の合計: 10 },
      { 日付の月: "2026年12月", 件数: 1, 作業時間の合計: 1 },
    ], s.name);
  }
});

test("取下げ: どの状態からでも取り下げられ、取り下げた日報は重複の判定から外れる(同じ日に書き直せる)", async () => {
  for (const s of servers) {
    const t = tokenOf("七郎", "日報.作成,日報.読取,日報.更新");
    const id = (await act(s.url, t, "日報を書く", { 日付: "2026-12-20", 作業内容: "検品", 作業時間: 2 })).body.ok;
    assert.equal((await act(s.url, t, "提出する", { 日報ID: id })).status, 200, s.name);
    assert.equal((await act(s.url, t, "日報を書く", { 日付: "2026-12-20", 作業内容: "検品", 作業時間: 2 })).status, 409, s.name);
    assert.equal((await act(s.url, t, "取り下げる", { 日報ID: id })).status, 200, s.name);
    const again = await act(s.url, t, "日報を書く", { 日付: "2026-12-20", 作業内容: "検品(書き直し)", 作業時間: 3 });
    assert.equal(again.status, 200, `${s.name} ${JSON.stringify(again)}`);
    const back = await act(s.url, t, "提出する", { 日報ID: id });
    assert.equal(back.status, 409, s.name);
    assert.match(back.body.message, /「取下げ」で終わり/);
  }
});

test("期限切れの絞り込み: 提出期限が今日より前・今日以降(日付の比較)", async () => {
  for (const s of servers) {
    const t = tokenOf("八郎", "日報.作成,日報.読取,日報.更新");
    await act(s.url, t, "日報を書く", { 日付: "2020-01-01", 作業内容: "昔の分", 作業時間: 1 });
    await act(s.url, t, "日報を書く", { 日付: "2099-01-01", 作業内容: "先の分", 作業時間: 1 });
    const get = async (q: string) => (await (await fetch(`${s.url}/api/data/${enc("日報")}?${q}`, { headers: { Authorization: `Bearer ${t}` } })).json()).map((r: any) => r.作業内容);
    assert.deepEqual(await get(`${enc("lt_today.提出期限")}=1`), ["昔の分"], s.name);
    assert.deepEqual(await get(`${enc("ge_today.提出期限")}=1`), ["先の分"], s.name);
  }
});

test("分析を集計の文で書く(SQL なし): 列の名前は記録の宣言どおり、取下げは除く、本人限定データは全件閲覧の許可が要る", async () => {
  for (const s of servers) {
    const viewer = runNode(dir, ["tools/dev-token.ts", "--sub", "部長", "--caps", "日報.読取,全件閲覧.日報"], { KOTONOHA_JWT_SECRET: SECRET });
    const url = `${s.url}/api/analyses/${enc("記入者別の作業時間")}`;
    assert.equal((await fetch(url, { headers: { Authorization: `Bearer ${tok("太郎")}` } })).status, 403, s.name);
    const r = await fetch(url, { headers: { Authorization: `Bearer ${viewer}` } });
    const rows = await r.json();
    assert.equal(r.status, 200, `${s.name} ${JSON.stringify(rows)}`);
    const taro = rows.find((x: any) => x.記入者 === "太郎");
    assert.ok(taro, `${s.name} ${JSON.stringify(rows)}`);
    assert.deepEqual(Object.keys(taro).sort(), ["合計時間", "日報数", "記入者"].sort(), s.name);
    assert.equal(typeof taro.日報数, "number", s.name);
    assert.equal(typeof taro.合計時間, "number", s.name);
    assert.equal(taro.合計時間, taro.日報数 * 3, `${s.name} ${JSON.stringify(taro)}`);
  }
});
