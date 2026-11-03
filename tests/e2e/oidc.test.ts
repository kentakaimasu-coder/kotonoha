// Web の OIDC ログイン(認可コード + PKCE)と backend の JWKS 検証を、模擬 IdP で通しで確かめる(ADR-002 / K-08)
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { createServer, type Server } from "node:http";
import { createHash, generateKeyPairSync, randomBytes, sign } from "node:crypto";
import { execFileSync, type ChildProcess } from "node:child_process";
import { join } from "node:path";
import { build, tmp, freePort, start, waitHttp, seedFile, runNode, shopSeed, SECRET } from "./helpers.ts";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const procs: ChildProcess[] = [];
let idp: Server;
let browser: any;
let webUrl = "";
const issued: { verifierOk: boolean; clientOk: boolean }[] = [];

before(async () => {
  // ---- 模擬 IdP ----
  const { publicKey, privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const idpPort = await freePort();
  const issuer = `http://127.0.0.1:${idpPort}`;
  const codes = new Map<string, { challenge: string; nonce: string; redirect: string }>();
  const jwt = (claims: object) => {
    const h = Buffer.from(JSON.stringify({ alg: "RS256", kid: "k1", typ: "JWT" })).toString("base64url");
    const c = Buffer.from(JSON.stringify(claims)).toString("base64url");
    return `${h}.${c}.${sign("sha256", Buffer.from(`${h}.${c}`), privateKey).toString("base64url")}`;
  };
  idp = createServer(async (req, res) => {
    const u = new URL(req.url!, issuer);
    if (u.pathname === "/.well-known/openid-configuration") {
      res.end(JSON.stringify({ issuer, authorization_endpoint: `${issuer}/authorize`, token_endpoint: `${issuer}/token`, jwks_uri: `${issuer}/jwks` }));
    } else if (u.pathname === "/jwks") {
      res.end(JSON.stringify({ keys: [{ ...publicKey.export({ format: "jwk" }), kid: "k1", alg: "RS256", use: "sig" }] }));
    } else if (u.pathname === "/authorize") {
      // 利用者の同意を省略して即座に認可コードを返す
      assert.equal(u.searchParams.get("code_challenge_method"), "S256");
      const code = randomBytes(8).toString("hex");
      codes.set(code, { challenge: u.searchParams.get("code_challenge")!, nonce: u.searchParams.get("nonce")!, redirect: u.searchParams.get("redirect_uri")! });
      res.writeHead(302, { Location: `${u.searchParams.get("redirect_uri")}?code=${code}&state=${u.searchParams.get("state")}` }).end();
    } else if (u.pathname === "/token" && req.method === "POST") {
      let body = "";
      for await (const c of req) body += c;
      const f = new URLSearchParams(body);
      const saved = codes.get(f.get("code") ?? "");
      const verifierOk = !!saved && createHash("sha256").update(f.get("code_verifier") ?? "").digest("base64url") === saved.challenge;
      const clientOk = f.get("client_id") === "shop-web" && f.get("client_secret") === "client-secret" && f.get("redirect_uri") === saved?.redirect;
      issued.push({ verifierOk, clientOk });
      if (!saved || !verifierOk || !clientOk) return res.writeHead(400).end(JSON.stringify({ error: "invalid_grant" }));
      codes.delete(f.get("code")!);
      const exp = Math.floor(Date.now() / 1000) + 600;
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({
        token_type: "Bearer",
        access_token: jwt({ iss: issuer, aud: "shop-api", sub: "oidc-alice", exp, roles: ["buyer"] }),
        id_token: jwt({ iss: issuer, aud: "shop-web", sub: "oidc-alice", exp, nonce: saved.nonce }),
      }));
    } else res.writeHead(404).end();
  });
  await new Promise<void>((r) => idp.listen(idpPort, "127.0.0.1", r));

  // ---- 生成物 ----
  const be = tmp("oidc-be"), web = tmp("oidc-web");
  build("examples/shop", "backend", be);
  build("examples/shop", "web", web);
  runNode(be, ["tools/seed.ts", seedFile(be, shopSeed())], { KOTONOHA_DB: join(be, "data/app.sqlite"), KOTONOHA_JWT_SECRET: SECRET });
  execFileSync("npm", ["install", "--no-audit", "--no-fund", "--silent"], { cwd: web, stdio: "inherit" });
  execFileSync("npm", ["run", "build", "--silent"], { cwd: web, stdio: "inherit" });
  const [bp, wp] = [await freePort(), await freePort()];
  webUrl = `http://127.0.0.1:${wp}`;
  procs.push(start(be, ["src/main.ts"], {
    PORT: String(bp), KOTONOHA_DB: join(be, "data/app.sqlite"),
    KOTONOHA_JWKS_URL: `${issuer}/jwks`, KOTONOHA_JWT_ISSUER: issuer, KOTONOHA_JWT_AUDIENCE: "shop-api",
    KOTONOHA_ROLE_MAP: JSON.stringify({ buyer: ["動作.注文する"] }),
  }));
  procs.push(start(web, ["src/server-main.ts"], {
    PORT: String(wp), KOTONOHA_BACKEND_URL: `http://127.0.0.1:${bp}`,
    KOTONOHA_OIDC_ISSUER: issuer, KOTONOHA_OIDC_CLIENT_ID: "shop-web", KOTONOHA_OIDC_CLIENT_SECRET: "client-secret",
    KOTONOHA_OIDC_REDIRECT_URI: `${webUrl}/login/callback`,
  }));
  await waitHttp(`http://127.0.0.1:${bp}/api/health`);
  await waitHttp(webUrl + "/login");
  await new Promise((r) => setTimeout(r, 300)); // JWKS の初回取得
  browser = await chromium.launch();
});
after(async () => {
  await browser?.close();
  procs.forEach((p) => p.kill());
  idp?.close();
});

test("OIDC でログインし、IdP のロールで購入できる", async () => {
  const page = await (await browser.newContext()).newPage();
  await page.goto(webUrl + "/login");
  assert.equal(await page.locator("#token").count(), 0, "OIDC 設定時は開発用のトークン貼り付けを出さない");
  await page.click("text=認証基盤でログイン");
  await page.waitForURL(/\/s\//);
  assert.deepEqual(issued.at(-1), { verifierOk: true, clientOk: true }, "PKCE とクライアント認証");
  await page.goto(webUrl + "/s/" + encodeURIComponent("商品詳細") + "/P01");
  await page.click("text=購入");
  await page.waitForSelector(".msg.ok");
  await page.goto(webUrl + "/s/" + encodeURIComponent("注文履歴"));
  assert.ok((await page.textContent("table.list")).includes("oidc-alice"), "注文者は IdP の sub");
});

test("state の不一致・直接のコールバックは拒否", async () => {
  const r = await fetch(webUrl + "/login/callback?code=x&state=forged", { redirect: "manual" });
  assert.equal(r.status, 400);
  assert.match(await r.text(), /state が一致しません/);
  const p = await fetch(webUrl + "/login", { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body: "token=abc" });
  assert.equal(p.status, 404, "OIDC 設定時はトークン貼り付けログインを受け付けない");
});
