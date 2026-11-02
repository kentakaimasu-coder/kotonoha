// KOTONOHA backend runtime: 認証アダプタ(ADR-002 提案: Bearer JWT。既定は HS256、秘密は環境変数の Secrets Provider 参照)
// 生成物に秘密情報は含めない(SEC-06 / AT-10)。
import { createHmac, timingSafeEqual } from "node:crypto";
import type { Principal } from "./db.ts";

export interface AuthConfig {
  secret: Buffer | null;
  issuer: string | null;
  audience: string | null;
  /** 時計のずれの許容(秒) */
  leeway: number;
}

export function authConfigFromEnv(env: NodeJS.ProcessEnv): AuthConfig {
  const s = env.KOTONOHA_JWT_SECRET ?? "";
  return {
    secret: s.length >= 32 ? Buffer.from(s, "utf8") : null,
    issuer: env.KOTONOHA_JWT_ISSUER || null,
    audience: env.KOTONOHA_JWT_AUDIENCE || null,
    leeway: 30,
  };
}

function b64url(buf: Buffer): string {
  return buf.toString("base64url");
}

function parseJsonPart(part: string): Record<string, unknown> | null {
  try {
    const v = JSON.parse(Buffer.from(part, "base64url").toString("utf8"));
    return v !== null && typeof v === "object" && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

/** Bearer トークンを検証して主体を返す。不正なら null(呼出側で 401)。 */
export function verifyToken(token: string, cfg: AuthConfig, nowSec: number = Math.floor(Date.now() / 1000)): Principal | null {
  if (cfg.secret === null) return null;
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  const header = parseJsonPart(parts[0]);
  const claims = parseJsonPart(parts[1]);
  if (!header || !claims) return null;
  // アルゴリズムは HS256 のみ受け付ける(alg=none や鍵混同を拒否)
  if (header.alg !== "HS256" || (header.typ !== undefined && header.typ !== "JWT")) return null;
  const expected = createHmac("sha256", cfg.secret).update(`${parts[0]}.${parts[1]}`).digest();
  let given: Buffer;
  try {
    given = Buffer.from(parts[2], "base64url");
  } catch {
    return null;
  }
  if (given.length !== expected.length || !timingSafeEqual(given, expected)) return null;
  if (typeof claims.sub !== "string" || claims.sub.length === 0) return null;
  if (typeof claims.exp !== "number" || claims.exp + cfg.leeway < nowSec) return null;
  if (typeof claims.nbf === "number" && claims.nbf - cfg.leeway > nowSec) return null;
  if (cfg.issuer !== null && claims.iss !== cfg.issuer) return null;
  if (cfg.audience !== null && claims.aud !== cfg.audience) return null;
  const caps = Array.isArray(claims.caps) ? claims.caps.filter((c): c is string => typeof c === "string") : [];
  return { sub: claims.sub, caps: new Set(caps) };
}

/** 開発用のトークン発行(tools/dev-token.ts から使う。サーバーは発行しない)。 */
export function signToken(claims: Record<string, unknown>, secret: string): string {
  const h = b64url(Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const p = b64url(Buffer.from(JSON.stringify(claims)));
  const sig = b64url(createHmac("sha256", secret).update(`${h}.${p}`).digest());
  return `${h}.${p}.${sig}`;
}
