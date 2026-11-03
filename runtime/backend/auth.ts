// KOTONOHA backend runtime: 認証アダプタ(ADR-002)
// - HS256: 共有秘密 KOTONOHA_JWT_SECRET(32 文字以上)
// - RS256 / ES256: OIDC の JWKS(KOTONOHA_JWKS_FILE / KOTONOHA_JWKS_JSON / KOTONOHA_JWKS_URL)
// - 主体の Capability: claim `caps`(または KOTONOHA_CAPS_CLAIM)と、ロール claim(KOTONOHA_ROLES_CLAIM、既定 roles)を
//   KOTONOHA_ROLE_MAP({"役割": ["商品.read", "動作.注文する"]})で展開したものの和
// 生成物に秘密情報は含めない(SEC-06 / AT-10)。
import { createHmac, createPublicKey, timingSafeEqual, verify as cryptoVerify, type KeyObject } from "node:crypto";
import { readFileSync } from "node:fs";
import type { Principal } from "./db.ts";

interface Jwk {
  kid?: string;
  kty: string;
  alg?: string;
  use?: string;
  crv?: string;
  [k: string]: unknown;
}

export interface AuthConfig {
  secret: Buffer | null;
  /** kid → 公開鍵(RS256/ES256) */
  keys: Map<string, { key: KeyObject; alg: "RS256" | "ES256" }>;
  issuer: string | null;
  audience: string | null;
  capsClaim: string;
  rolesClaim: string;
  roleMap: Record<string, string[]>;
  /** 時計のずれの許容(秒) */
  leeway: number;
}

function loadJwks(cfg: AuthConfig, jwks: { keys?: Jwk[] }): void {
  const next = new Map<string, { key: KeyObject; alg: "RS256" | "ES256" }>();
  for (const [i, k] of (jwks.keys ?? []).entries()) {
    if (k.use !== undefined && k.use !== "sig") continue;
    let alg: "RS256" | "ES256";
    if (k.kty === "RSA" && (k.alg === undefined || k.alg === "RS256")) alg = "RS256";
    else if (k.kty === "EC" && k.crv === "P-256" && (k.alg === undefined || k.alg === "ES256")) alg = "ES256";
    else continue;
    try {
      const key = createPublicKey({ key: k as never, format: "jwk" });
      next.set(k.kid ?? `#${i}`, { key, alg });
    } catch {
      // 不正な鍵は無視する
    }
  }
  cfg.keys = next;
}

export function authConfigFromEnv(env: NodeJS.ProcessEnv): AuthConfig {
  const s = env.KOTONOHA_JWT_SECRET ?? "";
  let roleMap: Record<string, string[]> = {};
  if (env.KOTONOHA_ROLE_MAP) {
    const v = JSON.parse(env.KOTONOHA_ROLE_MAP);
    if (v === null || typeof v !== "object" || Array.isArray(v)) throw new Error("KOTONOHA_ROLE_MAP は {役割: [Capability]} の JSON です");
    roleMap = v;
  }
  const cfg: AuthConfig = {
    secret: s.length >= 32 ? Buffer.from(s, "utf8") : null,
    keys: new Map(),
    issuer: env.KOTONOHA_JWT_ISSUER || null,
    audience: env.KOTONOHA_JWT_AUDIENCE || null,
    capsClaim: env.KOTONOHA_CAPS_CLAIM || "caps",
    rolesClaim: env.KOTONOHA_ROLES_CLAIM || "roles",
    roleMap,
    leeway: 30,
  };
  if (env.KOTONOHA_JWKS_JSON) loadJwks(cfg, JSON.parse(env.KOTONOHA_JWKS_JSON));
  else if (env.KOTONOHA_JWKS_FILE) loadJwks(cfg, JSON.parse(readFileSync(env.KOTONOHA_JWKS_FILE, "utf8")));
  return cfg;
}

