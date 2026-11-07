// KOTONOHA backend runtime: 拡張(外部関数・分析・実験)。ADR-0122 / ADR-0124 / ADR-0125
// - 外部関数の入出力は API と同じワイヤ形式(JSON)。戻り値は宣言した型で検証してから使う(信頼しない)
// - Python は常駐ワーカー(1 ファイル 1 プロセス、JSON 行プロトコル、時間制限つき)で実行する
// - 分析 SQL はコンパイル時に検査済み(SELECT 1 文・宣言した表だけ)。実行時も読み取り専用・行数上限で多重に守る
// - 実験の割付は SHA-256(実験名:対象者) で決定的(コンパイラの kotonoha_ext::assign と同じ計算)
import { createHash } from "node:crypto";
import { spawn, type ChildProcess } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { decodeInput, encode, HttpError, RuntimeFault, type Schema, type Wire } from "./rt.ts";
import { fromSql, q, type Ctx, type Database, type Row } from "./db.ts";

// ---- 外部関数 ----

/** 内部値の引数をワイヤ形式にして外部関数を呼び、戻り値を宣言型で検証して内部値に戻す */
export async function callExternal(
  name: string,
  args: unknown[],
  params: Wire[],
  ret: Wire,
  schema: Schema,
  impl: (wireArgs: unknown[]) => Promise<unknown> | unknown,
): Promise<unknown> {
  const wireArgs = args.map((a, i) => encode(a, params[i], schema));
  let out: unknown;
  try {
    out = await impl(wireArgs);
  } catch (e) {
    if (e instanceof HttpError || e instanceof RuntimeFault) throw e;
    throw new RuntimeFault("external_failed", `外部関数 ${name} が失敗しました`);
  }
  return decodeOutput(out, ret, schema, name);
}

/** 外部関数の戻り値(ワイヤ形式)を検証して内部値にする。一覧・記録も再帰的に検査する */
export function decodeOutput(v: unknown, w: Wire, s: Schema, path: string): unknown {
  try {
    switch (w.kind) {
      case "optional":
        return v === null || v === undefined ? null : decodeOutput(v, w.of, s, path);
      case "list":
        if (!Array.isArray(v)) throw new Error("一覧が必要です");
        return v.map((x, i) => decodeOutput(x, w.of, s, `${path}[${i}]`));
      case "record": {
        if (v === null || typeof v !== "object" || Array.isArray(v)) throw new Error("記録が必要です");
        const o: Record<string, unknown> = {};
        for (const f of w.fields) o[f.name] = decodeOutput((v as Record<string, unknown>)[f.name], f.type, s, `${path}.${f.name}`);
        return o;
      }
      case "unit":
        return undefined;
      default:
        return decodeInput(v, w, s, path);
    }
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    throw new RuntimeFault("external_type", `外部関数の戻り値が宣言した型と違います: ${msg}`);
  }
}

const here = dirname(fileURLToPath(import.meta.url));

/** Python の常駐ワーカー(実装ファイルごとに 1 つ) */
class PyWorker {
  file: string;
  proc: ChildProcess | null = null;
  buf = "";
  seq = 0;
  waiting = new Map<number, { ok: (v: unknown) => void; ng: (e: Error) => void; timer: NodeJS.Timeout }>();
  constructor(file: string) {
    this.file = file;
  }
  start(): ChildProcess {
    if (this.proc && this.proc.exitCode === null) return this.proc;
    const py = process.env.KOTONOHA_PYTHON ?? "python3";
    // -I: 環境変数・ユーザーの site-packages・カレントディレクトリを読み込まない(隔離モード)
    const p = spawn(py, ["-I", join(here, "pyworker.py"), this.file], { stdio: ["pipe", "pipe", "inherit"], env: { PATH: process.env.PATH ?? "", PYTHONIOENCODING: "utf-8" } });
    p.stdout!.setEncoding("utf8");
    p.stdout!.on("data", (d: string) => {
      this.buf += d;
      let i;
      while ((i = this.buf.indexOf("\n")) >= 0) {
        const line = this.buf.slice(0, i);
        this.buf = this.buf.slice(i + 1);
        let m: { id: number; ok?: unknown; error?: string };
        try {
          m = JSON.parse(line);
        } catch {
          continue;
        }
        const w = this.waiting.get(m.id);
        if (!w) continue;
        clearTimeout(w.timer);
        this.waiting.delete(m.id);
        if (m.error !== undefined) w.ng(new Error(m.error));
        else w.ok(m.ok);
      }
    });
    p.on("exit", () => {
      for (const w of this.waiting.values()) {
        clearTimeout(w.timer);
        w.ng(new Error("Python ワーカーが終了しました"));
      }
      this.waiting.clear();
      this.proc = null;
      this.buf = "";
    });
    p.on("error", () => {});
    this.proc = p;
    return p;
  }
  call(fn: string, args: unknown[]): Promise<unknown> {
    const p = this.start();
    const id = ++this.seq;
    const ms = Number(process.env.KOTONOHA_EXT_TIMEOUT_MS ?? 10000);
    return new Promise((ok, ng) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        ng(new Error("時間切れ"));
        // 暴走した処理は止めて次回に作り直す
        p.kill("SIGKILL");
      }, ms);
      this.waiting.set(id, { ok, ng, timer });
      p.stdin!.write(JSON.stringify({ id, fn, args }) + "\n");
    });
  }
}

