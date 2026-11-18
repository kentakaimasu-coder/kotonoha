// KOTONOHA web runtime: アプリの中のログイン(ADR-0142)
// 外部の認証基盤(OIDC)無しで公開できるように、利用者をアプリの中に登録してパスワードでログインする。
// - パスワードは scrypt(N=2^15, r=8, p=1, 塩 16 バイト)で保管し、比較は時間一定
// - 失敗が続いたら、その人(ログイン名)と接続元をしばらく止める
// - ログインの印は HS256 の JWT(backend と同じ KOTONOHA_JWT_SECRET で署名)を HttpOnly Cookie に入れる。
//   毎回、利用者がまだいるか・印の版(ver)が今の版かを確かめる(役割を変えた・消した・パスワードを変えたら古い印は使えない)
// - 最初の管理者は、起動時に出る一度だけの URL(/setup?t=…)で作る。ほかの人は管理者が作った招待のリンクで登録する
// - 利用者の一覧は JSON のファイル(KOTONOHA_USERS_FILE、既定 ./data/users.json)。書くときは別名で書いてから置き換える
import { createHash, createHmac, randomBytes, scryptSync, timingSafeEqual } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export interface LocalUser {
  id: string;
  /** ログイン名(メールアドレスなど)。小文字にそろえて比べる */
  login: string;
  /** 画面に出す名前 */
  name: string;
  /** scrypt$N$r$p$塩$値 */
  hash: string;
  roles: string[];
  /** 利用者の管理ができる人 */
  admin: boolean;
  /** 印の版。役割・パスワードを変えたら上げる */
  ver: number;
  created: string;
}

interface Invite {
  /** リンクの秘密の SHA-256(秘密そのものは保存しない) */
  hash: string;
  kind: "invite" | "reset";
  roles: string[];
  admin: boolean;
  /** reset のときの相手 */
  user?: string;
  expires: number;
}

interface Store {
  users: LocalUser[];
  invites: Invite[];
}

export interface LocalLoginConfig {
  usersFile: string;
  secret: string;
  /** 最初の管理者を作るための秘密(無ければ起動時に作る) */
  setupToken: string | null;
}

/** KOTONOHA_LOCAL_LOGIN=1 のとき有効 */
export function localLoginFromEnv(env: NodeJS.ProcessEnv): LocalLoginConfig | null {
  if (env.KOTONOHA_LOCAL_LOGIN !== "1") return null;
  const secret = env.KOTONOHA_JWT_SECRET ?? "";
  if (secret.length < 32) throw new Error("アプリの中のログイン(KOTONOHA_LOCAL_LOGIN=1)には KOTONOHA_JWT_SECRET(32 文字以上。本番は 64 文字以上)が要ります");
  return { usersFile: env.KOTONOHA_USERS_FILE || "./data/users.json", secret, setupToken: env.KOTONOHA_SETUP_TOKEN || null };
}

const SCRYPT = { N: 1 << 15, r: 8, p: 1, maxmem: 64 * 1024 * 1024 };
export const MIN_PASSWORD = 10;
const SESSION_SEC = 12 * 3600;
const INVITE_SEC = 7 * 24 * 3600;
const RESET_SEC = 24 * 3600;
/** 同じログイン名で続けて失敗してよい回数。接続元(会社・学校では大勢が同じ)はもっと多く許す */
const MAX_FAILS = 5;
const MAX_FAILS_IP = 30;
const LOCK_MS = 15 * 60 * 1000;

export function hashPassword(pw: string): string {
  const salt = randomBytes(16);
  const v = scryptSync(pw.normalize("NFKC"), salt, 32, SCRYPT);
  return `scrypt$${SCRYPT.N}$${SCRYPT.r}$${SCRYPT.p}$${salt.toString("base64url")}$${v.toString("base64url")}`;
}

export function verifyPassword(pw: string, stored: string): boolean {
  const m = /^scrypt\$(\d+)\$(\d+)\$(\d+)\$([A-Za-z0-9_-]+)\$([A-Za-z0-9_-]+)$/.exec(stored);
  if (!m) return false;
  const expected = Buffer.from(m[5], "base64url");
  const v = scryptSync(pw.normalize("NFKC"), Buffer.from(m[4], "base64url"), expected.length, { N: Number(m[1]), r: Number(m[2]), p: Number(m[3]), maxmem: SCRYPT.maxmem });
  return v.length === expected.length && timingSafeEqual(v, expected);
}

/** 知らないログイン名でも同じだけ時間をかける(いる人といない人を時間で見分けさせない) */
const DUMMY = hashPassword(randomBytes(12).toString("base64url"));

