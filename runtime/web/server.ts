// KOTONOHA web runtime: SSR サーバー + 同一オリジンの API プロキシ + ログイン(Cookie)
// 認可はすべて backend が行う(SEC-03)。このサーバーは秘密情報を持たない(AT-10)。
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { createHash, createHmac, randomBytes } from "node:crypto";
import ReactDOMServer from "react-dom/server";
import { App, h, loaderUrl, type Init, type LoadResult, type ScreenDef } from "./ui.ts";

export interface WebApp {
  appName: string;
  entry: string | null;
  nav: string[];
  screens: Record<string, ScreenDef>;
}

const TOKEN_RE = /^[A-Za-z0-9_\-.]{1,4096}$/;
const MAX_BODY = 64 * 1024;

let hsts = false;
function securityHeaders(extra: Record<string, string | number | string[]> = {}): Record<string, string | number | string[]> {
  return {
    "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'",
    "X-Content-Type-Options": "nosniff",
    "Referrer-Policy": "same-origin",
    "X-Frame-Options": "DENY",
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Resource-Policy": "same-origin",
    "Permissions-Policy": "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
    ...(hsts ? { "Strict-Transport-Security": "max-age=31536000; includeSubDomains" } : {}),
    ...extra,
  };
}

/** 本番モード(KOTONOHA_ENV=production)で安全でない設定なら起動しない(NR-02) */
export function checkWebProductionConfig(env: NodeJS.ProcessEnv, opts: { secureCookie: boolean; devLogin: boolean; oidc: unknown }): string[] {
  if (env.KOTONOHA_ENV !== "production") return [];
  const p: string[] = [];
  if (!opts.secureCookie) p.push("本番では KOTONOHA_COOKIE_SECURE=1(HTTPS)が必要です");
  if (opts.devLogin) p.push("本番では開発用のトークン貼り付けログインを有効にできません");
  if (!opts.oidc) p.push("本番では OIDC(KOTONOHA_OIDC_ISSUER など)の設定が必要です");
  if (env.KOTONOHA_DEV_JWT_SECRET) p.push("本番では開発用のワンクリックログイン(KOTONOHA_DEV_JWT_SECRET)を使えません");
  return p;
}

function escHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string);
}

/** <script type="application/json"> に埋め込む JSON(</script> 等で抜けられないようにする) */
function jsonForScript(v: unknown): string {
  return JSON.stringify(v).replace(/</g, "\\u003c").replace(/>/g, "\\u003e").replace(/&/g, "\\u0026").replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
}

function cookieOf(req: IncomingMessage, name: string): string | null {
  const c = req.headers.cookie ?? "";
  for (const part of c.split(";")) {
    const [k, ...v] = part.trim().split("=");
    if (k === name) return v.join("=");
  }
  return null;
}

// Cookie 名に製品名を含めない(NR-03)。HTTPS では __Host- 接頭辞で Secure・Path=/・ドメイン無しを強制する
let COOKIE = "sid";
let OIDC_COOKIE = "oauth";

function tokenOf(req: IncomingMessage): string | null {
  const t = cookieOf(req, COOKIE);
  return t !== null && TOKEN_RE.test(t) ? t : null;
}

async function readBody(req: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const c of req) {
    size += (c as Buffer).length;
    if (size > MAX_BODY) throw new Error("too large");
    chunks.push(c as Buffer);
  }
  return Buffer.concat(chunks);
}

function page(title: string, body: string, initJson: string | null): string {
  return `<!doctype html>
<html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escHtml(title)}</title><link rel="stylesheet" href="/assets/app.css"></head>
<body><div id="root">${body}</div>${initJson === null ? "" : `<script id="__init__" type="application/json">${initJson}</script><script type="module" src="/assets/client.js"></script>`}</body></html>`;
}

/** OIDC 認可コードフロー(PKCE)の設定。issuer が無ければ無効(ADR-002) */
export interface OidcConfig {
  issuer: string;
  clientId: string;
  clientSecret: string | null;
  redirectUri: string;
  scope: string;
  /** backend に渡すトークン(既定 access_token) */
  token: "access_token" | "id_token";
}

