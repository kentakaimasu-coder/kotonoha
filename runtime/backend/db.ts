// KOTONOHA backend runtime: DB アクセス・取引・データアダプタ(ADR-003, ADR-0104, ADR-0110, ADR-0118)
// - 方言: SQLite(node:sqlite、既定)/ PostgreSQL(KOTONOHA_DB=postgres://…、npm の pg を任意依存として使う)
// - SQL は常にプレースホルダとバインド変数(SEC-04)。識別子は宣言名を二重引用符で囲む
// - 整数・金額・日時(エポックミリ秒)・真偽(0/1)はどちらの方言でも 64 ビット整数列に格納する
import { createHash, randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { HttpError, appTz, todayIn, tzOffsetMs, type Schema, type Wire } from "./rt.ts";

const META = Symbol("kotonoha.entity");

interface EntityMeta {
  table: string;
  original: Record<string, unknown>;
}

export type Row = Record<string, unknown>;
export type Dialect = "sqlite" | "postgres";

/** 1 本の接続(取引中は取引用の接続) */
export interface Conn {
  all(sql: string, params: unknown[]): Promise<Row[]>;
  /** 変更行数を返す */
  run(sql: string, params: unknown[]): Promise<number>;
  exec(sql: string): Promise<void>;
}

export interface Database {
  dialect: Dialect;
  /** 取引外で使う接続 */
  conn: Conn;
  /** 本体を取引の中で実行する。失敗(例外)ならロールバックして再送出 */
  transaction<T>(body: (c: Conn) => Promise<T>): Promise<T>;
  /** 読み取り専用で実行する(分析 SQL の多重防御: ADR-0125) */
  readOnly<T>(body: (c: Conn) => Promise<T>): Promise<T>;
  close(): Promise<void>;
}

export interface Ctx {
  db: Database;
  /** 現在の接続(取引中は取引用) */
  conn: Conn;
  schema: Schema;
  /** 行レベル認可: データ → 本人の主体 ID を入れるフィールド */
  owners: Record<string, string>;
  /** 重複不可: データ → フィールドの組(ADR-0129) */
  uniques?: Record<string, string[][]>;
  /** 状態の順番: データ → フィールド → 値の順(ADR-0129) */
  sequences?: Record<string, Record<string, { values: string[]; anytime?: string[] }>>;
  /** 取引中に更新されたエンティティ */
  dirty: Set<Row> | null;
  principal: Principal | null;
  traceId: string;
  /** 実験の対象者(未認証の利用者の匿名 ID。X-Koto-Subject。ADR-0122) */
  subject?: string | null;
}

export interface Principal {
  sub: string;
  caps: Set<string>;
}

export function q(ident: string): string {
  if (ident.length === 0 || ident.includes('"') || /[\u0000-\u001f]/.test(ident)) throw new Error(`unsafe identifier ${ident}`);
  return `"${ident}"`;
}

/** `?` / `?n` プレースホルダを PostgreSQL の `$n` に変換する(単引用符の中は変換しない) */
export function toPgParams(sql: string): string {
  let n = 0;
  let inStr = false;
  let out = "";
  for (let i = 0; i < sql.length; i++) {
    const ch = sql[i];
    if (ch === "'") inStr = !inStr;
    if (!inStr && ch === "?") {
      let j = i + 1;
      while (j < sql.length && sql[j] >= "0" && sql[j] <= "9") j++;
      out += j > i + 1 ? `$${sql.slice(i + 1, j)}` : `$${++n}`;
      i = j - 1;
      continue;
    }
    out += ch;
  }
  return out;
}

/**
 * `?N`(番号付き)を `?` に置き換え、引数を出現順に並べ直す。
 * Node 22.18 の node:sqlite は番号付きプレースホルダに配列で値を渡すと
 * "column index out of range" になるため、SQLite には常に位置指定の `?` で渡す。
 */
export function toPositional(sql: string, params: unknown[]): { sql: string; params: unknown[] } {
  if (!/\?\d/.test(sql)) return { sql, params };
  const out: unknown[] = [];
  let inStr = false;
  let res = "";
  for (let i = 0; i < sql.length; i++) {
    const ch = sql[i];
    if (ch === "'") inStr = !inStr;
    if (!inStr && ch === "?") {
      let j = i + 1;
      while (j < sql.length && sql[j] >= "0" && sql[j] <= "9") j++;
      if (j > i + 1) {
        const n = Number(sql.slice(i + 1, j));
        if (n < 1 || n > params.length) throw new Error(`placeholder ?${n} out of range`);
        out.push(params[n - 1]);
        res += "?";
        i = j - 1;
        continue;
      }
      throw new Error("番号付きと位置指定のプレースホルダは混在できません");
    }
    res += ch;
  }
  return { sql: res, params: out };
}

/** 非同期の排他ロック(SQLite の 1 接続を取引中に他の要求と共有しないため) */
class Mutex {
  tail: Promise<void> = Promise.resolve();
  async run<T>(f: () => Promise<T>): Promise<T> {
    let release!: () => void;
    const prev = this.tail;
    this.tail = new Promise((r) => (release = r));
    await prev;
    try {
      return await f();
    } finally {
      release();
    }
  }
}

async function openSqlite(path: string): Promise<Database> {
  const { DatabaseSync } = await import("node:sqlite");
  if (path !== ":memory:") mkdirSync(dirname(path), { recursive: true });
  const raw = new DatabaseSync(path);
  raw.exec("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;");
  const lock = new Mutex();
  const direct: Conn = {
    async all(sql, params) {
      const p = toPositional(sql, params);
      const st = raw.prepare(p.sql);
      st.setReadBigInts(true);
      return st.all(...(p.params as never[])) as Row[];
    },
    async run(sql, params) {
      const p = toPositional(sql, params);
      return Number(raw.prepare(p.sql).run(...(p.params as never[])).changes);
    },
    async exec(sql) {
      raw.exec(sql);
    },
  };
  // 取引外の操作もロックを取る(取引中の接続に割り込ませない)
  const locked: Conn = {
    all: (s, p) => lock.run(() => direct.all(s, p)),
    run: (s, p) => lock.run(() => direct.run(s, p)),
    exec: (s) => lock.run(() => direct.exec(s)),
  };
  return {
    dialect: "sqlite",
    conn: locked,
    transaction: (body) =>
      lock.run(async () => {
        raw.exec("BEGIN IMMEDIATE");
        try {
          const r = await body(direct);
          raw.exec("COMMIT");
          return r;
        } catch (e) {
          raw.exec("ROLLBACK");
          throw e;
        }
      }),
    readOnly: (body) =>
      lock.run(async () => {
        raw.exec("PRAGMA query_only = ON");
        try {
          return await body(direct);
        } finally {
          raw.exec("PRAGMA query_only = OFF");
        }
      }),
    close: async () => raw.close(),
  };
}

async function openPostgres(url: string): Promise<Database> {
  let pg: any;
  try {
    pg = (await import("pg" as string)).default;
  } catch {
    throw new Error("PostgreSQL を使うには npm install pg(package.json の optionalDependencies)が必要です");
  }
  // BIGINT(20)・NUMERIC(1700: SUM の結果)は BigInt で受け取る
  pg.types.setTypeParser(20, (v: string) => BigInt(v));
  pg.types.setTypeParser(1700, (v: string) => (/^-?\d+$/.test(v) ? BigInt(v) : v));
  const pool = new pg.Pool({ connectionString: url, max: Number(process.env.KOTONOHA_PG_POOL ?? 10) });
  const wrap = (c: any): Conn => ({
    async all(sql, params) {
      return (await c.query(toPgParams(sql), params)).rows;
    },
    async run(sql, params) {
      return Number((await c.query(toPgParams(sql), params)).rowCount ?? 0);
    },
    async exec(sql) {
      await c.query(sql);
    },
  });
  return {
    dialect: "postgres",
    conn: wrap(pool),
    async transaction(body) {
      // SERIALIZABLE で「読んでから更新」の取りこぼしを防ぎ、直列化失敗(40001)・デッドロック(40P01)は再試行する
      for (let attempt = 0; ; attempt++) {
        const client = await pool.connect();
        try {
          await client.query("BEGIN ISOLATION LEVEL SERIALIZABLE");
          const r = await body(wrap(client));
          await client.query("COMMIT");
          return r;
        } catch (e: any) {
          await client.query("ROLLBACK").catch(() => {});
          if ((e?.code === "40001" || e?.code === "40P01") && attempt < 8) {
            await new Promise((r) => setTimeout(r, 5 * 2 ** attempt * Math.random()));
            continue;
          }
          throw e;
        } finally {
          client.release();
        }
      }
    },
    async readOnly(body) {
      const client = await pool.connect();
      try {
        await client.query("BEGIN TRANSACTION READ ONLY");
        await client.query(`SET LOCAL statement_timeout = ${Math.max(100, Number(process.env.KOTONOHA_ANALYSIS_TIMEOUT_MS ?? 10000))}`);
        const r = await body(wrap(client));
        await client.query("COMMIT");
        return r;
      } catch (e) {
        await client.query("ROLLBACK").catch(() => {});
        throw e;
      } finally {
        client.release();
      }
    },
    close: () => pool.end(),
  };
}

// ---- スキーマ照合(差分マイグレーション) ----
export interface ColumnDef {
  name: string;
  sql_type: string;
  pg_type: string;
  notnull: boolean;
  pk: boolean;
  def: string;
  pg_def: string;
}

export interface TableDef {
  name: string;
  ddl: string;
  pg_ddl: string;
  columns: ColumnDef[];
  /** 重複不可のフィールドの組と、判定から外す行の条件(約束 C-UNIQUE。ADR-0129) */
  unique?: { fields: string[]; where?: string }[];
}

/** 重複不可の索引名(PostgreSQL の 63 バイト制限に収まるよう、表とフィールドのハッシュから作る) */
export function uniqueIndexName(table: string, fields: string[], where: string = ""): string {
  return "uq_" + createHash("sha256").update([table, ...fields, where].join("\u0000")).digest("hex").slice(0, 24);
}

/** 状態の順番(約束 C-SEQUENCE)の検査: 作るときは最初の値、変えるときは同じ値か次の値だけ。
 *  「いつでも」の値(取消など)には、どの値からでも移れる(移ったら、そこで終わり) */
function checkSequence(ctx: Ctx, table: string, before: Row | null, after: Row): void {
  const seqs = (ctx.sequences ?? {})[table];
  if (!seqs) return;
  for (const [field, rule] of Object.entries(seqs)) {
    if (!(field in after)) continue;
    const order = rule.values;
    const anytime = rule.anytime ?? [];
    const to = after[field] as string;
    const chain = order.map((x) => `「${x}」`).join("→") + (anytime.length ? `(${anytime.map((x) => `「${x}」`).join("")}はいつでも)` : "");
    if (before === null) {
      if (to !== order[0]) throw new HttpError(409, "invalid_transition", `${field}は「${order[0]}」から始まります(${chain} の順)`);
      continue;
    }
    const from = before[field] as string;
    if (from === to) continue;
    if (anytime.includes(from)) throw new HttpError(409, "invalid_transition", `${field}は「${from}」で終わりです(${chain} の順)`);
    if (anytime.includes(to)) continue;
    if (order.indexOf(to) !== order.indexOf(from) + 1) {
      const next = order[order.indexOf(from) + 1];
      throw new HttpError(409, "invalid_transition", next === undefined ? `${field}は「${from}」で終わりです(${chain} の順)` : `${field}は「${from}」の次は「${next}」です(${chain} の順)`);
    }
  }
}

function isUniqueViolation(e: unknown): boolean {
  const x = e as { code?: string; message?: string } | null;
  return x?.code === "23505" || /UNIQUE constraint failed/i.test(String(x?.message ?? ""));
}

/** 書込み(重複不可の違反は 409 と日本語の説明にする) */
async function write(ctx: Ctx, table: string, sql: string, params: unknown[]): Promise<number> {
  try {
    return await ctx.conn.run(sql, params);
  } catch (e) {
    if (!isUniqueViolation(e)) throw e;
    const sets = (ctx.uniques ?? {})[table] ?? [];
    const what = sets.map((s) => s.join("と")).join("、または");
    throw new HttpError(409, "duplicate", what ? `既に登録されています(${table}: ${what}が同じものがあります)` : `既に登録されています(${table})`);
  }
}

export class MigrationError extends Error {}

interface ColInfo {
  name: string;
  type: string;
  notnull: boolean;
  pk: boolean;
}

async function describe(db: Database, c: Conn, table: string): Promise<ColInfo[]> {
  if (db.dialect === "sqlite") {
    const info = (await c.all(`PRAGMA table_info(${q(table)})`, [])) as { name: string; type: string; notnull: bigint; pk: bigint }[];
    return info.map((x) => ({ name: x.name, type: x.type.toUpperCase(), notnull: Number(x.notnull) === 1, pk: Number(x.pk) > 0 }));
  }
  const cols = (await c.all(
    `SELECT c.column_name AS name, c.data_type AS type, c.is_nullable AS nullable,
            EXISTS (SELECT 1 FROM information_schema.table_constraints t JOIN information_schema.key_column_usage k
                    ON t.constraint_name = k.constraint_name AND t.table_schema = k.table_schema
                    WHERE t.constraint_type = 'PRIMARY KEY' AND t.table_schema = current_schema() AND t.table_name = c.table_name AND k.column_name = c.column_name) AS pk
     FROM information_schema.columns c WHERE c.table_schema = current_schema() AND c.table_name = ? ORDER BY c.ordinal_position`,
    [table],
  )) as { name: string; type: string; nullable: string; pk: boolean }[];
  return cols.map((x) => ({ name: x.name, type: x.type.toUpperCase(), notnull: x.nullable === "NO", pk: x.pk }));
}

/**
 * 起動時のスキーマ照合(差分マイグレーション)。
 * - 表が無い → 作成
 * - 選択型(NULL 可)の列が増えた → ALTER TABLE ADD COLUMN
 * - 空の表で、型・NOT NULL・主キーが変わった/必須列が増えた/列が消えた → 作り直す
 * - データのある表で上記の破壊的変更 → 起動を拒否(データを黙って壊さない)
 * 適用した操作は _kotonoha_migrations に記録する。
 */
export async function migrate(db: Database, tables: TableDef[]): Promise<string[]> {
  const pg = db.dialect === "postgres";
  if (pg) {
    for (const t of tables) {
      for (const n of [t.name, ...t.columns.map((c) => c.name)]) {
        if (Buffer.byteLength(n, "utf8") > 63) throw new MigrationError(`PostgreSQL の識別子は 63 バイトまでです: ${n}`);
      }
    }
  }
  await db.conn.exec('CREATE TABLE IF NOT EXISTS "_kotonoha_migrations" ("at" TEXT NOT NULL, "action" TEXT NOT NULL)');
  return db.transaction(async (c) => {
    const applied: string[] = [];
    for (const t of tables) {
      const ddl = pg ? t.pg_ddl : t.ddl;
      const info = await describe(db, c, t.name);
      if (info.length === 0) {
        await c.exec(ddl);
        applied.push(`create ${t.name}`);
        continue;
      }
      const have = new Map(info.map((x) => [x.name, x]));
      const rows = Number(((await c.all(`SELECT COUNT(*) AS n FROM ${q(t.name)}`, []))[0] as { n: bigint }).n);
      const additive: ColumnDef[] = [];
      const breaking: string[] = [];
      for (const col of t.columns) {
        const h = have.get(col.name);
        const want = pg ? col.pg_type : col.sql_type;
        if (!h) {
          if (!col.notnull && !col.pk) additive.push(col);
          else breaking.push(`必須列 ${col.name} の追加`);
        } else if (h.type !== want || h.notnull !== col.notnull || h.pk !== col.pk) {
          breaking.push(`列 ${col.name} の型/NOT NULL/主キーの変更`);
        }
      }
      for (const h of info) if (!t.columns.some((x) => x.name === h.name)) breaking.push(`列 ${h.name} の削除`);
      if (breaking.length > 0) {
        if (rows > 0) throw new MigrationError(`表 ${t.name} にデータがあるため自動移行できません: ${breaking.join(", ")}。移行スクリプトを用意してください`);
        await c.exec(`DROP TABLE ${q(t.name)}`);
        await c.exec(ddl);
        applied.push(`recreate ${t.name} (${breaking.join(", ")})`);
        continue;
      }
      for (const col of additive) {
        await c.exec(`ALTER TABLE ${q(t.name)} ADD COLUMN ${pg ? col.pg_def : col.def}`);
        applied.push(`add ${t.name}.${col.name}`);
      }
    }
    // 重複不可(約束 C-UNIQUE): 宣言どおりの一意索引にそろえる
    for (const t of tables) {
      const want = new Map((t.unique ?? []).map((u) => [uniqueIndexName(t.name, u.fields, u.where ?? ""), u]));
      const have = pg
        ? await c.all("SELECT indexname AS name FROM pg_indexes WHERE schemaname = current_schema() AND tablename = ? AND indexname LIKE 'uq\\_%'", [t.name])
        : await c.all("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = ? AND name LIKE 'uq\\_%' ESCAPE '\\'", [t.name]);
      const haveNames = new Set(have.map((r) => String(r.name)));
      for (const n of haveNames) {
        if (!want.has(n)) {
          await c.exec(`DROP INDEX ${q(n)}`);
          applied.push(`drop unique ${t.name}`);
        }
      }
      for (const [n, u] of want) {
        if (haveNames.has(n)) continue;
        const fs = u.fields;
        try {
          await c.exec(`CREATE UNIQUE INDEX ${q(n)} ON ${q(t.name)} (${fs.map(q).join(", ")})${u.where ? ` WHERE ${u.where}` : ""}`);
        } catch (e) {
          throw new MigrationError(`表 ${t.name} に ${fs.join("と")} が同じデータが既に複数あるため「重複不可」を設定できません。重複を解消してから起動してください(${String((e as Error).message ?? e)})`);
        }
        applied.push(`unique ${t.name} (${fs.join(", ")})`);
      }
    }
    for (const a of applied) await c.run('INSERT INTO "_kotonoha_migrations" ("at", "action") VALUES (?, ?)', [new Date().toISOString(), a]);
    return applied;
  });
}

/** KOTONOHA_DB: SQLite のファイルパス(既定 data/app.sqlite、:memory: 可)または postgres:// URL */
export async function openDb(target: string, tables: TableDef[]): Promise<Database> {
  const db = /^postgres(ql)?:\/\//.test(target) ? await openPostgres(target) : await openSqlite(target);
  await migrate(db, tables);
  return db;
}

// ---- 値の変換 ----
function baseWire(w: Wire): Wire {
  return w.kind === "optional" ? w.of : w;
}

/** 内部値 → DB 値 */
export function toSql(v: unknown, w: Wire): unknown {
  if (v === null || v === undefined) return null;
  const b = baseWire(w);
  if (b.kind === "bool") return v ? 1n : 0n;
  return v;
}

/** DB 値 → 内部値 */
export function fromSql(v: unknown, w: Wire): unknown {
  if (v === null || v === undefined) return null;
  const b = baseWire(w);
  if (b.kind === "bool") return v !== 0n && v !== 0;
  if (b.kind === "int" || b.kind === "money" || b.kind === "datetime") return typeof v === "bigint" ? v : BigInt(v as number | string);
  return v;
}

export function decodeRow(ctx: Ctx, table: string, row: Row): Row {
  const fields = ctx.schema.entities[table];
  const e: Row = {};
  for (const f of fields) e[f.name] = fromSql(row[f.name], f.type);
  Object.defineProperty(e, META, { value: { table, original: { ...e } } satisfies EntityMeta, enumerable: false });
  return e;
}

/** 集計などの生 SQL(結果の整数は BigInt) */
export function all(ctx: Ctx, sql: string, params: unknown[]): Promise<Row[]> {
  return ctx.conn.all(sql, params);
}

// ---- 取引 ----
/** `取引:` ブロック。失敗・例外はロールバックして再送出。更新したエンティティは終わりに永続化する。 */
export async function tx<T>(ctx: Ctx, body: () => Promise<T>): Promise<T> {
  if (ctx.dirty !== null) throw new Error("入れ子の取引");
  const outer = ctx.conn;
  try {
    return await ctx.db.transaction(async (c) => {
      ctx.conn = c;
      ctx.dirty = new Set();
      const r = await body();
      await flush(ctx);
      return r;
    });
  } finally {
    ctx.conn = outer;
    ctx.dirty = null;
  }
}

/** エンティティのフィールド更新を記録する(取引の終わりに永続化)。 */
export function markDirty(ctx: Ctx, e: Row): void {
  if (ctx.dirty === null) throw new Error("取引外の更新(型検査で禁止されているはず)");
  if (!(META in e)) throw new Error("保存元不明のエンティティ");
  ctx.dirty.add(e);
}

async function flush(ctx: Ctx): Promise<void> {
  for (const e of ctx.dirty ?? []) {
    const meta = (e as Record<symbol, EntityMeta>)[META];
    const fields = ctx.schema.entities[meta.table];
    const changed = fields.filter((f) => f.name !== "ID" && e[f.name] !== meta.original[f.name]);
    if (changed.length === 0) continue;
    const of = ctx.owners[meta.table];
    if (of !== undefined && changed.some((f) => f.name === of)) throw new HttpError(403, "forbidden", "所有者は変更できません");
    checkSequence(ctx, meta.table, meta.original, Object.fromEntries(changed.map((f) => [f.name, e[f.name]])));
    const sets = changed.map((f) => `${q(f.name)} = ?`).join(", ");
    await write(ctx, meta.table, `UPDATE ${q(meta.table)} SET ${sets} WHERE "ID" = ?`, [...changed.map((f) => toSql(e[f.name], f.type)), e["ID"]]);
    meta.original = { ...e };
  }
}

// ---- 行レベル認可 ----
/** 組込み値 `本人`(動作は常に認証済みで呼ばれる) */
export function principalId(ctx: Ctx): string {
  if (ctx.principal === null) throw new HttpError(401, "unauthenticated", "認証が必要です");
  return ctx.principal.sub;
}

/** 本人の行だけに絞る条件(対象外のデータなら空)。主体がいなければ何も見せない。
 *  ただし許可 `全件閲覧.データ名` を持つ主体には絞らない。 */
function ownerCond(ctx: Ctx, table: string): { sql: string; params: unknown[] } {
  const f = ctx.owners[table];
  if (f === undefined) return { sql: "", params: [] };
  if (ctx.principal === null) return { sql: " AND 1 = 0", params: [] };
  // 許可 `全件閲覧.データ名` を持つ人(厨房・管理者など)は全員の行を扱える(ADR-0118 追補)
  if (ctx.principal.caps.has(`全件閲覧.${table}`)) return { sql: "", params: [] };
  return { sql: ` AND ${q(f)} = ?`, params: [ctx.principal.sub] };
}

// ---- データアダプタ(操作種別ごと) ----
export async function adGet(ctx: Ctx, table: string, id: unknown, optional: boolean): Promise<Row | null> {
  const oc = ownerCond(ctx, table);
  const rows = await ctx.conn.all(`SELECT * FROM ${q(table)} WHERE "ID" = ?${oc.sql}`, [id, ...oc.params]);
  if (rows.length === 0) {
    if (optional) return null;
    // 本人限定のデータは、他の人の分があることも知らせない(同じ 404)。ただし理由の候補は書く(ペルソナ評価の要望)
    throw new HttpError(404, "not_found", ctx.owners[table] !== undefined ? `${table} が見つかりません(無いか、ほかの人のものです)` : `${table} が見つかりません`);
  }
  return decodeRow(ctx, table, rows[0]);
}

export const PAGE_MAX = 1000;

export interface ListView {
  sql: string;
  params: unknown[];
  order: string;
}

/** 一覧 API の並べ替え・絞り込み(?order=&desc=1&eq.F=&ne.F=&today.F=1&me.F=1。ADR-0128)。
 *  フィールド名は表の定義にあるものだけ、値は型で検証する。行レベル認可の条件は常に付ける。 */
export function listView(ctx: Ctx, table: string, query: URLSearchParams = new URLSearchParams()): ListView {
  const oc = ownerCond(ctx, table);
  const fields = ctx.schema.entities[table];
  const fieldOf = (name: string) => {
    const f = fields.find((x) => x.name === name);
    if (!f) throw new HttpError(400, "invalid_input", `${table} にフィールド ${name} はありません`);
    return f;
  };
  let sql = oc.sql;
  const params: unknown[] = [...oc.params];
  for (const [key, raw] of query) {
    const dot = key.indexOf(".");
    if (dot < 0) continue;
    const op = key.slice(0, dot);
    if (!["eq", "ne", "today", "me", "lt_today", "gt_today", "ge_today", "le_today"].includes(op)) continue;
    const f = fieldOf(key.slice(dot + 1));
    const w = baseWire(f.type);
    if (op.endsWith("_today")) {
      // 今日より前・後・以降・まで(アプリの時刻帯の暦日)
      const t = todayIn(appTz());
      const cmp = { lt_today: "<", gt_today: ">", ge_today: ">=", le_today: "<=" }[op]!;
      if (w.kind === "datetime") {
        const bound = op === "lt_today" || op === "ge_today" ? t.start : t.end;
        const c = op === "gt_today" ? ">=" : op === "le_today" ? "<" : cmp;
        sql += ` AND ${q(f.name)} ${c} ?`;
        params.push(BigInt(bound));
      } else if (w.kind === "date") {
        sql += ` AND ${q(f.name)} ${cmp} ?`;
        params.push(t.date);
      } else throw new HttpError(400, "invalid_input", `${f.name} は日時・日付ではありません`);
      continue;
    }
    if (op === "today") {
      const t = todayIn(appTz());
      if (w.kind === "datetime") {
        sql += ` AND ${q(f.name)} >= ? AND ${q(f.name)} < ?`;
        params.push(BigInt(t.start), BigInt(t.end));
      } else if (w.kind === "date") {
        sql += ` AND ${q(f.name)} = ?`;
        params.push(t.date);
      } else throw new HttpError(400, "invalid_input", `${f.name} は日時・日付ではありません`);
      continue;
    }
    if (op === "me") {
      if (ctx.principal === null) {
        sql += " AND 1 = 0";
        continue;
      }
      sql += ` AND ${q(f.name)} = ?`;
      params.push(ctx.principal.sub);
      continue;
    }
    let v: unknown;
    switch (w.kind) {
      case "int":
      case "money":
        if (!/^-?\d+$/.test(raw)) throw new HttpError(400, "invalid_input", `${f.name} は整数です`);
        v = BigInt(raw);
        break;
      case "bool":
        if (raw !== "true" && raw !== "false") throw new HttpError(400, "invalid_input", `${f.name} は true / false です`);
        v = raw === "true" ? 1n : 0n;
        break;
      case "enum":
        if (!ctx.schema.enums[w.name].includes(raw)) throw new HttpError(400, "invalid_input", `${f.name} は ${ctx.schema.enums[w.name].join(" / ")} のいずれかです`);
        v = raw;
        break;
      case "string":
      case "id":
      case "email":
      case "url":
      case "date":
        v = raw;
        break;
      default:
        throw new HttpError(400, "invalid_input", `${f.name} では絞り込めません`);
    }
    // 「でない」は値が空(NULL)の行も含める
    sql += op === "eq" ? ` AND ${q(f.name)} = ?` : ` AND (${q(f.name)} IS NULL OR ${q(f.name)} <> ?)`;
    params.push(v);
  }
  let order = `"ID"`;
  const o = query.get("order");
  if (o !== null) {
    const f = fieldOf(o);
    order = `${q(f.name)} ${query.get("desc") === "1" ? "DESC" : "ASC"}, "ID"`;
  }
  return { sql, params, order };
}

export async function adList(ctx: Ctx, table: string, page: { limit: number; offset: number } = { limit: 1_000_000, offset: 0 }, view: ListView = listView(ctx, table)): Promise<Row[]> {
  const where = view.sql ? ` WHERE 1 = 1${view.sql}` : "";
  const rows = await ctx.conn.all(`SELECT * FROM ${q(table)}${where} ORDER BY ${view.order} LIMIT ? OFFSET ?`, [...view.params, BigInt(page.limit), BigInt(page.offset)]);
  return rows.map((r) => decodeRow(ctx, table, r));
}

export interface AggregateSpec {
  by: string;
  bucket: "day" | "month" | null;
  /** 単位の列名(「品名」「注文日時(日)」) */
  unit: string;
  /** [フィールド, 列名] */
  sums: [string, string][];
  /** 一覧と同じ絞り込み(eq.F など) */
  filters: [string, string][];
}

/** 日本語の文で書いた集計(ADR-0128): 単位ごとの件数と合計。行レベル認可・絞り込みは一覧と同じ。
 *  日時の日・月はアプリの時刻帯で区切る(SQL 方言に依存しないよう、行を読んでから数える) */
export async function adAggregate(ctx: Ctx, table: string, spec: AggregateSpec): Promise<Row[]> {
  const view = listView(ctx, table, new URLSearchParams(spec.filters));
  const where = view.sql ? ` WHERE 1 = 1${view.sql}` : "";
  const fields = ctx.schema.entities[table];
  const byWire = fields.find((f) => f.name === spec.by)!.type;
  const cols = [spec.by, ...spec.sums.map((x) => x[0]).filter((x) => x !== spec.by)];
  const rows = await ctx.conn.all(`SELECT ${cols.map(q).join(", ")} FROM ${q(table)}${where}`, view.params);
  const tz = appTz();
  const enumOrder = baseWire(byWire).kind === "enum" ? ctx.schema.enums[(baseWire(byWire) as { name: string }).name] : null;
  const groups = new Map<string, { key: unknown; sort: unknown; n: bigint; sums: bigint[] }>();
  for (const r of rows) {
    let key = fromSql(r[spec.by], byWire);
    let sort: unknown = key;
    if (spec.bucket !== null && key !== null) {
      let date: string;
      if (baseWire(byWire).kind === "datetime") {
        const t = Number(key as bigint);
        const d = new Date(t + tzOffsetMs(t, tz));
        date = `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;
      } else date = key as string;
      sort = spec.bucket === "day" ? date : date.slice(0, 7);
      key = spec.bucket === "day" ? date : `${Number(date.slice(0, 4))}年${Number(date.slice(5, 7))}月`;
    } else if (enumOrder !== null && key !== null) sort = enumOrder.indexOf(key as string);
    const id = typeof key === "bigint" ? `n:${key}` : JSON.stringify(key);
    let g = groups.get(id);
    if (!g) {
      g = { key, sort, n: 0n, sums: spec.sums.map(() => 0n) };
      groups.set(id, g);
    }
    g.n += 1n;
    spec.sums.forEach(([f], i) => {
      const v = fromSql(r[f], fields.find((x) => x.name === f)!.type);
      if (v !== null) g!.sums[i] += v as bigint;
    });
  }
  const cmp = (a: unknown, b: unknown): number => {
    if (a === b) return 0;
    if (a === null) return 1;
    if (b === null) return -1;
    return (a as number) < (b as number) ? -1 : 1;
  };
  return [...groups.values()]
    .sort((a, b) => cmp(a.sort, b.sort))
    .map((g) => {
      const o: Row = { [spec.unit]: g.key, 件数: g.n };
      spec.sums.forEach(([, col], i) => (o[col] = g.sums[i]));
      return o;
    });
}

export async function adCount(ctx: Ctx, table: string, view: ListView = listView(ctx, table)): Promise<number> {
  const where = view.sql ? ` WHERE 1 = 1${view.sql}` : "";
  return Number((await ctx.conn.all(`SELECT COUNT(*) AS n FROM ${q(table)}${where}`, view.params))[0].n);
}

/** 一覧 API のページング(?limit=&offset=。既定 100 件、上限 1000 件) */
export function pageOf(query: URLSearchParams): { limit: number; offset: number } {
  const num = (k: string, d: number) => {
    if (!query.has(k)) return d;
    const v = Number(query.get(k));
    if (!Number.isSafeInteger(v) || v < 0) throw new HttpError(400, "invalid_input", `${k} は 0 以上の整数です`);
    return v;
  };
  const limit = num("limit", 100);
  if (limit < 1 || limit > PAGE_MAX) throw new HttpError(400, "invalid_input", `limit は 1〜${PAGE_MAX} です`);
  return { limit, offset: num("offset", 0) };
}

export async function adCreate(ctx: Ctx, table: string, values: Row, returns: "id" | "entity"): Promise<unknown> {
  const fields = ctx.schema.entities[table];
  const idField = fields.find((f) => f.name === "ID");
  const row: Row = { ...values };
  if (idField) row["ID"] = randomUUID();
  const of = ctx.owners[table];
  if (of !== undefined) {
    if (ctx.principal === null) throw new HttpError(401, "unauthenticated", "認証が必要です");
    // 全件閲覧の許可を持つ人(配車係・受付など)は、他の人の分として登録できる。それ以外は本人の分だけ
    const staff = ctx.principal.caps.has(`全件閲覧.${table}`);
    if (row[of] !== undefined && row[of] !== ctx.principal.sub && !staff)
      throw new HttpError(403, "forbidden", `他の人の分は登録できません(許可 全件閲覧.${table} が必要です)`);
    if (row[of] === undefined || !staff) row[of] = ctx.principal.sub;
  }
  checkSequence(ctx, table, null, row);
  const cols = fields.filter((f) => f.name in row);
  await write(
    ctx,
    table,
    `INSERT INTO ${q(table)} (${cols.map((f) => q(f.name)).join(", ")}) VALUES (${cols.map(() => "?").join(", ")})`,
    cols.map((f) => toSql(row[f.name], f.type)),
  );
  if (returns === "id") return row["ID"];
  return adGet(ctx, table, row["ID"], false);
}

export async function adUpdate(ctx: Ctx, table: string, values: Row): Promise<Row> {
  const of = ctx.owners[table];
  if (of !== undefined && of in values) throw new HttpError(403, "forbidden", "所有者は変更できません");
  const fields = ctx.schema.entities[table].filter((f) => f.name !== "ID" && f.name in values);
  if (fields.length > 0 && (ctx.sequences ?? {})[table]) {
    checkSequence(ctx, table, await adGet(ctx, table, values["ID"], false), values);
  }
  if (fields.length > 0) {
    const oc = ownerCond(ctx, table);
    const n = await write(ctx, table, `UPDATE ${q(table)} SET ${fields.map((f) => `${q(f.name)} = ?`).join(", ")} WHERE "ID" = ?${oc.sql}`, [
      ...fields.map((f) => toSql(values[f.name], f.type)),
      values["ID"],
      ...oc.params,
    ]);
    if (n === 0) throw new HttpError(404, "not_found", `${table} が見つかりません`);
  }
  return (await adGet(ctx, table, values["ID"], false)) as Row;
}

export async function adDelete(ctx: Ctx, table: string, id: unknown): Promise<boolean> {
  const oc = ownerCond(ctx, table);
  return (await ctx.conn.run(`DELETE FROM ${q(table)} WHERE "ID" = ?${oc.sql}`, [id, ...oc.params])) > 0;
}