/** パスワードの決まり。だめなら理由を返す */
export function passwordProblem(pw: string, login: string): string | null {
  if (pw.length < MIN_PASSWORD) return `パスワードは ${MIN_PASSWORD} 文字以上にしてください`;
  if (pw.length > 200) return "パスワードは 200 文字以内にしてください";
  if (pw.toLowerCase() === login.toLowerCase()) return "パスワードをログイン名と同じにはできません";
  if (/^(.)\1+$/.test(pw) || /^(0123456789|1234567890|password|qwerty)/i.test(pw)) return "推測されやすいパスワードです。別のものにしてください";
  return null;
}

const sha = (s: string) => createHash("sha256").update(s).digest("base64url");
const b64 = (x: unknown) => Buffer.from(JSON.stringify(x)).toString("base64url");

export class LocalLogin {
  private cfg: LocalLoginConfig;
  private roles: Record<string, string[]>;
  private general: string[];
  private fails = new Map<string, { n: number; until: number }>();
  setupToken: string | null = null;

  constructor(cfg: LocalLoginConfig, roles: Record<string, string[]>, general: string[]) {
    this.cfg = cfg;
    this.roles = roles;
    this.general = general;
    if (this.load().users.length === 0) this.setupToken = cfg.setupToken ?? randomBytes(24).toString("base64url");
  }

  load(): Store {
    if (!existsSync(this.cfg.usersFile)) return { users: [], invites: [] };
    const v = JSON.parse(readFileSync(this.cfg.usersFile, "utf8")) as Store;
    return { users: Array.isArray(v.users) ? v.users : [], invites: Array.isArray(v.invites) ? v.invites.filter((i) => i.expires > Date.now()) : [] };
  }

  private save(s: Store): void {
    mkdirSync(dirname(this.cfg.usersFile), { recursive: true });
    const tmp = `${this.cfg.usersFile}.${process.pid}.tmp`;
    writeFileSync(tmp, JSON.stringify(s, null, 1), { mode: 0o600 });
    renameSync(tmp, this.cfg.usersFile);
  }

  roleNames(): string[] {
    return Object.keys(this.roles);
  }

  /** 失敗が続いた相手か */
  blocked(keys: string[]): boolean {
    const now = Date.now();
    return keys.some((k) => (this.fails.get(k)?.until ?? 0) > now);
  }

  private fail(keys: string[]): void {
    for (const k of keys) {
      const f = this.fails.get(k) ?? { n: 0, until: 0 };
      f.n += 1;
      if (f.n >= (k.startsWith("i:") ? MAX_FAILS_IP : MAX_FAILS)) {
        f.until = Date.now() + LOCK_MS;
        f.n = 0;
      }
      this.fails.set(k, f);
    }
    if (this.fails.size > 10_000) this.fails.clear();
  }

  /** ログイン名とパスワードを確かめる。合えば利用者 */
  authenticate(login: string, pw: string, ip: string): LocalUser | "blocked" | null {
    const key = login.trim().toLowerCase();
    const keys = [`l:${key}`, `i:${ip}`];
    if (this.blocked(keys)) return "blocked";
    const u = this.load().users.find((x) => x.login === key);
    const ok = verifyPassword(pw, u ? u.hash : DUMMY) && u !== undefined;
    if (!ok) {
      this.fail(keys);
      return null;
    }
    this.fails.delete(`l:${key}`);
    return u!;
  }

  /** 利用者のログインの印(backend が検証する JWT)。権限は役割の権限と、役割に関わらない操作の権限 */
  mint(u: LocalUser, now = Math.floor(Date.now() / 1000)): string {
    const caps = new Set<string>(this.general);
    for (const r of u.roles) for (const c of this.roles[r] ?? []) caps.add(c);
    const head = `${b64({ alg: "HS256", typ: "JWT" })}.${b64({ sub: `u:${u.id}`, name: u.name, roles: u.roles, caps: [...caps], ver: u.ver, local: true, iat: now, exp: now + SESSION_SEC })}`;
    return `${head}.${createHmac("sha256", this.cfg.secret).update(head).digest("base64url")}`;
  }

  /** Cookie の印を確かめる。署名・期限・利用者がまだいるか・版が今のものか */
  check(token: string): LocalUser | null {
    const p = token.split(".");
    if (p.length !== 3) return null;
    const expected = createHmac("sha256", this.cfg.secret).update(`${p[0]}.${p[1]}`).digest();
    let sig: Buffer;
    try {
      sig = Buffer.from(p[2], "base64url");
    } catch {
      return null;
    }
    if (sig.length !== expected.length || !timingSafeEqual(sig, expected)) return null;
    let c: { sub?: unknown; ver?: unknown; exp?: unknown; local?: unknown };
    try {
      c = JSON.parse(Buffer.from(p[1], "base64url").toString("utf8"));
    } catch {
      return null;
    }
    if (c.local !== true || typeof c.sub !== "string" || typeof c.exp !== "number" || c.exp < Math.floor(Date.now() / 1000)) return null;
    const u = this.load().users.find((x) => `u:${x.id}` === c.sub);
    return u && u.ver === c.ver ? u : null;
  }