const workers = new Map<string, PyWorker>();

/** Python の外部関数を呼ぶ(file は backend ルートからの相対パス) */
export function callPython(file: string, fn: string, args: unknown[]): Promise<unknown> {
  const abs = join(here, "..", "..", file);
  let w = workers.get(abs);
  if (!w) {
    w = new PyWorker(abs);
    workers.set(abs, w);
  }
  return w.call(fn, args);
}

export function stopPythonWorkers(): void {
  for (const w of workers.values()) w.proc?.kill();
  workers.clear();
}

// ---- 分析 ----

const ANALYSIS_ROW_MAX = 10_000;

/** クエリ文字列の値をワイヤ形式の値に直す(整数は 10 進の文字列、真偽は true/false) */
export function fromQuery(raw: string | null, w: Wire): unknown {
  if (raw === null) return w.kind === "optional" ? null : undefined;
  const b = w.kind === "optional" ? w.of : w;
  if (b.kind === "int" || b.kind === "money") return /^-?\d{1,16}$/.test(raw) ? Number(raw) : raw;
  if (b.kind === "bool") return raw === "true" ? true : raw === "false" ? false : raw;
  return raw;
}

function cell(v: unknown, w: Wire, col: string): unknown {
  const bad = () => new RuntimeFault("analysis_type", `分析の列 ${col} の値が宣言した型と違います`);
  if (v === null || v === undefined) {
    if (w.kind === "optional") return null;
    throw bad();
  }
  const b = w.kind === "optional" ? w.of : w;
  switch (b.kind) {
    case "int":
    case "money":
    case "datetime":
      if (typeof v === "bigint") return v;
      if (typeof v === "number" && Number.isSafeInteger(v)) return BigInt(v);
      if (typeof v === "string" && /^-?\d+$/.test(v)) return BigInt(v);
      throw bad();
    case "bool":
      return fromSql(v, b);
    case "enum":
    case "string":
    case "id":
    case "url":
    case "email":
      if (typeof v !== "string") throw bad();
      return v;
    default:
      throw bad();
  }
}

/** 分析を読み取り専用で実行し、宣言した列の型で検査して返す */
export async function runAnalysis(ctx: Ctx, db: Database, sql: string, params: unknown[], columns: { name: string; type: Wire }[]): Promise<Row[]> {
  const wrapped = `SELECT * FROM (${sql}) AS ${q("_a")} LIMIT ${ANALYSIS_ROW_MAX + 1}`;
  const rows = await db.readOnly((c) => c.all(wrapped, params));
  if (rows.length > ANALYSIS_ROW_MAX) throw new RuntimeFault("too_many_rows", `分析の結果が ${ANALYSIS_ROW_MAX} 行を超えました。条件を絞ってください`);
  void ctx;
  return rows.map((r) => {
    const o: Row = {};
    for (const c of columns) {
      if (!(c.name in r)) throw new RuntimeFault("analysis_column", `分析の結果に列 ${c.name} がありません(AS ${c.name} で名前を付けてください)`);
      o[c.name] = cell(r[c.name], c.type, c.name);
    }
    return o;
  });
}

// ---- 実験(A/B テスト) ----

export interface Experiment {
  name: string;
  variants: string[];
  weights: number[];
  goal: string;
}

/** 割付: SHA-256(実験名 ":" 対象者) の先頭 8 バイト(ビッグエンディアン)を 100 で割った余りで選ぶ */
export function assignIndex(experiment: string, subject: string, weights: number[]): number {
  const h = createHash("sha256").update(`${experiment}:${subject}`, "utf8").digest();
  const n = Number(h.readBigUInt64BE(0) % 100n);
  let acc = 0;
  for (let i = 0; i < weights.length; i++) {
    acc += weights[i];
    if (n < acc) return i;
  }
  return weights.length - 1;
}

const SUBJECT_RE = /^[A-Za-z0-9_-]{8,128}$/;

/** 対象者: 端末の匿名 ID(X-Koto-Subject または本文の subject)を優先し、無ければ認証済みの主体 ID。
 *  同じ端末では露出と成果が同じ対象者に結び付く(ログイン前に見てログイン後に成果、も数えられる) */
export function subjectOf(ctx: Ctx, body?: unknown): string | null {
  const fromBody = body && typeof body === "object" && !Array.isArray(body) ? (body as Record<string, unknown>).subject : undefined;
  const s = typeof fromBody === "string" ? fromBody : ctx.subject ?? null;
  if (s !== null && s !== undefined) {
    if (!SUBJECT_RE.test(s)) throw new HttpError(400, "invalid_input", "subject は 8〜128 文字の英数字・-・_ です");
    return "a:" + s;
  }
  return ctx.principal ? "u:" + ctx.principal.sub : null;
}

