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
  /** ソースの `役割` 宣言(役割 → 権限)。KOTONOHA_ROLE_MAP が無ければこれを使う(ADR-0126) */
  roles?: Record<string, string[]>;
  /** 重複不可: データ → フィールドの組(ADR-0129) */
  uniques?: Record<string, string[][]>;
  /** 状態の順番: データ → フィールド → 値の順(ADR-0129) */
  sequences?: Record<string, Record<string, { values: string[]; anytime?: string[] }>>;
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

/** 認証失敗(不正なトークン)が続く接続元を一時的に遮断する(トークン総当たり対策: NR-02) */
class AuthFailureGuard {
  fails = new Map<string, { n: number; since: number; blockedUntil: number }>();
  limit: number;
  windowMs: number;
  blockMs: number;
  constructor(limit: number, windowMs: number, blockMs: number) {
    this.limit = limit;
    this.windowMs = windowMs;
    this.blockMs = blockMs;
  }
  blocked(key: string): boolean {
    const f = this.fails.get(key);
    return f !== undefined && f.blockedUntil > Date.now();
  }
  /** 失敗を記録し、遮断に入ったら true */
  fail(key: string): boolean {
    const now = Date.now();
    let f = this.fails.get(key);
    if (!f || now - f.since > this.windowMs) f = { n: 0, since: now, blockedUntil: 0 };
    f.n += 1;
    if (f.n >= this.limit) f.blockedUntil = now + this.blockMs;
    this.fails.set(key, f);
    if (this.fails.size > 10000) this.fails.clear();
    return f.blockedUntil > now;
  }
}

const MAX_BODY = 64 * 1024;

/** API 応答に常に付けるヘッダ(痕跡を出さない・ブラウザに解釈させない: NR-02/NR-03) */
const API_HEADERS: Record<string, string> = {
  "Content-Type": "application/json; charset=utf-8",
  "X-Content-Type-Options": "nosniff",
  "Cache-Control": "no-store",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'",
  "Cross-Origin-Resource-Policy": "same-origin",
  "Referrer-Policy": "no-referrer",
};

function log(entry: Record<string, unknown>): void {
  process.stdout.write(JSON.stringify({ ts: new Date().toISOString(), ...entry }) + "\n");
}