  /** 最初の管理者を作る(利用者がまだいないときだけ) */
  setup(token: string, login: string, name: string, pw: string): LocalUser | string {
    if (this.setupToken === null) return "最初の管理者はもう作られています";
    const a = Buffer.from(sha(token)), b = Buffer.from(sha(this.setupToken));
    if (!timingSafeEqual(a, b)) return "このリンクは使えません(起動したときに出た URL を開いてください)";
    const s = this.load();
    if (s.users.length > 0) return "最初の管理者はもう作られています";
    const u = this.newUser(s, login, name, pw, this.roleNames(), true);
    if (typeof u === "string") return u;
    this.save(s);
    this.setupToken = null;
    return u;
  }

  private newUser(s: Store, login: string, name: string, pw: string, roles: string[], admin: boolean): LocalUser | string {
    const key = login.trim().toLowerCase();
    if (!/^[^\s\u0000-\u001f]{1,100}$/.test(key)) return "ログイン名は 1〜100 文字で、空白を入れずに書いてください(メールアドレスなど)";
    const nm = name.trim() || key;
    if (nm.length > 64 || /[\u0000-\u001f]/.test(nm)) return "名前は 64 文字以内にしてください";
    if (s.users.some((x) => x.login === key)) return "そのログイン名はもう使われています";
    const bad = passwordProblem(pw, key);
    if (bad) return bad;
    const u: LocalUser = { id: randomBytes(9).toString("base64url"), login: key, name: nm, hash: hashPassword(pw), roles: roles.filter((r) => r in this.roles), admin, ver: 1, created: new Date().toISOString() };
    s.users.push(u);
    return u;
  }

  /** 招待・再設定のリンクの秘密を作る(秘密はこの時だけ返す) */
  createLink(kind: "invite" | "reset", roles: string[], admin: boolean, user?: string): string {
    const s = this.load();
    const secret = randomBytes(24).toString("base64url");
    s.invites.push({ hash: sha(secret), kind, roles: roles.filter((r) => r in this.roles), admin, user, expires: Date.now() + (kind === "invite" ? INVITE_SEC : RESET_SEC) * 1000 });
    this.save(s);
    return secret;
  }

  findLink(secret: string): Invite | null {
    if (!/^[A-Za-z0-9_-]{20,64}$/.test(secret)) return null;
    return this.load().invites.find((i) => i.hash === sha(secret)) ?? null;
  }

  /** 招待のリンクで登録する */
  acceptInvite(secret: string, login: string, name: string, pw: string): LocalUser | string {
    const s = this.load();
    const i = s.invites.find((x) => x.hash === sha(secret) && x.kind === "invite");
    if (!i) return "このリンクは使えません(期限切れか、もう使われています)";
    const u = this.newUser(s, login, name, pw, i.roles, i.admin);
    if (typeof u === "string") return u;
    s.invites = s.invites.filter((x) => x !== i);
    this.save(s);
    return u;
  }

  /** 再設定のリンクでパスワードを決め直す */
  acceptReset(secret: string, pw: string): LocalUser | string {
    const s = this.load();
    const i = s.invites.find((x) => x.hash === sha(secret) && x.kind === "reset");
    const u = i && s.users.find((x) => x.id === i.user);
    if (!i || !u) return "このリンクは使えません(期限切れか、もう使われています)";
    const bad = passwordProblem(pw, u.login);
    if (bad) return bad;
    u.hash = hashPassword(pw);
    u.ver += 1;
    s.invites = s.invites.filter((x) => x !== i);
    this.save(s);
    return u;
  }

  /** 自分のパスワードを変える */
  changePassword(id: string, current: string, pw: string): LocalUser | string {
    const s = this.load();
    const u = s.users.find((x) => x.id === id);
    if (!u || !verifyPassword(current, u.hash)) return "今のパスワードが違います";
    const bad = passwordProblem(pw, u.login);
    if (bad) return bad;
    u.hash = hashPassword(pw);
    u.ver += 1;
    this.save(s);
    return u;
  }

  /** 管理者が役割・管理者かどうかを変える(変えたら、その人は次の操作でログインし直す) */
  update(id: string, roles: string[], admin: boolean, by: string): string | null {
    const s = this.load();
    const u = s.users.find((x) => x.id === id);
    if (!u) return "その利用者はいません";
    if (u.id === by && !admin) return "自分の管理者を外すことはできません(ほかの管理者に頼みます)";
    u.roles = roles.filter((r) => r in this.roles);
    u.admin = admin;
    u.ver += 1;
    this.save(s);
    return null;
  }

  remove(id: string, by: string): string | null {
    if (id === by) return "自分は消せません";
    const s = this.load();
    if (!s.users.some((x) => x.id === id)) return "その利用者はいません";
    s.users = s.users.filter((x) => x.id !== id);
    s.invites = s.invites.filter((x) => x.user !== id);
    this.save(s);
    return null;
  }
}
