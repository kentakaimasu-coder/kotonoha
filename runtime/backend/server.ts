// KOTONOHA backend runtime: HTTP サーバー(認証境界・認可・入力検証・構造化ログ)
// 認可はここで強制する(SEC-03: UI の表示/非表示に依存しない)。
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { randomUUID } from "node:crypto";
import { authConfigFromEnv, startJwksRefresh, verifyToken, type AuthConfig } from "./auth.ts";
import type { Ctx, Database, Principal } from "./db.ts";
import { Failure, HttpError, RuntimeFault, toJson, type Schema } from "./rt.ts";

export interface Access {
  public: boolean;
  /** 資源 Capability(すべて必要) */
  capabilities: string[];
  /** 動作単位の許可(これを持てば capabilities は不要: ADR-0118) */
  grant?: string;
  /** 行レベル認可の対象データ(本人の行だけ) */
  owner?: string;
}

export interface Route {
  id: string;
  method: "GET" | "POST";
  segments: string[];
  hasId: boolean;
  access: Access;
  handle: (ctx: Ctx, req: { id: string | null; query: URLSearchParams; body: unknown }) => Promise<{ status: number; body: unknown; headers?: Record<string, string> }>;
}

export interface App {
  name: string;
  schema: Schema;
  routes: Route[];
  /** 行レベル認可: データ → 本人の主体 ID を入れるフィールド */
  owners: Record<string, string>;
}

/** 書込み系(POST)のレート制限: 主体または IP ごとのトークンバケット */
class RateLimiter {
  buckets = new Map<string, { tokens: number; at: number }>();
  perMinute: number;
  constructor(perMinute: number) {
    this.perMinute = perMinute;
  }
  allow(key: string): boolean {
    if (this.perMinute <= 0) return true;
    const now = Date.now();
    const b = this.buckets.get(key) ?? { tokens: this.perMinute, at: now };
    b.tokens = Math.min(this.perMinute, b.tokens + ((now - b.at) / 60000) * this.perMinute);
    b.at = now;
    if (b.tokens < 1) {
      this.buckets.set(key, b);
      return false;
    }
    b.tokens -= 1;
    this.buckets.set(key, b);
    if (this.buckets.size > 10000) this.buckets.clear();
    return true;
  }
}

const MAX_BODY = 64 * 1024;

function log(entry: Record<string, unknown>): void {
  process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), ...entry }) + "\n");
}

function send(res: ServerResponse, status: number, body: unknown, traceId: string, extra: Record<string, string> = {}): void {
  const text = toJson(body);
  res.writeHead(status, {
    ...extra,
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(text),
    "X-Content-Type-Options": "nosniff",
    "Cache-Control": "no-store",
    "X-Trace-Id": traceId,
  });
  res.end(text);
}

async function readBody(req: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const c of req) {
    size += (c as Buffer).length;
    if (size > MAX_BODY) throw new HttpError(413, "payload_too_large", "リクエストが大きすぎます");
    chunks.push(c as Buffer);
  }
  if (size === 0) return {};
  const ct = req.headers["content-type"] ?? "";
  if (!/^application\/json\b/i.test(ct)) throw new HttpError(415, "unsupported_media_type", "Content-Type は application/json です");
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new HttpError(400, "invalid_json", "JSON として解釈できません");
  }
}

function principalOf(req: IncomingMessage, cfg: AuthConfig): Principal | null {
  const h = req.headers.authorization ?? "";
  const m = /^Bearer ([A-Za-z0-9_\-.]+)$/.exec(h);
  return m ? verifyToken(m[1], cfg) : null;
}

