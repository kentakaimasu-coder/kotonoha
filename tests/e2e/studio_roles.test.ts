// Studio: 役割の無いアプリで最初の役割を作ると、確かめるで役割を書いていない人をその役割の人にする
// (役割を作ると、ひな形の確かめるが ✗(403)になった。第 13・14 回評価で 10 人)
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
  dir = join(tmp("studio-roles"), "日報");
  execFileSync(KOTO, ["init", dir, "--template", "日報"], { stdio: "pipe" });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  await waitHttp(url + "/", 60000);
  token = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(url + "/")).text())![1];
});
after(() => {
  proc?.kill();
});
const src = () => readFileSync(join(dir, "src/app.koto"), "utf8");

test("最初の役割を作ると、確かめるの人(「太郎が」「花子から」)はその役割の人になり、2 つ目の役割では変えない", async () => {
  assert.match(src(), /\n    太郎が 日報を書く/);
  let j: any = await (
    await fetch(url + "/api/op", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ op: "add_role", name: "社員", caps: ["日報.読取", "日報.作成"] }) })
  ).json();
  assert.equal(j.ok, true, j.error);
  assert.match(String(j.note ?? j.result?.note ?? JSON.stringify(j)), /「社員」の人にしました/);
  assert.match(src(), /\n    太郎\(社員\)が 日報を書く/);
  assert.match(src(), /\n    太郎\(社員\)から 日報が 1件見える/);
  assert.match(src(), /\n    花子\(社員\)から 日報が 0件見える/);
  assert.equal(j.outline.ok, true);
  j = await (
    await fetch(url + "/api/op", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": token }, body: JSON.stringify({ op: "add_role", name: "課長", caps: ["日報.読取", "全件閲覧.日報"] }) })
  ).json();
  assert.equal(j.ok, true, j.error);
  assert.doesNotMatch(src(), /\(課長\)/);
  assert.match(src(), /\n    太郎\(社員\)が 日報を書く/);
});

test("問い合わせ: 店の人の役割(作成の権限なし・全件閲覧あり)を作っても、確かめるのお客さんは書き換えない", async () => {
  const d2 = join(tmp("studio-roles2"), "問い合わせ");
  execFileSync(KOTO, ["init", d2, "--template", "問い合わせ"], { stdio: "pipe" });
  const port = await freePort();
  const u2 = `http://127.0.0.1:${port}`;
  const p2 = spawn(KOTO, ["-C", d2, "studio", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  try {
    await waitHttp(u2 + "/", 60000);
    const t2 = /name="studio-token" content="([0-9a-f]+)"/.exec(await (await fetch(u2 + "/")).text())![1];
    const j: any = await (
      await fetch(u2 + "/api/op", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": t2 }, body: JSON.stringify({ op: "add_role", name: "先生", caps: ["問い合わせ.読取", "問い合わせ.更新", "全件閲覧.問い合わせ"] }) })
    ).json();
    assert.equal(j.ok, true, j.error);
    const s2 = readFileSync(join(d2, "src/app.koto"), "utf8");
    assert.doesNotMatch(s2, /\(先生\)/);
    assert.match(s2, /\n    花子が 問い合わせる/);
  } finally {
    p2.kill();
  }
});