/** JWKS URL を定期取得する(IdP の鍵ローテーション対応)。HTTPS 以外は localhost のみ許可。 */
export function startJwksRefresh(cfg: AuthConfig, url: string, intervalMs = 10 * 60 * 1000): () => void {
  const u = new URL(url);
  if (u.protocol !== "https:" && !["127.0.0.1", "localhost", "[::1]"].includes(u.hostname)) throw new Error("KOTONOHA_JWKS_URL は https です");
  const load = async () => {
    try {
      const r = await fetch(u, { signal: AbortSignal.timeout(10_000) });
      if (r.ok) loadJwks(cfg, (await r.json()) as { keys?: Jwk[] });
    } catch {
      // 前回の鍵を使い続ける
    }
  };
  void load();
  const t = setInterval(load, intervalMs);
  t.unref();
  return () => clearInterval(t);
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

function verifySignature(alg: unknown, kid: unknown, signingInput: string, sig: Buffer, cfg: AuthConfig): boolean {
  if (alg === "HS256") {
    if (cfg.secret === null) return false;
    const expected = createHmac("sha256", cfg.secret).update(signingInput).digest();
    return sig.length === expected.length && timingSafeEqual(sig, expected);
  }
  if (alg === "RS256" || alg === "ES256") {
    const entry = typeof kid === "string" ? cfg.keys.get(kid) : cfg.keys.size === 1 ? [...cfg.keys.values()][0] : undefined;
    // 鍵と alg の組み合わせが一致しなければ拒否(アルゴリズム混同攻撃の防止)
    if (!entry || entry.alg !== alg) return false;
    if (alg === "RS256") return cryptoVerify("sha256", Buffer.from(signingInput), entry.key, sig);
    return cryptoVerify("sha256", Buffer.from(signingInput), { key: entry.key, dsaEncoding: "ieee-p1363" }, sig);
  }
  // alg=none など
  return false;
}

/** Bearer トークンを検証して主体を返す。不正なら null(呼出側で 401)。 */
export function verifyToken(token: string, cfg: AuthConfig, nowSec: number = Math.floor(Date.now() / 1000)): Principal | null {
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  const header = parseJsonPart(parts[0]);
  const claims = parseJsonPart(parts[1]);
  if (!header || !claims) return null;
  if (header.typ !== undefined && header.typ !== "JWT" && header.typ !== "at+jwt") return null;
  if (header.crit !== undefined) return null;
  let sig: Buffer;
  try {
    sig = Buffer.from(parts[2], "base64url");
  } catch {
    return null;
  }
  if (!verifySignature(header.alg, header.kid, `${parts[0]}.${parts[1]}`, sig, cfg)) return null;
  if (typeof claims.sub !== "string" || claims.sub.length === 0 || claims.sub.length > 256) return null;
  if (typeof claims.exp !== "number" || claims.exp + cfg.leeway < nowSec) return null;
  if (typeof claims.nbf === "number" && claims.nbf - cfg.leeway > nowSec) return null;
  if (cfg.issuer !== null && claims.iss !== cfg.issuer) return null;
  if (cfg.audience !== null) {
    const aud = claims.aud;
    if (!(aud === cfg.audience || (Array.isArray(aud) && aud.includes(cfg.audience)))) return null;
  }
  const caps = new Set<string>();
  const direct = claims[cfg.capsClaim];
  const list = Array.isArray(direct) ? direct : typeof direct === "string" ? direct.split(" ") : [];
  for (const c of list) if (typeof c === "string" && c.length > 0) caps.add(c);
  const roles = claims[cfg.rolesClaim];
  if (Array.isArray(roles)) for (const r of roles) if (typeof r === "string") for (const c of cfg.roleMap[r] ?? []) caps.add(c);
  return { sub: claims.sub, caps };
}

/** 開発用のトークン発行(tools/dev-token.ts から使う。サーバーは発行しない)。 */
export function signToken(claims: Record<string, unknown>, secret: string): string {
  const h = b64url(Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const p = b64url(Buffer.from(JSON.stringify(claims)));
  const sig = b64url(createHmac("sha256", secret).update(`${h}.${p}`).digest());
  return `${h}.${p}.${sig}`;
}
