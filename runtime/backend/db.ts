// KOTONOHA backend runtime: SQLite アクセス・取引・データアダプタ(ADR-003, ADR-0104, ADR-0110)
// SQL は常にプレースホルダとバインド変数(SEC-04)。識別子は宣言名を二重引用符で囲む。
import { DatabaseSync } from "node:sqlite";
import { randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { HttpError, type Schema, type Wire } from "./rt.ts";

const META = Symbol("kotonoha.entity");

interface EntityMeta {
  table: string;
  original: Record<string, unknown>;
}

export interface Ctx {
  db: DatabaseSync;
  schema: Schema;
  /** 取引中に更新されたエンティティ */
  dirty: Set<Record<string, unknown>> | null;
  principal: Principal | null;
  traceId: string;
}

export interface Principal {
  sub: string;
  caps: Set<string>;
}

export function q(ident: string): string {
  if (ident.length === 0 || ident.includes('"') || /[\u0000-\u001f]/.test(ident)) throw new Error(`unsafe identifier ${ident}`);
  return `"${ident}"`;
}

export function openDb(path: string, ddl: string[]): DatabaseSync {
  if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
  const db = new DatabaseSync(path);
  db.exec("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;");
  for (const stmt of ddl) db.exec(stmt);
  return db;
}

function baseWire(w: Wire): Wire {
  return w.kind === "optional" ? w.of : w;
}

/** 内部値 → SQLite 値 */
function toSql(v: unknown, w: Wire): unknown {
  if (v === null || v === undefined) return null;
  const b = baseWire(w);
  if (b.kind === "bool") return v ? 1n : 0n;
  return v;
}

/** SQLite 値 → 内部値 */
function fromSql(v: unknown, w: Wire): unknown {
  if (v === null || v === undefined) return null;
  const b = baseWire(w);
  if (b.kind === "bool") return v !== 0n && v !== 0;
  if (b.kind === "int" || b.kind === "money" || b.kind === "datetime") return typeof v === "bigint" ? v : BigInt(v as number);
  return v;
}

export function decodeRow(ctx: Ctx, table: string, row: Record<string, unknown>): Record<string, unknown> {
  const fields = ctx.schema.entities[table];
  const e: Record<string, unknown> = {};
  for (const f of fields) e[f.name] = fromSql(row[f.name], f.type);
  Object.defineProperty(e, META, { value: { table, original: { ...e } } satisfies EntityMeta, enumerable: false });
  return e;
}

export function all(ctx: Ctx, sql: string, params: unknown[]): Record<string, unknown>[] {
  const st = ctx.db.prepare(sql);
  st.setReadBigInts(true);
  return st.all(...(params as never[])) as Record<string, unknown>[];
}

// ---- 取引 ----
/** `取引:` ブロック。本体が値 {v} を返したらそれを返す。失敗・例外はロールバックして再送出。 */
export function tx<T>(ctx: Ctx, body: () => T): T {
  if (ctx.dirty !== null) throw new Error("入れ子の取引");
  ctx.dirty = new Set();
  ctx.db.exec("BEGIN IMMEDIATE");
  try {
    const r = body();
    flush(ctx);
    ctx.db.exec("COMMIT");
    return r;
  } catch (e) {
    ctx.db.exec("ROLLBACK");
    throw e;
  } finally {
    ctx.dirty = null;
  }
}

/** エンティティのフィールド更新を記録する(取引の終わりに永続化)。 */
export function markDirty(ctx: Ctx, e: Record<string, unknown>): void {
  if (ctx.dirty === null) throw new Error("取引外の更新(型検査で禁止されているはず)");
  if (!(META in e)) throw new Error("保存元不明のエンティティ");
  ctx.dirty.add(e);
}

function flush(ctx: Ctx): void {
  for (const e of ctx.dirty ?? []) {
    const meta = (e as Record<symbol, EntityMeta>)[META];
    const fields = ctx.schema.entities[meta.table];
    const changed = fields.filter((f) => f.name !== "ID" && e[f.name] !== meta.original[f.name]);
    if (changed.length === 0) continue;
    const sets = changed.map((f) => `${q(f.name)} = ?`).join(", ");
    const params = changed.map((f) => toSql(e[f.name], f.type));
    ctx.db.prepare(`UPDATE ${q(meta.table)} SET ${sets} WHERE "ID" = ?`).run(...(params as never[]), e["ID"] as never);
    meta.original = { ...e };
  }
}

// ---- データアダプタ(操作種別ごと) ----
export function adGet(ctx: Ctx, table: string, id: unknown, optional: boolean): Record<string, unknown> | null {
  const rows = all(ctx, `SELECT * FROM ${q(table)} WHERE "ID" = ?`, [id]);
  if (rows.length === 0) {
    if (optional) return null;
    throw new HttpError(404, "not_found", `${table} が見つかりません`);
  }
  return decodeRow(ctx, table, rows[0]);
}

export function adList(ctx: Ctx, table: string): Record<string, unknown>[] {
  return all(ctx, `SELECT * FROM ${q(table)} ORDER BY "ID"`, []).map((r) => decodeRow(ctx, table, r));
}

export function adCreate(ctx: Ctx, table: string, values: Record<string, unknown>, returns: "id" | "entity"): unknown {
  const fields = ctx.schema.entities[table];
  const idField = fields.find((f) => f.name === "ID");
  const row: Record<string, unknown> = { ...values };
  if (idField) row["ID"] = randomUUID();
  const cols = fields.filter((f) => f.name in row);
  const sql = `INSERT INTO ${q(table)} (${cols.map((f) => q(f.name)).join(", ")}) VALUES (${cols.map(() => "?").join(", ")})`;
  ctx.db.prepare(sql).run(...(cols.map((f) => toSql(row[f.name], f.type)) as never[]));
  if (returns === "id") return row["ID"];
  return adGet(ctx, table, row["ID"], false);
}

export function adUpdate(ctx: Ctx, table: string, values: Record<string, unknown>): Record<string, unknown> {
  const fields = ctx.schema.entities[table].filter((f) => f.name !== "ID" && f.name in values);
  if (fields.length > 0) {
    const sql = `UPDATE ${q(table)} SET ${fields.map((f) => `${q(f.name)} = ?`).join(", ")} WHERE "ID" = ?`;
    const r = ctx.db.prepare(sql).run(...(fields.map((f) => toSql(values[f.name], f.type)) as never[]), values["ID"] as never);
    if (Number(r.changes) === 0) throw new HttpError(404, "not_found", `${table} が見つかりません`);
  }
  return adGet(ctx, table, values["ID"], false) as Record<string, unknown>;
}

export function adDelete(ctx: Ctx, table: string, id: unknown): boolean {
  const r = ctx.db.prepare(`DELETE FROM ${q(table)} WHERE "ID" = ?`).run(id as never);
  return Number(r.changes) > 0;
}
