// アプリの中のログイン(ADR-0142): 最初の管理者・招待・役割・失敗が続いたときの停止・印の取り消し・パスワードの保管
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { KOTO, tmp, freePort, start, waitHttp, SECRET } from "./helpers.ts";
import { hashPassword, verifyPassword, passwordProblem } from "../../runtime/web/local_login.ts";

let webUrl = "";
let usersFile = "";
let webDir = "";
const procs: ChildProcess[] = [];
const SETUP = "setup-token-for-e2e-0123456789";

before(async () => {
  const proj = join(tmp("local-login"), "受付");
  execFileSync(KOTO, ["init", proj, "--template", "予約受付"], { stdio: "pipe" });
  const be = tmp("ll-be");
  const web = tmp("ll-web");
  for (const [t, o] of [["backend", be], ["web", web]]) execFileSync(KOTO, ["-C", proj, "build", "--target", t, "--out", o], { stdio: "pipe" });
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: web, stdio: "inherit" });
  execFileSync("npm", ["run", "build", "--silent"], { cwd: web, stdio: "inherit" });
  webDir = web;
  usersFile = join(web, "data/users.json");
  const [bp, wp] = [await freePort(), await freePort()];
  procs.push(start(be, ["src/main.ts"], { KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_DB: join(be, "data/app.sqlite"), PORT: String(bp) }));
  procs.push(
    start(web, ["src/server-main.ts"], {
      PORT: String(wp),
      KOTONOHA_BACKEND_URL: `http://127.0.0.1:${bp}`,
      KOTONOHA_LOCAL_LOGIN: "1",
      KOTONOHA_JWT_SECRET: SECRET,
      KOTONOHA_USERS_FILE: usersFile,
      KOTONOHA_SETUP_TOKEN: SETUP,
    }),
  );
  webUrl = `http://127.0.0.1:${wp}`;
  await waitHttp(`http://127.0.0.1:${bp}/api/health`);
  await waitHttp(webUrl + "/login");
});
after(() => procs.forEach((p) => p.kill()));

const form = (o: Record<string, string | string[]>) => {
  const f = new URLSearchParams();
  for (const [k, v] of Object.entries(o)) for (const x of Array.isArray(v) ? v : [v]) f.append(k, x);
  return f;
};
async function post(path: string, body: Record<string, string | string[]>, cookie = "") {
  return fetch(webUrl + path, { method: "POST", body: form(body), redirect: "manual", headers: { "Content-Type": "application/x-www-form-urlencoded", Cookie: cookie } });
}
const cookieOf = (r: Response) => (r.headers.get("set-cookie") ?? "").split(";")[0];
async function api(path: string, cookie: string, body?: unknown) {
  return fetch(webUrl + path, body === undefined ? { headers: { Cookie: cookie } } : { method: "POST", headers: { Cookie: cookie, "Content-Type": "application/json", "X-CSRF": "1" }, body: JSON.stringify(body) });
}

let admin = "";
let member = "";
let frameId = "";

test("パスワードは scrypt で保管し、決まりに合わないものは断る", () => {
  const h = hashPassword("とても長いパスワード123");
  assert.match(h, /^scrypt\$32768\$8\$1\$/);
  assert.ok(verifyPassword("とても長いパスワード123", h));
  assert.ok(!verifyPassword("ちがうパスワード12345", h));
  assert.match(passwordProblem("short", "a@x") ?? "", /10 文字以上/);
  assert.match(passwordProblem("a@example.jp", "a@example.jp") ?? "", /文字以上|ログイン名と同じ/);
  assert.match(passwordProblem("aaaaaaaaaaaa", "x") ?? "", /推測されやすい/);
  assert.equal(passwordProblem("koto-nOha-2026!", "x"), null);
});

test("最初の管理者: 違う URL では作れず、正しい URL で作るとそのまま入れる。2 回目は作れない", async () => {
  const login = await fetch(webUrl + "/login");
  assert.match(await login.text(), /まだ利用者がいません/);
  const bad = await post("/setup", { t: "wrong-token-0000000000000", login: "boss@example.jp", name: "店長", password: "koto-nOha-2026!", password2: "koto-nOha-2026!" });
  assert.equal(bad.status, 400);
  const mismatch = await post("/setup", { t: SETUP, login: "boss@example.jp", name: "店長", password: "koto-nOha-2026!", password2: "koto-nOha-2026?" });
  assert.match(await mismatch.text(), /確認のパスワードが合いません/);
  const ok = await post("/setup", { t: SETUP, login: "Boss@Example.jp", name: "店長", password: "koto-nOha-2026!", password2: "koto-nOha-2026!" });
  assert.equal(ok.status, 303);
  admin = cookieOf(ok);
  assert.match(admin, /=.+\..+\./);
  const again = await post("/setup", { t: SETUP, login: "x@example.jp", name: "x", password: "koto-nOha-2026!", password2: "koto-nOha-2026!" });
  assert.equal(again.status, 404);
  // 平文のパスワードはファイルに無い
  const saved = readFileSync(usersFile, "utf8");
  assert.doesNotMatch(saved, /koto-nOha-2026!/);
  assert.match(saved, /"login": "boss@example.jp"/);
});

test("最初の管理者はすべての役割を持ち、受付係の操作ができる", async () => {
  const r = await api("/api/actions/" + encodeURIComponent("枠を追加"), admin, { 名前: "朝の回", 開始日時: "2026-12-05T10:00:00+09:00", 定員: 2 });
  assert.equal(r.status, 200, await r.clone().text());
  const j = (await r.json()) as any;
  frameId = String(typeof j === "object" && j !== null ? (j.value ?? j.result ?? j.id) : j);
  const page = await (await fetch(webUrl + "/login", { headers: { Cookie: admin } })).text();
  assert.match(page, /店長 さん\(boss@example.jp\)でログインしています/);
  assert.match(page, /利用者の管理/);
});

