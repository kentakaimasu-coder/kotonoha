// 生成 backend の権限・認証・運用機能の結合テスト
// ADR-0118(動作単位の許可・行レベル認可)/ ADR-002(JWKS RS256・ES256、ロール対応)/ K-05 マイグレーション / K-09 ページング / K-10 レート制限
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, sign as cryptoSign, createHmac } from "node:crypto";
import { cpSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync, type ChildProcess } from "node:child_process";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, shopSeed, SECRET, ROOT, KOTO } from "./helpers.ts";

const procs: ChildProcess[] = [];
after(() => procs.forEach((p) => p.kill()));
const p = (...segs: string[]) => "/" + segs.map(encodeURIComponent).join("/");

async function boot(env: Record<string, string>, opts: { dir?: string; seed?: boolean } = {}) {
  const dir = opts.dir ?? tmp("sec");
  if (!opts.dir) build("examples/shop", "backend", dir);
  const e = { KOTONOHA_DB: join(dir, "data/app.sqlite"), ...env };
  if (opts.seed !== false) runNode(dir, ["tools/seed.ts", seedFile(dir, shopSeed())], { ...e, KOTONOHA_JWT_SECRET: SECRET });
  const port = await freePort();
  const proc = start(dir, ["src/main.ts"], { ...e, PORT: String(port) });
  procs.push(proc);
  const url = `http://127.0.0.1:${port}`;
  await waitHttp(url + "/api/health");
  return { dir, url, proc, env: e };
}