export function oidcFromEnv(env: NodeJS.ProcessEnv): OidcConfig | null {
  if (!env.KOTONOHA_OIDC_ISSUER) return null;
  const issuer = env.KOTONOHA_OIDC_ISSUER.replace(/\/$/, "");
  const u = new URL(issuer);
  if (u.protocol !== "https:" && !["127.0.0.1", "localhost", "[::1]"].includes(u.hostname)) throw new Error("KOTONOHA_OIDC_ISSUER は https です");
  if (!env.KOTONOHA_OIDC_CLIENT_ID || !env.KOTONOHA_OIDC_REDIRECT_URI) throw new Error("KOTONOHA_OIDC_CLIENT_ID と KOTONOHA_OIDC_REDIRECT_URI が必要です");
  return {
    issuer,
    clientId: env.KOTONOHA_OIDC_CLIENT_ID,
    clientSecret: env.KOTONOHA_OIDC_CLIENT_SECRET || null,
    redirectUri: env.KOTONOHA_OIDC_REDIRECT_URI,
    scope: env.KOTONOHA_OIDC_SCOPE || "openid",
    token: env.KOTONOHA_OIDC_TOKEN === "id_token" ? "id_token" : "access_token",
  };
}

const b64u = (b: Buffer) => b.toString("base64url");

