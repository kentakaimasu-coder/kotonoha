// KOTONOHA backend runtime: 値・演算・符号化(docs/architecture.md §4, §5)
// 生成コードはこのモジュールの関数だけを使って厳密層を実行する。
// 整数・金額・日時(エポックミリ秒)は内部では BigInt(i64 範囲を検査)。

export const I64_MIN = -(2n ** 63n);
export const I64_MAX = 2n ** 63n - 1n;

/** `X を確認 または 失敗(e)` の失敗。HTTP 422 になる。 */
export class Failure extends Error {
  value: unknown;
  constructor(value: unknown) {
    super("KOTONOHA failure");
    this.value = value;
  }
}

/** HTTP エラー(400/401/403/404/413 など)。 */
export class HttpError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

/** 実行時エラー(オーバーフロー・0除算)。HTTP 500 ではなく 422 の `runtime_error` として返す。 */
export class RuntimeFault extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

function checkI64(v: bigint): bigint {
  if (v < I64_MIN || v > I64_MAX) throw new RuntimeFault("overflow", "整数の範囲(64 ビット)を超えました");
  return v;
}

export function add(a: unknown, b: unknown): unknown {
  if (typeof a === "bigint" && typeof b === "bigint") return checkI64(a + b);
  if (typeof a === "string" && typeof b === "string") return a + b;
  throw new TypeError("add: 型検査済みの値ではありません");
}
export function sub(a: unknown, b: unknown): bigint {
  if (typeof a === "bigint" && typeof b === "bigint") return checkI64(a - b);
  throw new TypeError("sub: 型検査済みの値ではありません");
}
export function mul(a: unknown, b: unknown): bigint {
  if (typeof a === "bigint" && typeof b === "bigint") return checkI64(a * b);
  throw new TypeError("mul: 型検査済みの値ではありません");
}
/** 整数除算は 0 方向への切り捨て(BigInt の / と同じ)。0 除算は実行時エラー。 */
export function div(a: unknown, b: unknown): bigint {
  if (typeof a === "bigint" && typeof b === "bigint") {
    if (b === 0n) throw new RuntimeFault("division_by_zero", "0 で割ることはできません");
    return checkI64(a / b);
  }
  throw new TypeError("div: 型検査済みの値ではありません");
}
export function neg(a: unknown): bigint {
  if (typeof a === "bigint") return checkI64(-a);
  throw new TypeError("neg: 型検査済みの値ではありません");
}
export function abs(a: unknown): bigint {
  if (typeof a === "bigint") return checkI64(a < 0n ? -a : a);
  throw new TypeError("abs: 型検査済みの値ではありません");
}
export function max(a: unknown, b: unknown): bigint {
  if (typeof a === "bigint" && typeof b === "bigint") return a > b ? a : b;
  throw new TypeError("max: 型検査済みの値ではありません");
}
export function min(a: unknown, b: unknown): bigint {
  if (typeof a === "bigint" && typeof b === "bigint") return a < b ? a : b;
  throw new TypeError("min: 型検査済みの値ではありません");
}
// ---- 日付・時刻帯(ADR-0129) ----
/** アプリの時刻帯(「今日」・応答の日時の表記・日ごとの集計。既定 Asia/Tokyo) */
export function appTz(): string {
  return process.env.KOTONOHA_APP_TZ || "Asia/Tokyo";
}

