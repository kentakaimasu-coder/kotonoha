// 複合テスト: 居酒屋のテーブル注文(examples/izakaya)を koto dev で起動し、
// 客(スマホ 2 卓)・厨房(PC)・店長(PC)が同時に使う流れを通しで確かめる。
// 本人の行だけ(客)と全件閲覧(厨房・店長)、同時注文、品切れ、売上の分析、A/B 実験、権限、アクセシビリティを 1 つの流れで確認する。
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { cpSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, ROOT, tmp, freePort, waitHttp } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium, devices } = require("playwright");
const enc = encodeURIComponent;

let dir = "", url = "", proc: ChildProcess, browser: any;

before(async () => {
  dir = join(tmp("izakaya"), "app");
  cpSync(join(ROOT, "examples/izakaya"), dir, { recursive: true });
  const port = await freePort();
  url = `http://127.0.0.1:${port}`;
  proc = spawn(KOTO, ["-C", dir, "dev", "--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
  proc.stderr?.on("data", (d) => process.env.E2E_VERBOSE && process.stderr.write(d));
  await waitHttp(url + "/login", 240000);
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  proc?.kill();
});

/** 役割ごとのトークン(実運用ではロールの割り当て KOTONOHA_ROLE_MAP で与える権限) */
function token(sub: string, caps: string[]): string {
  const be = join(dir, ".koto/dev/backend");
  return execFileSync(process.execPath, ["--no-warnings", "tools/dev-token.ts", "--sub", sub, "--caps", caps.join(",")], {
    cwd: be,
    env: { ...process.env, KOTONOHA_JWT_SECRET: readFileSync(join(dir, ".koto/dev/secret"), "utf8").trim() },
  })
    .toString()
    .trim();
}
const CUSTOMER = ["注文.作成", "メニュー.読取", "注文.読取"];
const KITCHEN = ["注文.読取", "注文.更新", "全件閲覧.注文"];
const MANAGER = ["メニュー.読取", "メニュー.更新", "注文.読取", "全件閲覧.注文", "実験.結果"];

async function session(kind: "phone" | "pc", tok: string) {
  const ctx = await browser.newContext({ ...(kind === "phone" ? devices["iPhone 13"] : { viewport: { width: 1280, height: 860 } }), locale: "ja-JP" });
  const page = await ctx.newPage();
  const errors: string[] = [];
  page.on("pageerror", (e: Error) => errors.push(String(e)));
  await page.goto(url + "/login");
  await page.fill("#token", tok);
  await page.click("form[action='/login'] button");
  await page.waitForURL(/\/s\//);
  return { ctx, page, errors };
}

async function order(page: any, item: string, button: "1" | "2") {
  await page.goto(`${url}/s/${enc("メニュー")}`);
  await page.click(`text=${item}`);
  await page.waitForSelector("dl");
  const main = (await page.textContent("main"))!;
  const label = button === "2" ? "2つ注文" : main.includes("これを1つください") ? "これを1つください" : "1つ注文";
  const done = page.waitForResponse((r: any) => r.url().includes(enc("注文する")));
  await page.click(`button:has-text("${label}")`);
  const r = await done;
  return r.status();
}

test("客 2 卓が同時に注文 → それぞれ自分の注文だけ → 厨房は全卓を見て提供 → 客に反映 → 品切れ → 売上 → A/B", async () => {
  const t1 = await session("phone", token("1番卓", CUSTOMER));
  const t2 = await session("phone", token("2番卓", CUSTOMER));
  // 同時に注文する
  const [a, b, c] = await Promise.all([order(t1.page, "生ビール", "2"), order(t2.page, "枝豆", "1"), (async () => 0)()]);
  assert.deepEqual([a, b, c], [200, 200, 0]);
  assert.equal(await order(t1.page, "唐揚げ", "1"), 200);
  // 客の画面には、厨房・店長用のボタン(品切れにする)が出ない(押して 403 になるのを防ぐ表示の出し分け)
  assert.equal(await t1.page.locator(`button:has-text("品切れにする")`).count(), 0);

  // 客はそれぞれ自分の注文だけ見える
  await t1.page.goto(`${url}/s/${enc("注文の状況")}`);
  assert.equal(await t1.page.locator("tbody tr").count(), 2);
  assert.ok(!(await t1.page.textContent("main"))!.includes("枝豆"));
  await t2.page.goto(`${url}/s/${enc("注文の状況")}`);
  assert.equal(await t2.page.locator("tbody tr").count(), 1);

  // 厨房は全卓の注文を見て、唐揚げを提供済みにする(開発用ログインの「厨房として入る」= ソースの役割で入る)
  const kctx = await browser.newContext({ viewport: { width: 1280, height: 860 }, locale: "ja-JP" });
  const k = { ctx: kctx, page: await kctx.newPage(), errors: [] as string[] };
  k.page.on("pageerror", (e: Error) => k.errors.push(String(e)));
  await k.page.goto(url + "/login");
  await k.page.fill("#dev-name", "厨房の山田");
  await k.page.click("text=厨房として入る");
  await k.page.waitForURL(/\/s\//);
  await k.page.goto(`${url}/s/${enc("注文の状況")}`);
  assert.equal(await k.page.locator("tbody tr").count(), 3);
  // 一覧の行のリンク(先頭の列)から詳細へ
  await k.page.locator("tbody tr", { hasText: "唐揚げ" }).locator("a").first().click();
  await k.page.waitForSelector("dl");
  assert.equal(await k.page.locator(`button:has-text("提供済みにする")`).count(), 1);
  const served = k.page.waitForResponse((r: any) => r.url().includes(enc("提供済みにする")));
  await k.page.click(`button:has-text("提供済みにする")`);
  assert.equal((await served).status(), 200);

  // 厨房の画面: まだ出していない注文だけが、古い順に並ぶ(並べ替え・絞り込み)
  await k.page.goto(`${url}/s/${enc("厨房")}`);
  const pending = await k.page.locator("tbody tr").allTextContents();
  assert.equal(pending.length, 2, pending.join(" | "));
  assert.ok(pending.every((r: string) => r.includes("調理中") && !r.includes("唐揚げ")), pending.join(" | "));
  assert.match((await k.page.textContent("main"))!, /状態が調理中/);
  // 今日の注文: 新しい順(最後に頼んだ唐揚げが先頭)
  await k.page.goto(`${url}/s/${enc("今日の注文")}`);
  const today = await k.page.locator("tbody tr").allTextContents();
  assert.equal(today.length, 3, today.join(" | "));
  assert.ok(today[0].includes("唐揚げ"), today.join(" | "));

  // 客の画面に反映される
  await t1.page.goto(`${url}/s/${enc("注文の状況")}`);
  const rows = await t1.page.locator("tbody tr").allTextContents();
  assert.ok(rows.some((r: string) => r.includes("唐揚げ") && r.includes("提供済み")), rows.join(" | "));
  assert.ok(rows.some((r: string) => r.includes("生ビール") && r.includes("調理中")), rows.join(" | "));

  // 店長がだし巻き卵を品切れにすると、客は注文できない(理由が日本語で出る)
  const m = await session("pc", token("店長", MANAGER));
  await m.page.goto(`${url}/s/${enc("品の詳細")}/m4`);
  const so = m.page.waitForResponse((r: any) => r.url().includes(enc("品切れにする")));
  await m.page.click(`button:has-text("品切れにする")`);
  assert.equal((await so).status(), 200);
  assert.equal(await order(t2.page, "だし巻き卵", "1"), 422);
  await t2.page.waitForFunction(() => /品切れ/.test(document.querySelector(".msg.error")?.textContent ?? ""));

  // 売上: 客は見られない、店長は見られる(金額は円表記)
  await t2.page.goto(`${url}/s/${enc("売上")}`);
  assert.match((await t2.page.textContent("main"))!, /権限がありません/);
  await m.page.goto(`${url}/s/${enc("売上")}`);
  const sales = (await m.page.textContent("main"))!;
  assert.ok(sales.includes("生ビール") && sales.includes("¥1,100") && sales.includes("¥680") && sales.includes("¥380"), sales);

  // A/B: 注文ボタン文言の成果が数えられている(店長は結果を見られる)
  const res = await fetch(`${url}/api/experiments/${enc("注文ボタン文言")}/results`, { headers: { Authorization: `Bearer ${token("店長", MANAGER)}` } });
  assert.equal(res.status, 200);
  const r = await res.json();
  assert.ok(r.variants.reduce((s: number, v: any) => s + v.exposures, 0) >= 2, JSON.stringify(r));

  for (const s of [t1, t2, k, m]) {
    assert.deepEqual(s.errors, []);
    await s.ctx.close();
  }
});

test("一覧の絞り込み API: 本人の行の制限は外れない・不正なフィールドや値は 400", async () => {
  const cust = token("9番卓", CUSTOMER);
  const get = (q: string) => fetch(`${url}/api/data/${enc("注文")}?${q}`, { headers: { Authorization: `Bearer ${cust}` } });
  const mine = await get(`${enc("ne.状態")}=${enc("提供済み")}`);
  assert.equal(mine.status, 200);
  assert.ok((await mine.json()).every((r: any) => r.卓 === "9番卓"));
  assert.equal((await get(`order=${enc("存在しない")}`)).status, 400);
  assert.equal((await get(`${enc("eq.状態")}=${enc("壊れた値")}`)).status, 400);
  assert.equal((await get(`${enc("eq.数量")}=abc`)).status, 400);
  assert.equal((await get(`${enc("today.品名")}=1`)).status, 400);
  // SQL を紛れ込ませても、フィールド名の照合で拒否される
  assert.equal((await get(`order=${enc('数量" DESC; DROP TABLE "注文')}`)).status, 400);
});

test("権限の境界: 客は提供済みにできない・品切れにできない・他の卓の注文を ID で見られない・数量の上限", async () => {
  const cust = token("3番卓", CUSTOMER);
  const other = token("4番卓", CUSTOMER);
  const post = (path: string, tok: string, body: unknown) =>
    fetch(`${url}/api/actions/${enc(path)}`, { method: "POST", headers: { Authorization: `Bearer ${tok}`, "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const mine = await (await post("注文する", cust, { メニューID: "m2", 数量: 1 })).json();
  assert.equal((await post("提供済みにする", cust, { 注文ID: mine.ok })).status, 403);
  assert.equal((await post("品切れにする", cust, { メニューID: "m1" })).status, 403);
  assert.equal((await fetch(`${url}/api/data/${enc("注文")}/${mine.ok}`, { headers: { Authorization: `Bearer ${other}` } })).status, 404);
  const big = await post("注文する", cust, { メニューID: "m2", 数量: 11 });
  assert.equal(big.status, 422);
  assert.equal((await big.json()).value, "数量不正");
  // 全件閲覧の許可が無い人は売上(本人限定データの分析)を見られない
  assert.equal((await fetch(`${url}/api/analyses/${enc("品名別の売上")}`, { headers: { Authorization: `Bearer ${token("x", ["注文.読取"])}` } })).status, 403);
});

test("役割: トークンの roles に役割名だけ入れれば、ソースの役割の権限を持つ", async () => {
  const { createHmac } = await import("node:crypto");
  const secret = readFileSync(join(dir, ".koto/dev/secret"), "utf8").trim();
  const b = (x: unknown) => Buffer.from(JSON.stringify(x)).toString("base64url");
  const jwt = (sub: string, roles: string[]) => {
    const h = `${b({ alg: "HS256", typ: "JWT" })}.${b({ sub, roles, exp: Math.floor(Date.now() / 1000) + 600 })}`;
    return `${h}.${createHmac("sha256", secret).update(h).digest("base64url")}`;
  };
  const cust = jwt("6番卓", ["客"]), kitchen = jwt("厨房", ["厨房"]);
  const post = (path: string, tok: string, body: unknown) =>
    fetch(`${url}/api/actions/${enc(path)}`, { method: "POST", headers: { Authorization: `Bearer ${tok}`, "Content-Type": "application/json" }, body: JSON.stringify(body) });
  const o = await post("注文する", cust, { メニューID: "m2", 数量: 1 });
  assert.equal(o.status, 200);
  const id = (await o.json()).ok;
  assert.equal((await post("提供済みにする", cust, { 注文ID: id })).status, 403);
  assert.equal((await post("提供済みにする", kitchen, { 注文ID: id })).status, 200);
  // 客の役割は自分の注文だけ、厨房は全員分
  const mine = await (await fetch(`${url}/api/data/${enc("注文")}`, { headers: { Authorization: `Bearer ${cust}` } })).json();
  const all = await (await fetch(`${url}/api/data/${enc("注文")}`, { headers: { Authorization: `Bearer ${kitchen}` } })).json();
  assert.ok(mine.every((x: any) => x.卓 === "6番卓") && all.length > mine.length);
});

test("同じ品を 20 卓が同時に注文しても、注文と売上の数が合う", async () => {
  const before = await (await fetch(`${url}/api/analyses/${enc("品名別の売上")}`, { headers: { Authorization: `Bearer ${token("店長", MANAGER)}` } })).json();
  const beer0 = before.find((x: any) => x.品名 === "生ビール")?.数量 ?? 0;
  const rs = await Promise.all(
    [...Array(20)].map((_, i) =>
      fetch(`${url}/api/actions/${enc("注文する")}`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token(`卓${i}`, CUSTOMER)}`, "Content-Type": "application/json" },
        body: JSON.stringify({ メニューID: "m1", 数量: 1 }),
      }).then((r) => r.status),
    ),
  );
  assert.ok(rs.every((s) => s === 200), rs.join(","));
  const after = await (await fetch(`${url}/api/analyses/${enc("品名別の売上")}`, { headers: { Authorization: `Bearer ${token("店長", MANAGER)}` } })).json();
  const beer = after.find((x: any) => x.品名 === "生ビール");
  assert.equal(beer.数量, beer0 + 20);
  assert.equal(beer.売上, beer.数量 * 550);
});

test("スマホでの客の画面がアクセシビリティ自動監査(WCAG 2.2 A/AA)で違反なし", async () => {
  const axe = readFileSync(require.resolve("axe-core/axe.min.js"), "utf8");
  const s = await session("phone", token("5番卓", CUSTOMER));
  const out: string[] = [];
  for (const path of [`/s/${enc("メニュー")}`, `/s/${enc("品の詳細")}/m1`, `/s/${enc("注文の状況")}`]) {
    await s.page.goto(url + path);
    await s.page.evaluate(axe);
    const v = await s.page.evaluate(async () => {
      const r = await (window as any).axe.run(document, { runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"] } });
      return r.violations.map((x: any) => `${x.id}: ${x.nodes.map((n: any) => n.target.join(" ")).join(", ")}`);
    });
    out.push(...v.map((x: string) => `${decodeURIComponent(path)} ${x}`));
  }
  await s.ctx.close();
  assert.deepEqual(out, []);
});