export function createWebServer(
  app: WebApp,
  opts: { backendUrl: string; root: string; secureCookie: boolean; tz: string; oidc?: OidcConfig | null; devLogin?: boolean; devQuick?: { secret: string; caps: string[]; roles?: Record<string, string[]> } | null },
) {
  const backend = new URL(opts.backendUrl);
  const oidc = opts.oidc ?? null;
  const devLogin = opts.devLogin ?? oidc === null;
  // 開発用のワンクリックログイン(koto dev が設定する)。名前を入れるだけで、アプリの全権限を持つ利用者として入れる
  const devQuick = devLogin && opts.devQuick && opts.devQuick.secret.length >= 32 ? opts.devQuick : null;
  const devToken = (sub: string, role: string) => {
    const b = (x: unknown) => Buffer.from(JSON.stringify(x)).toString("base64url");
    const now = Math.floor(Date.now() / 1000);
    // ソースの役割で入る / 一般の利用者(本人のものだけ)/ 管理者(全件閲覧も)
    const roles = devQuick!.roles ?? {};
    const caps = role.startsWith("r:") && roles[role.slice(2)] ? roles[role.slice(2)] : role === "admin" ? devQuick!.caps : devQuick!.caps.filter((c) => !c.startsWith("全件閲覧."));
    const head = `${b({ alg: "HS256", typ: "JWT" })}.${b({ sub, caps, iat: now, exp: now + 12 * 3600 })}`;
    return `${head}.${createHmac("sha256", devQuick!.secret).update(head).digest("base64url")}`;
  };
  let discovery: { authorization_endpoint: string; token_endpoint: string } | null = null;
  async function discover() {
    if (discovery) return discovery;
    const r = await fetch(`${oidc!.issuer}/.well-known/openid-configuration`, { signal: AbortSignal.timeout(10_000) });
    if (!r.ok) throw new Error(`OIDC discovery に失敗しました (${r.status})`);
    const d = (await r.json()) as { issuer: string; authorization_endpoint: string; token_endpoint: string };
    if (d.issuer.replace(/\/$/, "") !== oidc!.issuer) throw new Error("OIDC discovery の issuer が一致しません");
    discovery = d;
    return d;
  }
  const secure = opts.secureCookie ? "; Secure" : "";
  COOKIE = opts.secureCookie ? "__Host-sid" : "sid";
  OIDC_COOKIE = opts.secureCookie ? "__Secure-oauth" : "oauth";
  hsts = opts.secureCookie;
  const problems = checkWebProductionConfig(process.env, { secureCookie: opts.secureCookie, devLogin, oidc });
  if (problems.length > 0) throw new Error(`本番モードの設定に問題があります:\n- ${problems.join("\n- ")}`);
  const assets: Record<string, [string, string]> = {
    "/assets/client.js": ["dist/client.js", "text/javascript; charset=utf-8"],
    "/assets/app.css": ["assets/app.css", "text/css; charset=utf-8"],
  };

  // 実験(A/B テスト)の匿名の対象者 ID(ADR-0122)。個人を特定する情報は入れない
  const VID = opts.secureCookie ? "__Host-vid" : "vid";
  const vidOf = (req: IncomingMessage): string | null => {
    const v = cookieOf(req, VID);
    return v && /^[A-Za-z0-9_-]{16,64}$/.test(v) ? v : null;
  };
  async function backendFetch(path: string, init: RequestInit, token: string | null, traceId: string, vid: string | null = null): Promise<Response> {
    const headers = new Headers(init.headers);
    if (token) headers.set("Authorization", `Bearer ${token}`);
    if (vid) headers.set("X-Koto-Subject", vid);
    headers.set("X-Request-Id", traceId);
    return fetch(new URL(path, backend), { ...init, headers, redirect: "manual" });
  }

  async function load(def: ScreenDef, id: string | null, token: string | null, traceId: string, vid: string | null): Promise<Record<string, LoadResult>> {
    const out: Record<string, LoadResult> = {};
    for (const l of def.loaders) {
      try {
        const init: RequestInit = l.method === "POST" ? { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" } : { method: "GET" };
        const r = await backendFetch(loaderUrl(l, id), init, token, traceId, vid);
        let body: unknown = null;
        try {
          body = await r.json();
        } catch {
          body = null;
        }
        out[l.key] = { ok: r.ok, status: r.status, body };
      } catch {
        out[l.key] = { ok: false, status: 502, body: { message: "backend に接続できません" } };
      }
    }
    return out;
  }

  const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
    const traceId = crypto.randomUUID();
    const url = new URL(req.url ?? "/", "http://localhost");
    try {
      // 静的ファイル
      const asset = assets[url.pathname];
      if (asset) {
        const p = join(opts.root, asset[0]);
        if (!existsSync(p)) {
          res.writeHead(404, securityHeaders());
          return res.end();
        }
        res.writeHead(200, securityHeaders({ "Content-Type": asset[1], "Cache-Control": "no-cache" }));
        return res.end(readFileSync(p));
      }
      // API プロキシ
      if (url.pathname.startsWith("/api/")) {
        // Authorization: Bearer を付けた要求(プログラムからの呼出)は Cookie を使わないので CSRF の対象外。
        // ブラウザは他サイトからこのヘッダを付けられない(CORS の事前確認に応答しないため)
        const bearer = /^Bearer ([A-Za-z0-9_\-.]{1,8192})$/.exec(String(req.headers.authorization ?? ""))?.[1] ?? null;
        // CSRF が成り立つのは「Cookie のログインを使った、状態を変える要求」だけ。その場合に限り X-CSRF を求める
        const cookieAuth = bearer === null && tokenOf(req) !== null;
        const unsafe = req.method !== "GET" && req.method !== "HEAD";
        if (cookieAuth && unsafe && req.headers["x-csrf"] !== "1") {
          res.writeHead(403, securityHeaders({ "Content-Type": "application/json" }));
          return res.end(JSON.stringify({ error: "csrf", message: "X-CSRF ヘッダが必要です" }));
        }
        const origin = req.headers.origin;
        if (origin && new URL(origin).host !== req.headers.host) {
          res.writeHead(403, securityHeaders({ "Content-Type": "application/json" }));
          return res.end(JSON.stringify({ error: "csrf", message: "別オリジンからの要求です" }));
        }
        const body = req.method === "POST" ? await readBody(req) : undefined;
        const headers: Record<string, string> = {};
        if (req.headers["content-type"]) headers["Content-Type"] = String(req.headers["content-type"]);
        const r = await backendFetch(url.pathname + url.search, { method: req.method, headers, body }, bearer ?? tokenOf(req), traceId, bearer ? null : vidOf(req));
        const buf = Buffer.from(await r.arrayBuffer());
        res.writeHead(r.status, securityHeaders({ "Content-Type": r.headers.get("content-type") ?? "application/json", "Cache-Control": "no-store" }));
        return res.end(buf);
      }
      // ログイン(トークンは backend が検証する。ここでは形式だけ確認して HttpOnly Cookie に保存)
      // OIDC: 認可エンドポイントへ(state・nonce・PKCE を一時 Cookie に保存)
      if (url.pathname === "/login/oidc" && oidc) {
        const d = await discover();
        const state = b64u(randomBytes(16)), nonce = b64u(randomBytes(16)), verifier = b64u(randomBytes(32));
        const challenge = b64u(createHash("sha256").update(verifier).digest());
        const auth = new URL(d.authorization_endpoint);
        for (const [k, v] of Object.entries({ response_type: "code", client_id: oidc.clientId, redirect_uri: oidc.redirectUri, scope: oidc.scope, state, nonce, code_challenge: challenge, code_challenge_method: "S256" })) auth.searchParams.set(k, v);
        // 認可サーバーからの戻り(トップレベルの GET)で送られるよう SameSite=Lax
        res.writeHead(302, securityHeaders({ Location: auth.toString(), "Set-Cookie": `${OIDC_COOKIE}=${state}.${nonce}.${verifier}; HttpOnly; SameSite=Lax; Path=/login; Max-Age=600${secure}` }));
        return res.end();
      }
      if (url.pathname === "/login/callback" && oidc) {
        const fail = (msg: string) => {
          res.writeHead(400, securityHeaders({ "Content-Type": "text/html; charset=utf-8", "Set-Cookie": `${OIDC_COOKIE}=; HttpOnly; Path=/login; Max-Age=0` }));
          res.end(page("ログイン", `<section class="page"><h1>ログインできませんでした</h1><p role="alert">${escHtml(msg)}</p><a href="/login">戻る</a></section>`, null));
        };
        const saved = cookieOf(req, OIDC_COOKIE)?.split(".") ?? [];
        if (url.searchParams.get("error")) return fail(`認可サーバーが拒否しました: ${url.searchParams.get("error")}`);
        if (saved.length !== 3 || url.searchParams.get("state") !== saved[0]) return fail("state が一致しません(やり直してください)");
        const code = url.searchParams.get("code");
        if (!code) return fail("認可コードがありません");
        const d = await discover();
        const form = new URLSearchParams({ grant_type: "authorization_code", code, redirect_uri: oidc.redirectUri, client_id: oidc.clientId, code_verifier: saved[2] });
        if (oidc.clientSecret) form.set("client_secret", oidc.clientSecret);
        const tr = await fetch(d.token_endpoint, { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded", Accept: "application/json" }, body: form, signal: AbortSignal.timeout(10_000) });
        if (!tr.ok) return fail(`トークンを取得できませんでした (${tr.status})`);
        const tokens = (await tr.json()) as Record<string, unknown>;
        // id_token の nonce を照合する(署名は backend が JWKS で検証する)
        if (typeof tokens.id_token === "string") {
          const claims = JSON.parse(Buffer.from(tokens.id_token.split(".")[1] ?? "", "base64url").toString("utf8") || "{}");
          if (claims.nonce !== saved[1]) return fail("nonce が一致しません");
        }
        const t = tokens[oidc.token];
        if (typeof t !== "string" || !TOKEN_RE.test(t)) return fail(`${oidc.token} がありません`);
        res.writeHead(303, securityHeaders({ Location: "/", "Set-Cookie": [`${COOKIE}=${t}; HttpOnly; SameSite=Strict; Path=/${secure}`, `${OIDC_COOKIE}=; HttpOnly; Path=/login; Max-Age=0`] }));
        return res.end();
      }
      // ログイン・ログアウトの POST を他サイトから送らせない(ログイン CSRF 対策。ペルソナ評価で指摘)
      if ((url.pathname === "/login" || url.pathname === "/logout" || url.pathname === "/login/dev") && req.method === "POST") {
        const origin = req.headers.origin;
        const site = req.headers["sec-fetch-site"];
        if ((origin && origin !== "null" && new URL(origin).host !== req.headers.host) || origin === "null" || site === "cross-site") {
          res.writeHead(403, securityHeaders({ "Content-Type": "text/plain; charset=utf-8" }));
          return res.end("別のサイトからの要求は受け付けません");
        }
      }
      if (url.pathname === "/login/dev" && req.method === "POST" && devQuick) {
        const form = new URLSearchParams((await readBody(req)).toString("utf8"));
        const name = (form.get("name") ?? "").trim() || "開発者";
        if (name.length > 64 || /[\u0000-\u001f]/.test(name)) {
          res.writeHead(400, securityHeaders({ "Content-Type": "text/plain; charset=utf-8" }));
          return res.end("名前は 64 文字以内にしてください");
        }
        res.writeHead(303, securityHeaders({ Location: "/", "Set-Cookie": `${COOKIE}=${devToken(name, form.get("role") ?? "user")}; HttpOnly; SameSite=Strict; Path=/${secure}` }));
        return res.end();
      }
      if (url.pathname === "/login") {
        if (req.method === "POST") {
          if (!devLogin) {
            res.writeHead(404, securityHeaders());
            return res.end();
          }
          const form = new URLSearchParams((await readBody(req)).toString("utf8"));
          const t = form.get("token") ?? "";
          if (!TOKEN_RE.test(t)) {
            res.writeHead(400, securityHeaders({ "Content-Type": "text/html; charset=utf-8" }));
            return res.end(page("ログイン", `<p role="alert">トークンの形式が不正です</p><a href="/login">戻る</a>`, null));
          }
          const cookie = `${COOKIE}=${t}; HttpOnly; SameSite=Strict; Path=/${opts.secureCookie ? "; Secure" : ""}`;
          res.writeHead(303, securityHeaders({ Location: "/", "Set-Cookie": cookie }));
          return res.end();
        }
        res.writeHead(200, securityHeaders({ "Content-Type": "text/html; charset=utf-8" }));
        return res.end(
          page(
            "ログイン",
            `<section class="page"><h1>ログイン</h1>${oidc ? `<p><a class="button" href="/login/oidc">認証基盤でログイン</a></p>` : ""}${devQuick ? `<form method="post" action="/login/dev" class="form"><h2>開発用: 名前を入れてすぐ入る</h2><p>開発中だけの機能です。別の名前で入ると別の人として扱われます。「本人のものだけ閲覧可」のデータは、一般の利用者なら自分の分だけ、管理者なら全員の分が見えます。</p><div class="field"><label for="dev-name">名前</label><input id="dev-name" name="name" value="開発者" maxlength="64" autocomplete="off"></div>${Object.keys(devQuick.roles ?? {}).map((r) => `<button type="submit" name="role" value="r:${escHtml(r)}">${escHtml(r)}として入る</button> `).join("")}<button type="submit" name="role" value="user">一般の利用者として入る</button> <button type="submit" name="role" value="admin">管理者として入る</button></form>` : ""}${devLogin ? `<p>開発用: 発行済みのトークンを入力してログインします。</p><form method="post" action="/login"><div class="field"><label for="token">トークン</label><input id="token" name="token" type="password" required autocomplete="off"></div><button type="submit">ログイン</button></form>` : ""}<form method="post" action="/logout"><button type="submit">ログアウト</button></form></section>`,
            null,
          ),
        );
      }
      if (url.pathname === "/logout" && req.method === "POST") {
        res.writeHead(303, securityHeaders({ Location: "/", "Set-Cookie": `${COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0` }));
        return res.end();
      }
      if (url.pathname === "/") {
        res.writeHead(app.entry ? 302 : 404, securityHeaders({ Location: app.entry ? "/s/" + encodeURIComponent(app.entry) : "" }));
        return res.end();
      }
      // 画面(SSR)
      const segs = url.pathname.split("/").filter((s) => s).map((s) => decodeURIComponent(s));
      if (segs[0] === "s" && (segs.length === 2 || segs.length === 3)) {
        const def = app.screens[segs[1]];
        const id = segs[2] ?? null;
        if (def && (def.input === null) === (id === null)) {
          const hasExp = def.loaders.some((l) => l.method === "POST");
          let vid = vidOf(req);
          const extra: Record<string, string> = {};
          if (hasExp && vid === null) {
            vid = b64u(randomBytes(16));
            extra["Set-Cookie"] = `${VID}=${vid}; HttpOnly; SameSite=Lax; Path=/; Max-Age=31536000${secure}`;
          }
          const data = await load(def, id, tokenOf(req), traceId, vid);
          const init: Init = { appName: app.appName, screen: def.name, id, data, tz: opts.tz, nav: app.nav, loggedIn: tokenOf(req) !== null };
          const html = ReactDOMServer.renderToString(h(App, { init, screens: app.screens }));
          const notFound = Object.values(data).some((d) => d.status === 404);
          res.writeHead(notFound ? 404 : 200, securityHeaders({ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store", ...extra }));
          return res.end(page(`${def.name} - ${app.appName}`, html, jsonForScript(init)));
        }
      }
      res.writeHead(404, securityHeaders({ "Content-Type": "text/html; charset=utf-8" }));
      // 画面の URL は /s/画面名(入力のある画面は /s/画面名/ID)。迷ったときのために入口の画面を並べる
      const links = app.nav.map((n) => `<li><a href="/s/${encodeURIComponent(n)}">${escHtml(n)}</a></li>`).join("");
      res.end(page("見つかりません", `<section class="page"><h1>見つかりません</h1><p>画面の URL は <code>/s/画面名</code> の形です。</p><ul>${links}</ul><a href="/">トップへ</a></section>`, null));
    } catch (e) {
      // fetch の失敗は原因(cause: 接続拒否・禁止ポートなど)まで記録する
      const cause = (e as { cause?: unknown })?.cause;
      process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), level: "error", trace_id: traceId, msg: String((e as Error)?.stack ?? e), cause: cause ? String((cause as Error)?.message ?? cause) : undefined }) + "\n");
      if (!res.headersSent) res.writeHead(500, securityHeaders({ "Content-Type": "text/plain; charset=utf-8" }));
      res.end("内部エラー");
    }
  });
  // 遅いリクエストで接続を占有させない(slowloris 対策)
  server.headersTimeout = 10_000;
  server.requestTimeout = 30_000;
  server.keepAliveTimeout = 5_000;
  return server;
}