async function api(url: string, method: string, path: string, tok?: string, body?: unknown) {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (tok) headers.Authorization = `Bearer ${tok}`;
  const r = await fetch(url + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  return { status: r.status, body: await r.json().catch(() => null), headers: r.headers };
}

const hs = (dir: string, sub: string, caps: string[]) => runNode(dir, ["tools/dev-token.ts", "--sub", sub, "--caps", caps.join(",")], { KOTONOHA_JWT_SECRET: SECRET });

test("ADR-0118 動作単位の許可: 動作.注文する だけで注文でき、他の更新系は呼べない", async () => {
  const { dir, url } = await boot({ KOTONOHA_JWT_SECRET: SECRET });
  const buyer = hs(dir, "buyer", ["動作.注文する"]);
  const r = await api(url, "POST", p("api", "actions", "注文する"), buyer, { 商品ID: "P01", 数量: 1 });
  assert.equal(r.status, 200, JSON.stringify(r.body));
  assert.equal((await api(url, "POST", p("api", "actions", "商品登録"), buyer, { 名前: "x", 価格: 1, 在庫: 1 })).status, 403);
});

test("ADR-0118 行レベル認可: 注文は注文者本人のものだけ見える", async () => {
  const { dir, url } = await boot({ KOTONOHA_JWT_SECRET: SECRET });
  const a = hs(dir, "alice", ["動作.注文する"]);
  const b = hs(dir, "bob", ["動作.注文する"]);
  await api(url, "POST", p("api", "actions", "注文する"), a, { 商品ID: "P01", 数量: 1 });
  await api(url, "POST", p("api", "actions", "注文する"), a, { 商品ID: "P01", 数量: 2 });
  await api(url, "POST", p("api", "actions", "注文する"), b, { 商品ID: "P01", 数量: 3 });
  const la = await api(url, "GET", p("api", "data", "注文"), a);
  assert.equal(la.status, 200);
  assert.deepEqual(la.body.map((o: any) => [o.注文者, o.数量]).sort(), [["alice", 1], ["alice", 2]]);
  assert.equal(la.headers.get("x-total-count"), "2", "件数も本人分だけ");
  const lb = await api(url, "GET", p("api", "data", "注文"), b);
  assert.deepEqual(lb.body.map((o: any) => o.注文者), ["bob"]);
  assert.equal((await api(url, "GET", p("api", "data", "注文"))).status, 401);
});

test("K-09 一覧のページング(limit / offset / X-Total-Count)", async () => {
  const { url } = await boot({ KOTONOHA_JWT_SECRET: SECRET });
  const r = await api(url, "GET", p("api", "data", "商品") + "?limit=2&offset=1");
  assert.equal(r.body.length, 2);
  assert.equal(r.body[0].ID, "P02");
  assert.equal(r.headers.get("x-total-count"), "3");
  for (const q of ["?limit=0", "?limit=1001", "?offset=-1", "?limit=abc"]) assert.equal((await api(url, "GET", p("api", "data", "商品") + q)).status, 400, q);
});

test("K-10 書込みのレート制限は 429", async () => {
  const { dir, url } = await boot({ KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_RATE_LIMIT_PER_MINUTE: "2" });
  const t = hs(dir, "spammer", ["動作.注文する"]);
  const codes = [];
  for (let i = 0; i < 3; i++) codes.push((await api(url, "POST", p("api", "actions", "注文する"), t, { 商品ID: "P01", 数量: 1 })).status);
  assert.deepEqual(codes, [200, 200, 429]);
});

function jwt(header: object, claims: object, signer: (input: Buffer) => Buffer): string {
  const h = Buffer.from(JSON.stringify(header)).toString("base64url");
  const c = Buffer.from(JSON.stringify(claims)).toString("base64url");
  return `${h}.${c}.${signer(Buffer.from(`${h}.${c}`)).toString("base64url")}`;
}

test("ADR-002 OIDC 形式: JWKS の RS256 / ES256 とロール → Capability の対応", async () => {
  const rsa = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const ec = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwks = { keys: [{ ...rsa.publicKey.export({ format: "jwk" }), kid: "r1", alg: "RS256", use: "sig" }, { ...ec.publicKey.export({ format: "jwk" }), kid: "e1", alg: "ES256" }] };
  const dir = tmp("jwks");
  build("examples/shop", "backend", dir);
  writeFileSync(join(dir, "jwks.json"), JSON.stringify(jwks));
  const { url } = await boot(
    { KOTONOHA_JWKS_FILE: join(dir, "jwks.json"), KOTONOHA_JWT_ISSUER: "https://idp.example", KOTONOHA_JWT_AUDIENCE: "shop", KOTONOHA_ROLE_MAP: JSON.stringify({ buyer: ["動作.注文する"] }) },
    { dir },
  );
  const exp = Math.floor(Date.now() / 1000) + 600;
  const claims = { sub: "oidc-user", iss: "https://idp.example", aud: ["shop", "other"], exp, roles: ["buyer"] };
  const rs = jwt({ alg: "RS256", kid: "r1", typ: "JWT" }, claims, (i) => cryptoSign("sha256", i, rsa.privateKey));
  const es = jwt({ alg: "ES256", kid: "e1" }, claims, (i) => cryptoSign("sha256", i, { key: ec.privateKey, dsaEncoding: "ieee-p1363" }));
  const order = (t: string) => api(url, "POST", p("api", "actions", "注文する"), t, { 商品ID: "P01", 数量: 1 });
  assert.equal((await order(rs)).status, 200, "RS256");
  assert.equal((await order(es)).status, 200, "ES256");
  // 発行者違い・受信者違い・鍵と alg の不一致・公開鍵を HMAC 秘密に使う混同攻撃・ロールなし
  const bad = [
    jwt({ alg: "RS256", kid: "r1" }, { ...claims, iss: "https://evil" }, (i) => cryptoSign("sha256", i, rsa.privateKey)),
    jwt({ alg: "RS256", kid: "r1" }, { ...claims, aud: "other" }, (i) => cryptoSign("sha256", i, rsa.privateKey)),
    jwt({ alg: "ES256", kid: "r1" }, claims, (i) => cryptoSign("sha256", i, { key: ec.privateKey, dsaEncoding: "ieee-p1363" })),
    jwt({ alg: "HS256", kid: "r1" }, claims, (i) => createHmac("sha256", rsa.publicKey.export({ format: "pem", type: "spki" })).update(i).digest()),
  ];
  for (const [i, t] of bad.entries()) assert.equal((await order(t)).status, 401, `bad[${i}]`);
  const noRole = jwt({ alg: "RS256", kid: "r1" }, { ...claims, roles: [] }, (i) => cryptoSign("sha256", i, rsa.privateKey));
  assert.equal((await order(noRole)).status, 403);
});

test("K-05 差分マイグレーション: 選択型の列追加はデータを保ったまま適用、破壊的変更は起動拒否", async () => {
  const proj = tmp("proj");
  cpSync(join(ROOT, "examples/shop"), proj, { recursive: true, filter: (s) => !s.includes("/build") });
  const be = tmp("mig");
  const gen = () => spawnSync(KOTO, ["-C", proj, "build", "--target", "backend", "--out", be]).status;
  assert.equal(gen(), 0);
  const first = await boot({ KOTONOHA_JWT_SECRET: SECRET }, { dir: be });
  first.proc.kill();
  await new Promise((r) => setTimeout(r, 300));
  const src = join(proj, "src/app.koto");
  const orig = readFileSync(src, "utf8");
  writeFileSync(src, orig.replace("    在庫: 整数\n", "    在庫: 整数\n    説明: 選択<文字列>\n"));
  assert.equal(gen(), 0);
  const second = await boot({ KOTONOHA_JWT_SECRET: SECRET }, { dir: be, seed: false });
  const r = await api(second.url, "GET", p("api", "data", "商品", "P01"));
  assert.deepEqual(r.body, { ID: "P01", 名前: "緑茶", 価格: 300, 在庫: 10, 説明: null }, "既存データは保持され、新しい列は null");
  second.proc.kill();
  await new Promise((r) => setTimeout(r, 300));
  // 必須列の追加(データあり)は起動拒否
  writeFileSync(src, orig.replace("    在庫: 整数\n", "    在庫: 整数\n    重さ: 整数\n").replace("アダプタ 商品を新規登録(名前: 文字列, 価格: 金額<JPY>, 在庫: 整数)", "アダプタ 商品を新規登録(名前: 文字列, 価格: 金額<JPY>, 在庫: 整数, 重さ: 整数)").replace("新ID = 商品を新規登録(名前, 価格, 在庫)", "新ID = 商品を新規登録(名前, 価格, 在庫, 0)"));
  assert.equal(gen(), 0);
  const run = spawnSync(process.execPath, ["--no-warnings", "src/main.ts"], { cwd: be, env: { ...process.env, KOTONOHA_DB: join(be, "data/app.sqlite"), PORT: String(await freePort()) }, timeout: 10000 });
  assert.notEqual(run.status, 0);
  assert.match(run.stderr.toString(), /自動移行できません: 必須列 重さ の追加/);
});

test("TC-012 SQLite でも同時注文で在庫が負にならない(取引の排他)", async () => {
  const { dir, url } = await boot({ KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" });
  const t = hs(dir, "buyer", ["動作.注文する"]);
  const rs = await Promise.all(Array.from({ length: 20 }, () => api(url, "POST", p("api", "actions", "注文する"), t, { 商品ID: "P01", 数量: 1 })));
  assert.deepEqual([rs.filter((r) => r.status === 200).length, rs.filter((r) => r.status === 422).length], [10, 10]);
  assert.equal((await api(url, "GET", p("api", "data", "商品", "P01"))).body.在庫, 0);
});

test("NR-02/NR-03 痕跡を出さないヘッダ・ヘルスチェック", async () => {
  const { url } = await boot({ KOTONOHA_JWT_SECRET: SECRET });
  const r = await fetch(url + "/api/health");
  assert.deepEqual(await r.json(), { ok: true }, "製品名・アプリ名を返さない");
  for (const path of ["/api/health", p("api", "data", "商品"), "/api/none"]) {
    const h = (await fetch(url + path)).headers;
    assert.equal(h.get("content-security-policy"), "default-src 'none'; frame-ancestors 'none'");
    assert.equal(h.get("x-content-type-options"), "nosniff");
    assert.equal(h.get("cross-origin-resource-policy"), "same-origin");
    assert.equal(h.get("x-powered-by"), null);
    for (const [k, v] of h) assert.ok(!/kotonoha/i.test(k) && !/kotonoha/i.test(v), `${k}: ${v}`);
  }
  const nf = await (await fetch(url + "/api/none")).text();
  assert.ok(!/kotonoha|node|sqlite/i.test(nf), nf);
});

test("NR-02 不正なトークンが続く接続元は一時的に遮断(429)", async () => {
  const { dir, url } = await boot({ KOTONOHA_JWT_SECRET: SECRET, KOTONOHA_AUTH_FAIL_LIMIT: "3" });
  const codes = [];
  for (let i = 0; i < 5; i++) codes.push((await api(url, "GET", p("api", "data", "注文"), "bad.token.value")).status);
  assert.deepEqual(codes, [401, 401, 401, 429, 429], "3 回目の失敗で遮断に入り、以後は 429");
  // 遮断するのは不正なトークンの試行だけ。同じ接続元(NAT 共有など)の正規の利用者は巻き込まない
  assert.equal((await api(url, "GET", p("api", "data", "注文"), hs(dir, "a", []))).status, 200);
});

test("NR-02 本番モードは安全でない設定で起動しない", async () => {
  const dir = tmp("prod");
  build("examples/shop", "backend", dir);
  const run = (env: Record<string, string>) =>
    spawnSync(process.execPath, ["--no-warnings", "src/main.ts"], { cwd: dir, env: { ...process.env, KOTONOHA_DB: ":memory:", KOTONOHA_ENV: "production", PORT: "0", ...env }, timeout: 10000 });
  const weak = run({ KOTONOHA_JWT_SECRET: SECRET });
  assert.notEqual(weak.status, 0);
  assert.match(weak.stderr.toString(), /64 文字以上/);
  const none = run({});
  assert.match(none.stderr.toString(), /認証鍵/);
});