function send(res: ServerResponse, status: number, body: unknown, traceId: string, extra: Record<string, string> = {}): void {
  const text = toJson(body);
  res.writeHead(status, {
    ...extra,
    ...API_HEADERS,
    "Content-Length": Buffer.byteLength(text),
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

/** 認証ヘッダを検証する。ヘッダが無ければ "none"、あるのに不正なら "invalid" */
function principalOf(req: IncomingMessage, cfg: AuthConfig): Principal | "none" | "invalid" {
  const h = req.headers.authorization;
  if (h === undefined || h === "") return "none";
  const m = /^Bearer ([A-Za-z0-9_\-.]{1,8192})$/.exec(h);
  return (m ? verifyToken(m[1], cfg) : null) ?? "invalid";
}

/** 本番モード(KOTONOHA_ENV=production)で安全でない設定なら起動しない */
export function checkProductionConfig(env: NodeJS.ProcessEnv, cfg: AuthConfig): string[] {
  if (env.KOTONOHA_ENV !== "production") return [];
  const problems: string[] = [];
  if (cfg.secret === null && cfg.keys.size === 0 && !env.KOTONOHA_JWKS_URL) problems.push("認証鍵(KOTONOHA_JWKS_URL / _FILE / _JSON または KOTONOHA_JWT_SECRET)がありません");
  if (cfg.secret !== null && (env.KOTONOHA_JWT_SECRET ?? "").length < 64) problems.push("本番の KOTONOHA_JWT_SECRET は 64 文字以上にしてください(OIDC の JWKS を推奨)");
  if (cfg.issuer === null && (cfg.keys.size > 0 || env.KOTONOHA_JWKS_URL)) problems.push("JWKS を使う場合は KOTONOHA_JWT_ISSUER を設定してください");
  if (cfg.audience === null && (cfg.keys.size > 0 || env.KOTONOHA_JWKS_URL)) problems.push("JWKS を使う場合は KOTONOHA_JWT_AUDIENCE を設定してください");
  if ((env.KOTONOHA_RATE_LIMIT_PER_MINUTE ?? "") === "0") problems.push("本番でレート制限を無効にはできません");
  return problems;
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
  if (!env.KOTONOHA_ROLE_MAP && app.roles) cfg.roleMap = app.roles;
  if (env.KOTONOHA_JWKS_URL) startJwksRefresh(cfg, env.KOTONOHA_JWKS_URL);
  if (cfg.secret === null && cfg.keys.size === 0 && !env.KOTONOHA_JWKS_URL)
    log({ level: "warn", msg: "認証鍵(KOTONOHA_JWT_SECRET または JWKS)が未設定のため、認証が必要な API はすべて 401 になります" });
  const problems = checkProductionConfig(env, cfg);
  if (problems.length > 0) throw new Error(`本番モードの設定に問題があります:\n- ${problems.join("\n- ")}`);
  const limiter = new RateLimiter(Number(env.KOTONOHA_RATE_LIMIT_PER_MINUTE ?? 120));
  const guard = new AuthFailureGuard(Number(env.KOTONOHA_AUTH_FAIL_LIMIT ?? 20), 10 * 60 * 1000, 15 * 60 * 1000);
  const audit = (e: Record<string, unknown>) => log({ level: "audit", ...e });
  const server = createServer(async (req, res) => {
    const traceId = /^[A-Za-z0-9-]{8,64}$/.test(String(req.headers["x-request-id"] ?? "")) ? String(req.headers["x-request-id"]) : randomUUID();
    const started = performance.now();
    const url = new URL(req.url ?? "/", "http://localhost");
    let status = 500;
    let sub: string | null = null;
    try {
      // ヘルスチェックは製品名・バージョンを返さない(NR-03)
      if (url.pathname === "/api/health") {
        status = 200;
        return send(res, 200, { ok: true }, traceId);
      }
      const ip = req.socket.remoteAddress ?? "?";
      const m = match(app.routes, req.method ?? "GET", url.pathname);
      if (m === "method") throw new HttpError(405, "method_not_allowed", "メソッドが違います");
      if (m === null) throw new HttpError(404, "not_found", "API がありません");
      const auth = principalOf(req, cfg);
      if (auth === "invalid") {
        if (guard.blocked(ip)) throw new HttpError(429, "too_many_auth_failures", "認証の失敗が続いたため一時的に制限しています");
        if (guard.fail(ip)) audit({ event: "auth_blocked", trace_id: traceId, ip });
        audit({ event: "auth_failed", trace_id: traceId, ip, path: url.pathname });
      }
      const principal = typeof auth === "string" ? null : auth;
      sub = principal?.sub ?? null;
      if (!m.route.access.public) {
        if (principal === null) throw new HttpError(401, "unauthenticated", "認証が必要です");
        const a = m.route.access;
        const granted = a.grant !== undefined && principal.caps.has(a.grant);
        if (!granted) {
          const missing = a.capabilities.filter((c) => !principal.caps.has(c));
          // 許可だけで守る API(資源 Capability が無い)は、許可を持たなければ拒否する
          const denied = missing.length > 0 || (a.capabilities.length === 0 && a.grant !== undefined);
          if (denied) {
            audit({ event: "forbidden", trace_id: traceId, sub, route: m.route.id });
            const need = missing.length > 0 ? `${missing.join(" と ")} が必要です` : "";
            const alt = a.grant ? `(または許可 ${a.grant} があれば、これだけで足ります)` : "";
            throw new HttpError(403, "forbidden", `権限がありません。${need || `許可 ${a.grant} が必要です`}${need ? alt : ""}`);
          }
        }
      }
      if (m.route.method === "POST" && !limiter.allow(principal?.sub ?? ip)) {
        audit({ event: "rate_limited", trace_id: traceId, sub, ip });
        throw new HttpError(429, "rate_limited", "要求が多すぎます。しばらく待ってから再試行してください");
      }
      const body = m.route.method === "POST" ? await readBody(req) : {};
      const subj = req.headers["x-koto-subject"];
      const ctx: Ctx = { db, conn: db.conn, schema: app.schema, owners: app.owners, uniques: app.uniques ?? {}, sequences: app.sequences ?? {}, dirty: null, principal, traceId, subject: typeof subj === "string" ? subj : null };
      const r = await m.route.handle(ctx, { id: m.id, query: url.searchParams, body });
      status = r.status;
      if (m.route.method === "POST") audit({ event: "action", trace_id: traceId, sub, route: m.route.id, status });
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
  // 遅いリクエストで接続を占有させない(slowloris 対策: NR-02)
  server.headersTimeout = 10_000;
  server.requestTimeout = 30_000;
  server.keepAliveTimeout = 5_000;
  server.maxHeadersCount = 100;
  return server;
}