/** パスを照合する。セグメントは percent-decode して比較する。 */
function match(routes: Route[], method: string, path: string): { route: Route; id: string | null } | null | "method" {
  let segs: string[];
  try {
    segs = path.split("/").filter((s) => s.length > 0).map((s) => decodeURIComponent(s));
  } catch {
    return null;
  }
  let methodMismatch = false;
  for (const r of routes) {
    const n = r.segments.length + (r.hasId ? 1 : 0);
    if (segs.length !== n) continue;
    if (!r.segments.every((s, i) => s === segs[i])) continue;
    if (r.method !== method) {
      methodMismatch = true;
      continue;
    }
    return { route: r, id: r.hasId ? segs[n - 1] : null };
  }
  return methodMismatch ? "method" : null;
}

export function createApp(app: App, db: Database, env: NodeJS.ProcessEnv = process.env) {
  const cfg = authConfigFromEnv(env);
  if (env.KOTONOHA_JWKS_URL) startJwksRefresh(cfg, env.KOTONOHA_JWKS_URL);
  if (cfg.secret === null && cfg.keys.size === 0 && !env.KOTONOHA_JWKS_URL)
    log({ level: "warn", msg: "認証鍵(KOTONOHA_JWT_SECRET または JWKS)が未設定のため、認証が必要な API はすべて 401 になります" });
  const limiter = new RateLimiter(Number(env.KOTONOHA_RATE_LIMIT_PER_MINUTE ?? 120));
  return createServer(async (req, res) => {
    const traceId = /^[A-Za-z0-9-]{8,64}$/.test(String(req.headers["x-request-id"] ?? "")) ? String(req.headers["x-request-id"]) : randomUUID();
    const started = performance.now();
    const url = new URL(req.url ?? "/", "http://localhost");
    let status = 500;
    let sub: string | null = null;
    try {
      if (url.pathname === "/api/health") {
        status = 200;
        return send(res, 200, { ok: true, app: app.name }, traceId);
      }
      const m = match(app.routes, req.method ?? "GET", url.pathname);
      if (m === "method") throw new HttpError(405, "method_not_allowed", "メソッドが違います");
      if (m === null) throw new HttpError(404, "not_found", "API がありません");
      const principal = principalOf(req, cfg);
      sub = principal?.sub ?? null;
      if (!m.route.access.public) {
        if (principal === null) throw new HttpError(401, "unauthenticated", "認証が必要です");
        const granted = m.route.access.grant !== undefined && principal.caps.has(m.route.access.grant);
        if (!granted) {
          const missing = m.route.access.capabilities.filter((c) => !principal.caps.has(c));
          if (missing.length > 0)
            throw new HttpError(403, "forbidden", `権限がありません: ${m.route.access.grant ? m.route.access.grant + " または " : ""}${missing.join(", ")}`);
        }
      }
      if (m.route.method === "POST" && !limiter.allow(principal?.sub ?? req.socket.remoteAddress ?? "?")) {
        throw new HttpError(429, "rate_limited", "要求が多すぎます。しばらく待ってから再試行してください");
      }
      const body = m.route.method === "POST" ? await readBody(req) : {};
      const ctx: Ctx = { db, conn: db.conn, schema: app.schema, owners: app.owners, dirty: null, principal, traceId };
      const r = await m.route.handle(ctx, { id: m.id, query: url.searchParams, body });
      status = r.status;
      send(res, r.status, r.body, traceId, r.headers);
    } catch (e) {
      if (e instanceof HttpError) {
        status = e.status;
        send(res, e.status, { error: e.code, message: e.message }, traceId);
      } else if (e instanceof Failure) {
        status = 422;
        send(res, 422, { error: "failure", value: e.value }, traceId);
      } else if (e instanceof RuntimeFault) {
        status = 422;
        send(res, 422, { error: "runtime_error", code: e.code, message: e.message }, traceId);
      } else {
        status = 500;
        log({ level: "error", trace_id: traceId, msg: String((e as Error)?.stack ?? e) });
        send(res, 500, { error: "internal", message: "内部エラー" }, traceId);
      }
    } finally {
      log({ level: "info", trace_id: traceId, method: req.method, path: url.pathname, status, ms: Math.round(performance.now() - started), sub });
    }
  });
}