/** 時刻 t(ミリ秒)における時刻帯 tz の UTC からのずれ(ミリ秒) */
export function tzOffsetMs(t: number, tz: string): number {
  const parts = new Intl.DateTimeFormat("en-US", { timeZone: tz, hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" }).formatToParts(new Date(t));
  const n = (k: string) => Number(parts.find((p) => p.type === k)!.value);
  return Date.UTC(n("year"), n("month") - 1, n("day"), n("hour"), n("minute"), n("second")) - Math.floor(t / 1000) * 1000;
}

const ymd = (d: Date) => `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;

/** 時刻 t をアプリの時刻帯での暦日に("2026-12-05") */
export function localDate(t: number, tz: string = appTz()): string {
  return ymd(new Date(t + tzOffsetMs(t, tz)));
}

/** 時刻帯 tz での今日の暦日と、その始まり・終わり(ミリ秒) */
export function todayIn(tz: string, now: number = Date.now()): { date: string; start: number; end: number } {
  const date = localDate(now, tz);
  const midnightLocal = Date.parse(date + "T00:00:00Z");
  const start = midnightLocal - tzOffsetMs(midnightLocal, tz);
  const nextLocal = midnightLocal + 86_400_000;
  return { date, start, end: nextLocal - tzOffsetMs(nextLocal, tz) };
}

/** 組込み `今日()` */
export function today(): string {
  return todayIn(appTz()).date;
}

/** 組込み `日後` / `日前`: 日時は 24 時間 × N、日付は暦日で N 日 */
export function addDays(v: unknown, n: unknown): unknown {
  const days = BigInt(n as bigint);
  if (typeof v === "bigint") return v + days * 86_400_000n;
  const t = Date.parse((v as string) + "T00:00:00Z") + Number(days) * 86_400_000;
  if (Number.isNaN(t)) throw new RuntimeFault("invalid_date", "日付を計算できません");
  return ymd(new Date(t));
}

/** 組込み `日数差(a, b)`: b − a の日数(アプリの時刻帯の暦日で数える) */
export function daysBetween(a: unknown, b: unknown): bigint {
  const d = (v: unknown) => (typeof v === "bigint" ? localDate(Number(v)) : (v as string));
  return BigInt(Math.round((Date.parse(d(b) + "T00:00:00Z") - Date.parse(d(a) + "T00:00:00Z")) / 86_400_000));
}

/** 応答の日時: アプリの時刻帯のずれ付き ISO 8601("2026-12-05T19:00:00.000+09:00") */
export function isoInTz(t: number, tz: string = appTz()): string {
  const off = tzOffsetMs(t, tz);
  const local = new Date(t + off).toISOString().slice(0, -1);
  if (off === 0) return local + "Z";
  const m = Math.abs(off) / 60000;
  return `${local}${off < 0 ? "-" : "+"}${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

export function eq(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => eq(x, b[i]));
  const ka = Object.keys(a as object), kb = Object.keys(b as object);
  return ka.length === kb.length && ka.every((k) => eq((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

// ---- ワイヤ形式(全クライアント共通) ----
export type Wire =
  | { kind: "bool" | "int" | "string" | "id" | "url" | "email" | "datetime" | "date" | "unit" }
  | { kind: "money"; currency: string; minor_units: number }
  | { kind: "enum"; name: string }
  | { kind: "entity"; name: string }
  | { kind: "optional"; of: Wire }
  | { kind: "list"; of: Wire }
  | { kind: "record"; fields: { name: string; type: Wire }[] }
  | { kind: "result"; ok: Wire; err: Wire };

export interface Schema {
  entities: Record<string, { name: string; type: Wire }[]>;
  enums: Record<string, string[]>;
}

const SAFE = BigInt(Number.MAX_SAFE_INTEGER);

/** 内部値 → JSON 値 */
export function encode(v: unknown, w: Wire, s: Schema): unknown {
  switch (w.kind) {
    case "optional":
      return v === null || v === undefined ? null : encode(v, w.of, s);
    case "int":
    case "money": {
      const b = v as bigint;
      // JSON 数値として出す。JS クライアントで精度が落ちる範囲は文字列にせずエラーにする(既知の制約 K-03)
      if (b > SAFE || b < -SAFE) throw new RuntimeFault("unsafe_integer", "JSON で安全に表せない整数です(±2^53 超)");
      return Number(b);
    }
    case "datetime":
      // アプリの時刻帯で表す(画面と同じ日本時間で読める。ずれ付きなので時刻そのものは同じ)
      return isoInTz(Number(v as bigint));
    case "date":
      return v;
    case "bool":
    case "string":
    case "id":
    case "url":
    case "email":
    case "enum":
      return v;
    case "list":
      return (v as unknown[]).map((x) => encode(x, w.of, s));
    case "record": {
      const o: Record<string, unknown> = {};
      for (const f of w.fields) o[f.name] = encode((v as Record<string, unknown>)[f.name], f.type, s);
      return o;
    }
    case "entity": {
      const o: Record<string, unknown> = {};
      for (const f of s.entities[w.name]) o[f.name] = encode((v as Record<string, unknown>)[f.name], f.type, s);
      return o;
    }
    case "unit":
      return null;
    case "result":
      throw new TypeError("result は応答の外側で扱う");
  }
}

const ID_MAX = 256;
const STR_MAX = 10_000;
const EMAIL_RE = /^[^\s@]+@[^\s@]+$/;

/** JSON 入力 → 内部値(検証付き)。不正なら HTTP 400。 */
export function decodeInput(v: unknown, w: Wire, s: Schema, path: string): unknown {
  const bad = (msg: string) => new HttpError(400, "invalid_input", `${path}: ${msg}`);
  switch (w.kind) {
    case "optional":
      return v === null || v === undefined ? null : decodeInput(v, w.of, s, path);
    case "int":
    case "money":
      if (typeof v !== "number" || !Number.isSafeInteger(v)) throw bad("整数が必要です");
      return BigInt(v);
    case "bool":
      if (typeof v !== "boolean") throw bad("真偽値が必要です");
      return v;
    case "id":
      if (typeof v !== "string" || v.length === 0 || v.length > ID_MAX) throw bad(`1〜${ID_MAX} 文字の識別子が必要です`);
      return v;
    case "string":
      if (typeof v !== "string" || v.length > STR_MAX) throw bad(`${STR_MAX} 文字以内の文字列が必要です`);
      return v;
    case "url":
      if (typeof v !== "string" || !URL.canParse(v) || !/^https?:/i.test(v)) throw bad("http(s) の URL が必要です");
      return v;
    case "email":
      if (typeof v !== "string" || !EMAIL_RE.test(v) || v.length > 320) throw bad("メールアドレスが必要です");
      return v;
    case "enum":
      if (typeof v !== "string" || !s.enums[w.name].includes(v)) throw bad(`${s.enums[w.name].join(" / ")} のいずれかが必要です`);
      return v;
    case "date": {
      // 暦日 "2026-12-05"(全角数字・/ 区切りも受け付ける: NR-01)
      const t = typeof v === "string" ? v.normalize("NFKC").trim().replace(/\//g, "-") : "";
      const m = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(t);
      if (!m) throw bad("日付(例: 2026-12-05)が必要です");
      const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
      const dt = new Date(Date.UTC(y, mo - 1, d));
      if (dt.getUTCFullYear() !== y || dt.getUTCMonth() !== mo - 1 || dt.getUTCDate() !== d) throw bad("存在しない日付です");
      return `${m[1]}-${String(mo).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
    }
    case "datetime": {
      if (typeof v !== "string" || !/(Z|[+-]\d\d:\d\d)$/.test(v)) throw bad("タイムゾーン付きの ISO 8601 日時が必要です");
      const t = Date.parse(v);
      if (Number.isNaN(t)) throw bad("日時として解釈できません");
      return BigInt(t);
    }
    default:
      throw bad(`型 ${w.kind} は入力にできません`);
  }
}

/** JSON 文字列化(BigInt を含まないことを前提に、未エンコード値を検出する) */
export function toJson(v: unknown): string {
  return JSON.stringify(v, (_k, x) => {
    if (typeof x === "bigint") throw new TypeError("未エンコードの BigInt");
    return x;
  });
}
