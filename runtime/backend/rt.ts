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
export function eq(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => eq(x, b[i]));
  const ka = Object.keys(a as object), kb = Object.keys(b as object);
  return ka.length === kb.length && ka.every((k) => eq((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

// ---- ワイヤ形式(全クライアント共通) ----
export type Wire =
  | { kind: "bool" | "int" | "string" | "id" | "url" | "email" | "datetime" | "unit" }
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
      return new Date(Number(v as bigint)).toISOString();
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