test("招待のリンクで登録した人は、付けた役割の操作だけができる", async () => {
  const r = await post("/users/invite", { role: "利用者" }, admin);
  const html = await r.text();
  const link = /\/invite\/([A-Za-z0-9_-]+)/.exec(html)?.[1];
  assert.ok(link, html);
  const reg = await post("/invite/" + link, { login: "hanako@example.jp", name: "花子", password: "hanako-pass-2026", password2: "hanako-pass-2026" });
  assert.equal(reg.status, 303);
  member = cookieOf(reg);
  // リンクは一度だけ
  const twice = await post("/invite/" + link, { login: "jiro@example.jp", name: "次郎", password: "jiro-pass-20266", password2: "jiro-pass-20266" });
  assert.equal(twice.status, 404);
  // 利用者は予約できるが、枠を追加できない
  const list = await api("/api/data/" + encodeURIComponent("枠"), member);
  assert.equal(list.status, 200);
  const add = await api("/api/actions/" + encodeURIComponent("枠を追加"), member, { 名前: "夜の回", 開始日時: "2026-12-05T19:00:00+09:00", 定員: 2 });
  assert.equal(add.status, 403);
  // 利用者の管理は管理者だけ
  const users = await fetch(webUrl + "/users", { headers: { Cookie: member } });
  assert.equal(users.status, 403);
  const forged = await post("/users/invite", { role: "受付係", admin: "1" }, member);
  assert.equal(forged.status, 403);
});

test("ログイン: 違うパスワードが 5 回続くと、正しいパスワードでも 15 分止める", async () => {
  const ok = await post("/login/local", { login: "hanako@example.jp", password: "hanako-pass-2026" });
  assert.equal(ok.status, 303);
  for (let i = 0; i < 5; i++) assert.equal((await post("/login/local", { login: "hanako@example.jp", password: "wrong-password-" + i })).status, 401);
  const blocked = await post("/login/local", { login: "hanako@example.jp", password: "hanako-pass-2026" });
  assert.equal(blocked.status, 429);
  // 知らないログイン名も同じ言葉で断る(いる人かどうかを教えない)
  const unknown = await post("/login/local", { login: "nobody@example.jp", password: "whatever-123456" });
  assert.match(await unknown.text(), /ログイン名かパスワードが違います/);
});

test("管理者が役割を変える・消すと、その人の古いログインの印は使えなくなる", async () => {
  const s = JSON.parse(readFileSync(usersFile, "utf8"));
  const hanako = s.users.find((u: any) => u.login === "hanako@example.jp");
  const r = await post("/users/" + encodeURIComponent(hanako.id), { role: [] }, admin);
  assert.match(await r.text(), /保存しました/);
  const page = await (await fetch(webUrl + "/login", { headers: { Cookie: member } })).text();
  assert.doesNotMatch(page, /花子 さん/);
  // 自分の管理者は外せない・自分は消せない
  const me = s.users.find((u: any) => u.login === "boss@example.jp");
  assert.match(await (await post("/users/" + encodeURIComponent(me.id), { role: ["受付係"] }, admin)).text(), /自分の管理者を外すことはできません/);
  assert.match(await (await post("/users/" + encodeURIComponent(me.id) + "/delete", {}, admin)).text(), /自分は消せません/);
  // 再設定のリンクでパスワードを決め直す
  const rr = await post("/users/" + encodeURIComponent(hanako.id) + "/reset", {}, admin);
  const link = /\/reset\/([A-Za-z0-9_-]+)/.exec(await rr.text())?.[1];
  assert.ok(link);
  const done = await post("/reset/" + link, { password: "hanako-new-2026", password2: "hanako-new-2026" });
  assert.equal(done.status, 303);
  const del = await post("/users/" + encodeURIComponent(hanako.id) + "/delete", {}, admin);
  assert.match(await del.text(), /消しました/);
  assert.equal((await api("/api/actions/" + encodeURIComponent("予約する"), cookieOf(done), { 枠ID: frameId, 人数: 1 })).status, 401);
});

test("別のサイトからのログインのフォームは受け付けない(ログイン CSRF)", async () => {
  const r = await fetch(webUrl + "/login/local", { method: "POST", body: form({ login: "boss@example.jp", password: "koto-nOha-2026!" }), redirect: "manual", headers: { "Content-Type": "application/x-www-form-urlencoded", Origin: "https://evil.example" } });
  assert.equal(r.status, 403);
});

test("本番の設定: アプリの中のログインなら OIDC が無くても起動でき、秘密が短いと起動しない", () => {
  const out = execFileSync(
    process.execPath,
    [
      "--no-warnings",
      "-e",
      `import(${JSON.stringify(join(webDir, "src/runtime/server.ts"))}).then((m) => { const base = { KOTONOHA_ENV: "production" }; console.log(JSON.stringify([m.checkWebProductionConfig(base, { secureCookie: true, devLogin: false, oidc: null, local: { secret: "x".repeat(64) } }), m.checkWebProductionConfig(base, { secureCookie: true, devLogin: false, oidc: null, local: { secret: "x".repeat(40) } }), m.checkWebProductionConfig(base, { secureCookie: true, devLogin: false, oidc: null, local: null })])); })`,
    ],
    { encoding: "utf8", cwd: webDir },
  );
  const [okLocal, shortSecret, none] = JSON.parse(out);
  assert.deepEqual(okLocal, []);
  assert.match(shortSecret.join(), /64 文字以上/);
  assert.match(none.join(), /ログインの仕組みが必要です/);
});