const EXPOSURE = q("_exp_exposure");
const CONVERSION = q("_exp_conversion");

export async function ensureExperimentTables(db: Database): Promise<void> {
  await db.conn.exec(
    `CREATE TABLE IF NOT EXISTS ${EXPOSURE} ("experiment" TEXT NOT NULL, "subject" TEXT NOT NULL, "variant" BIGINT NOT NULL, "at" BIGINT NOT NULL, PRIMARY KEY ("experiment", "subject"))`,
  );
  await db.conn.exec(
    `CREATE TABLE IF NOT EXISTS ${CONVERSION} ("experiment" TEXT NOT NULL, "subject" TEXT NOT NULL, "at" BIGINT NOT NULL, PRIMARY KEY ("experiment", "subject"))`,
  );
}

let tablesReady: Promise<void> | null = null;
function ready(db: Database): Promise<void> {
  tablesReady ??= ensureExperimentTables(db);
  return tablesReady;
}

/** 割り付けて露出を記録する(同じ対象者には常に同じ変種。最初の露出だけ記録) */
export async function assign(ctx: Ctx, x: Experiment, body: unknown): Promise<{ variant: string; index: number }> {
  const subject = subjectOf(ctx, body);
  if (subject === null) throw new HttpError(400, "invalid_input", "対象者がわかりません(ログインするか subject を送ってください)");
  const index = assignIndex(x.name, subject, x.weights);
  await ready(ctx.db);
  await ctx.db.conn.run(`INSERT INTO ${EXPOSURE} ("experiment", "subject", "variant", "at") VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING`, [
    x.name,
    subject,
    BigInt(index),
    BigInt(Date.now()),
  ]);
  return { variant: x.variants[index], index };
}

/** 成果の動作が成功したときに呼ぶ。露出済みの対象者だけ、1 人 1 回数える */
export async function convert(ctx: Ctx, experiments: Experiment[]): Promise<void> {
  if (experiments.length === 0) return;
  let subject: string | null;
  try {
    subject = subjectOf(ctx);
  } catch {
    return;
  }
  if (subject === null) return;
  await ready(ctx.db);
  for (const x of experiments) {
    await ctx.db.conn.run(
      `INSERT INTO ${CONVERSION} ("experiment", "subject", "at") SELECT ?, ?, ? WHERE EXISTS (SELECT 1 FROM ${EXPOSURE} WHERE "experiment" = ? AND "subject" = ?) ON CONFLICT DO NOTHING`,
      [x.name, subject, BigInt(Date.now()), x.name, subject],
    );
  }
}

/** 結果: 変種ごとの露出数・成果数・成果率、対照群との差の z 値と両側 p 値(2 標本の比率の検定) */
export async function results(ctx: Ctx, x: Experiment): Promise<unknown> {
  await ready(ctx.db);
  const exp = await ctx.db.conn.all(`SELECT "variant" AS v, COUNT(*) AS n FROM ${EXPOSURE} WHERE "experiment" = ? GROUP BY "variant"`, [x.name]);
  const conv = await ctx.db.conn.all(
    `SELECT e."variant" AS v, COUNT(*) AS n FROM ${CONVERSION} c JOIN ${EXPOSURE} e ON e."experiment" = c."experiment" AND e."subject" = c."subject" WHERE c."experiment" = ? GROUP BY e."variant"`,
    [x.name],
  );
  const count = (rows: Row[], i: number) => Number(rows.find((r) => Number(r.v) === i)?.n ?? 0);
  const n0 = count(exp, 0);
  const c0 = count(conv, 0);
  const variants = x.variants.map((name, i) => {
    const n = count(exp, i);
    const c = count(conv, i);
    const rate = n === 0 ? 0 : c / n;
    let z: number | null = null;
    let p: number | null = null;
    if (i > 0 && n > 0 && n0 > 0) {
      const pooled = (c + c0) / (n + n0);
      const se = Math.sqrt(pooled * (1 - pooled) * (1 / n + 1 / n0));
      if (se > 0) {
        z = (rate - c0 / n0) / se;
        p = 2 * (1 - normalCdf(Math.abs(z)));
      }
    }
    return { variant: name, control: i === 0, exposures: n, conversions: c, rate, z, p_value: p };
  });
  return { experiment: x.name, goal: x.goal, variants, note: "p_value は 2 標本の比率の z 検定(両側)。途中で何度も見て止めると誤判定が増えます。事前に人数を決めてください" };
}

/** 標準正規分布の累積分布関数(Abramowitz-Stegun 7.1.26 による erf 近似、誤差 1.5e-7 以下) */
export function normalCdf(z: number): number {
  const x = Math.abs(z) / Math.SQRT2;
  const t = 1 / (1 + 0.3275911 * x);
  const y = 1 - ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * Math.exp(-x * x);
  return z >= 0 ? (1 + y) / 2 : (1 - y) / 2;
}
