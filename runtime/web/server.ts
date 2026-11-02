// KOTONOHA web runtime: SSR サーバー + 同一オリジンの API プロキシ + ログイン(Cookie)
// 認可はすべて backend が行う(SEC-03)。このサーバーは秘密情報を持たない(AT-10)。
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import ReactDOMServer from "react-dom/server";
import { App, h, loaderUrl, type Init, type LoadResult, type ScreenDef } from "./ui.ts";

export interface WebApp {
  appName: string;
  entry: string | null;
  nav: string[];
  screens: Record<string, ScreenDef>;
}

const COOKIE = "kotonoha_token";
const TOKEN_RE = /^[A-Za-z0-9_\-.]{1,4096}$/;
const MAX_BODY = 64 * 1024;

function securityHeaders(extra: Record<string, string | number> = {}): Record<string, string | number> {
  return {
    "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
    "X-Content-Type-Options": "nosniff",
    "Referrer-Policy": "same-origin",
    "X-Frame-Options": "DENY",
    ...extra,
  };
}

function escHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string);
}

/** <script type="application/json"> に埋め込む JSON(</script> 等で抜けられないようにする) */
function jsonForScript(v: unknown): string {
  return JSON.stringify(v).replace(/</g, "\\u003c").replace(/>/g, "\\u003e").replace(/&/g, "\\u0026").replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
}

function tokenOf(req: IncomingMessage): string | null {
  const c = req.headers.cookie ?? "";
  for (const part of c.split(";")) {
    const [k, ...v] = part.trim().split("=");
    if (k === COOKIE) {
      const t = v.join("=");
      return TOKEN_RE.test(t) ? t : null;
    }
  }
  return null;
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
<body><div id="root">${body}</div>${initJson === null ? "" : `<script id="__KOTONOHA_INIT__" type="application/json">${initJson}</script><script type="module" src="/assets/client.js"></script>`}</body></html>`;
}

export function createWebServer(app: WebApp, opts: { backendUrl: string; root: string; secureCookie: boolean; tz: string }) {
  const backend = new URL(opts.backendUrl);
  const assets: Record<string, [string, string]> = {
    "/assets/client.js": ["dist/client.js", "text/javascript; charset=utf-8"],
    "/assets/app.css": ["assets/app.css", "text/css; charset=utf-8"],
  };

  async function backendFetch(path: string, init: RequestInit, token: string | null, traceId: string): Promise<Response> {
    const headers = new Headers(init.headers);
    if (token) headers.set("Authorization", `Bearer ${token}`);
    headers.set("X-Request-Id", traceId);
    return fetch(new URL(path, backend), { ...init, headers, redirect: "manual" });
  }

  async function load(def: ScreenDef, id: string | null, token: string | null, traceId: string): Promise<Record<string, LoadResult>> {
    const out: Record<string, LoadResult> = {};
    for (const l of def.loaders) {
      try {
        const r = await backendFetch(loaderUrl(l, id), { method: "GET" }, token, traceId);
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

  return createServer(async (req: IncomingMessage, res: ServerResponse) => {
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
        if (req.headers["x-kotonoha-request"] !== "1") {
          res.writeHead(403, securityHeaders({ "Content-Type": "application/json" }));
          return res.end(JSON.stringify({ error: "csrf", message: "X-Kotonoha-Request ヘッダが必要です" }));
        }
        const origin = req.headers.origin;
        if (origin && new URL(origin).host !== req.headers.host) {
          res.writeHead(403, securityHeaders({ "Content-Type": "application/json" }));
          return res.end(JSON.stringify({ error: "csrf", message: "別オリジンからの要求です" }));
        }
        const body = req.method === "POST" ? await readBody(req) : undefined;
        const headers: Record<string, string> = {};
        if (req.headers["content-type"]) headers["Content-Type"] = String(req.headers["content-type"]);
        const r = await backendFetch(url.pathname + url.search, { method: req.method, headers, body }, tokenOf(req), traceId);
        const buf = Buffer.from(await r.arrayBuffer());
        res.writeHead(r.status, securityHeaders({ "Content-Type": r.headers.get("content-type") ?? "application/json", "Cache-Control": "no-store" }));
        return res.end(buf);
      }
      // ログイン(トークンは backend が検証する。ここでは形式だけ確認して HttpOnly Cookie に保存)
      if (url.pathname === "/login") {
        if (req.method === "POST") {
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
            `<section class="page"><h1>ログイン</h1><p>認証基盤(ADR-002)から発行されたトークンを入力してください。</p><form method="post" action="/login"><div class="field"><label for="token">トークン</label><input id="token" name="token" type="password" required autocomplete="off"></div><button type="submit">ログイン</button></form><form method="post" action="/logout"><button type="submit">ログアウト</button></form></section>`,
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
          const data = await load(def, id, tokenOf(req), traceId);
          const init: Init = { appName: app.appName, screen: def.name, id, data, tz: opts.tz, nav: app.nav };
          const html = ReactDOMServer.renderToString(h(App, { init, screens: app.screens }));
          const notFound = Object.values(data).some((d) => d.status === 404);
          res.writeHead(notFound ? 404 : 200, securityHeaders({ "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" }));
          return res.end(page(`${def.name} - ${app.appName}`, html, jsonForScript(init)));
        }
      }
      res.writeHead(404, securityHeaders({ "Content-Type": "text/html; charset=utf-8" }));
      res.end(page("見つかりません", `<section class="page"><h1>見つかりません</h1><a href="/">トップへ</a></section>`, null));
    } catch (e) {
      process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), level: "error", trace_id: traceId, msg: String((e as Error)?.stack ?? e) }) + "\n");
      if (!res.headersSent) res.writeHead(500, securityHeaders({ "Content-Type": "text/plain; charset=utf-8" }));
      res.end("内部エラー");
    }
  });
}
