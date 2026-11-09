// このファイルは koto build --target backend が生成しました。手で編集しないでください(正本は KOTONOHA ソース)。
import * as rt from "./runtime/rt.ts";
import * as db from "./runtime/db.ts";
import type { Ctx } from "./runtime/db.ts";
import type { App, Route } from "./runtime/server.ts";

export const schema: rt.Schema = {
  "entities": {
    "商品": [
      {
        "name": "ID",
        "type": {
          "kind": "id"
        }
      },
      {
        "name": "名前",
        "type": {
          "kind": "string"
        }
      },
      {
        "name": "価格",
        "type": {
          "currency": "JPY",
          "kind": "money",
          "minor_units": 0
        }
      },
      {
        "name": "在庫",
        "type": {
          "kind": "int"
        }
      }
    ],
    "注文": [
      {
        "name": "ID",
        "type": {
          "kind": "id"
        }
      },
      {
        "name": "商品ID",
        "type": {
          "kind": "id"
        }
      },
      {
        "name": "数量",
        "type": {
          "kind": "int"
        }
      },
      {
        "name": "注文者",
        "type": {
          "kind": "id"
        }
      }
    ],
    "注文明細": [
      {
        "name": "ID",
        "type": {
          "kind": "id"
        }
      },
      {
        "name": "商品ID",
        "type": {
          "kind": "id"
        }
      },
      {
        "name": "売上金額",
        "type": {
          "currency": "JPY",
          "kind": "money",
          "minor_units": 0
        }
      },
      {
        "name": "状態",
        "type": {
          "kind": "enum",
          "name": "注文状態"
        }
      },
      {
        "name": "確定日時",
        "type": {
          "kind": "datetime"
        }
      }
    ]
  },
  "enums": {
    "注文失敗": [
      "数量不正",
      "在庫不足"
    ],
    "注文状態": [
      "確定",
      "取消"
    ],
    "登録失敗": [
      "在庫不正"
    ]
  }
} as any;

export const tables: db.TableDef[] = [
  {
    "columns": [
      {
        "def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "name": "ID",
        "notnull": true,
        "pg_def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "pg_type": "TEXT",
        "pk": true,
        "sql_type": "TEXT"
      },
      {
        "def": "\"名前\" TEXT NOT NULL",
        "name": "名前",
        "notnull": true,
        "pg_def": "\"名前\" TEXT NOT NULL",
        "pg_type": "TEXT",
        "pk": false,
        "sql_type": "TEXT"
      },
      {
        "def": "\"価格\" INTEGER NOT NULL",
        "name": "価格",
        "notnull": true,
        "pg_def": "\"価格\" BIGINT NOT NULL",
        "pg_type": "BIGINT",
        "pk": false,
        "sql_type": "INTEGER"
      },
      {
        "def": "\"在庫\" INTEGER NOT NULL",
        "name": "在庫",
        "notnull": true,
        "pg_def": "\"在庫\" BIGINT NOT NULL",
        "pg_type": "BIGINT",
        "pk": false,
        "sql_type": "INTEGER"
      }
    ],
    "ddl": "CREATE TABLE IF NOT EXISTS \"商品\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"名前\" TEXT NOT NULL,\n    \"価格\" INTEGER NOT NULL,\n    \"在庫\" INTEGER NOT NULL\n);",
    "name": "商品",
    "pg_ddl": "CREATE TABLE IF NOT EXISTS \"商品\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"名前\" TEXT NOT NULL,\n    \"価格\" BIGINT NOT NULL,\n    \"在庫\" BIGINT NOT NULL\n);"
  },
  {
    "columns": [
      {
        "def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "name": "ID",
        "notnull": true,
        "pg_def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "pg_type": "TEXT",
        "pk": true,
        "sql_type": "TEXT"
      },
      {
        "def": "\"商品ID\" TEXT NOT NULL",
        "name": "商品ID",
        "notnull": true,
        "pg_def": "\"商品ID\" TEXT NOT NULL",
        "pg_type": "TEXT",
        "pk": false,
        "sql_type": "TEXT"
      },
      {
        "def": "\"数量\" INTEGER NOT NULL",
        "name": "数量",
        "notnull": true,
        "pg_def": "\"数量\" BIGINT NOT NULL",
        "pg_type": "BIGINT",
        "pk": false,
        "sql_type": "INTEGER"
      },
      {
        "def": "\"注文者\" TEXT NOT NULL",
        "name": "注文者",
        "notnull": true,
        "pg_def": "\"注文者\" TEXT NOT NULL",
        "pg_type": "TEXT",
        "pk": false,
        "sql_type": "TEXT"
      }
    ],
    "ddl": "CREATE TABLE IF NOT EXISTS \"注文\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"商品ID\" TEXT NOT NULL,\n    \"数量\" INTEGER NOT NULL,\n    \"注文者\" TEXT NOT NULL\n);",
    "name": "注文",
    "pg_ddl": "CREATE TABLE IF NOT EXISTS \"注文\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"商品ID\" TEXT NOT NULL,\n    \"数量\" BIGINT NOT NULL,\n    \"注文者\" TEXT NOT NULL\n);"
  },
  {
    "columns": [
      {
        "def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "name": "ID",
        "notnull": true,
        "pg_def": "\"ID\" TEXT NOT NULL PRIMARY KEY",
        "pg_type": "TEXT",
        "pk": true,
        "sql_type": "TEXT"
      },
      {
        "def": "\"商品ID\" TEXT NOT NULL",
        "name": "商品ID",
        "notnull": true,
        "pg_def": "\"商品ID\" TEXT NOT NULL",
        "pg_type": "TEXT",
        "pk": false,
        "sql_type": "TEXT"
      },
      {
        "def": "\"売上金額\" INTEGER NOT NULL",
        "name": "売上金額",
        "notnull": true,
        "pg_def": "\"売上金額\" BIGINT NOT NULL",
        "pg_type": "BIGINT",
        "pk": false,
        "sql_type": "INTEGER"
      },
      {
        "def": "\"状態\" TEXT NOT NULL CHECK (\"状態\" IN ('確定', '取消'))",
        "name": "状態",
        "notnull": true,
        "pg_def": "\"状態\" TEXT NOT NULL CHECK (\"状態\" IN ('確定', '取消'))",
        "pg_type": "TEXT",
        "pk": false,
        "sql_type": "TEXT"
      },
      {
        "def": "\"確定日時\" INTEGER NOT NULL",
        "name": "確定日時",
        "notnull": true,
        "pg_def": "\"確定日時\" BIGINT NOT NULL",
        "pg_type": "BIGINT",
        "pk": false,
        "sql_type": "INTEGER"
      }
    ],
    "ddl": "CREATE TABLE IF NOT EXISTS \"注文明細\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"商品ID\" TEXT NOT NULL,\n    \"売上金額\" INTEGER NOT NULL,\n    \"状態\" TEXT NOT NULL CHECK (\"状態\" IN ('確定', '取消')),\n    \"確定日時\" INTEGER NOT NULL\n);",
    "name": "注文明細",
    "pg_ddl": "CREATE TABLE IF NOT EXISTS \"注文明細\" (\n    \"ID\" TEXT NOT NULL PRIMARY KEY,\n    \"商品ID\" TEXT NOT NULL,\n    \"売上金額\" BIGINT NOT NULL,\n    \"状態\" TEXT NOT NULL CHECK (\"状態\" IN ('確定', '取消')),\n    \"確定日時\" BIGINT NOT NULL\n);"
  }
];

// アダプタ 商品をIDで取得(操作 取得(商品))
async function ad_商品をIDで取得(ctx: Ctx, v_ID: any): Promise<any> {
  return db.adGet(ctx, "商品", v_ID, false);
}

// アダプタ 商品を新規登録(操作 登録(商品))
async function ad_商品を新規登録(ctx: Ctx, v_名前: any, v_価格: any, v_在庫: any): Promise<any> {
  return db.adCreate(ctx, "商品", { "名前": v_名前, "価格": v_価格, "在庫": v_在庫 }, "id");
}

// アダプタ 注文を登録(操作 登録(注文))
async function ad_注文を登録(ctx: Ctx, v_商品ID: any, v_数量: any, v_注文者: any): Promise<any> {
  return db.adCreate(ctx, "注文", { "商品ID": v_商品ID, "数量": v_数量, "注文者": v_注文者 }, "id");
}

// 動作 商品登録  必要: [商品.create]
export async function act_商品登録(ctx: Ctx, v_名前: any, v_価格: any, v_在庫: any): Promise<any> {
  const __t: any = await db.tx(ctx, async () => {
    if (!((v_在庫 >= 0n))) throw new rt.Failure("在庫不正");
    let v_新ID: any = (await ad_商品を新規登録(ctx, v_名前, v_価格, v_在庫));
    return { v: v_新ID };
    return undefined;
  });
  if (__t !== undefined) return __t.v;
  return undefined;
}

// 動作 注文する  必要: [商品.read, 商品.update, 注文.create]
export async function act_注文する(ctx: Ctx, v_商品ID: any, v_数量: any): Promise<any> {
  const __t: any = await db.tx(ctx, async () => {
    if (!((v_数量 > 0n))) throw new rt.Failure("数量不正");
    {
      let v_商品: any = (await ad_商品をIDで取得(ctx, v_商品ID));
      if (!((v_商品["在庫"] >= v_数量))) throw new rt.Failure("在庫不足");
      v_商品["在庫"] = rt.sub(v_商品["在庫"], v_数量);
      db.markDirty(ctx, v_商品, "在庫");
      let v_注文ID: any = (await ad_注文を登録(ctx, v_商品ID, v_数量, db.principalId(ctx)));
      return { v: v_注文ID };
    }
    return undefined;
  });
  if (__t !== undefined) return __t.v;
  return undefined;
}

export const routes: Route[] = [
  {
    id: "data.list.商品",
    method: "GET",
    segments: ["api","data","商品"],
    hasId: false,
    access: {"capabilities":[],"public":true},
    handle: async (ctx, req) => { const v = db.listView(ctx, "商品", req.query); return { status: 200, body: (await db.adList(ctx, "商品", db.pageOf(req.query), v)).map((e) => rt.encode(e, { kind: "entity", name: "商品" }, schema)), headers: { "X-Total-Count": String(await db.adCount(ctx, "商品", v)) } }; },
  },
  {
    id: "data.list.注文",
    method: "GET",
    segments: ["api","data","注文"],
    hasId: false,
    access: {"capabilities":[],"owner":"注文","public":false},
    handle: async (ctx, req) => { const v = db.listView(ctx, "注文", req.query); return { status: 200, body: (await db.adList(ctx, "注文", db.pageOf(req.query), v)).map((e) => rt.encode(e, { kind: "entity", name: "注文" }, schema)), headers: { "X-Total-Count": String(await db.adCount(ctx, "注文", v)) } }; },
  },
  {
    id: "data.get.商品",
    method: "GET",
    segments: ["api","data","商品"],
    hasId: true,
    access: {"capabilities":[],"public":true},
    handle: async (ctx, req) => ({ status: 200, body: rt.encode(await db.adGet(ctx, "商品", rt.decodeInput(req.id, { kind: "id" }, schema, "id"), false), { kind: "entity", name: "商品" }, schema) }),
  },
  {
    id: "query.売れ筋商品",
    method: "GET",
    segments: ["api","queries","売れ筋商品"],
    hasId: false,
    access: {"capabilities":["注文明細.read"],"public":false},
    handle: async (ctx, req) => {
      const allowed = [20];
      const limit = req.query.has("limit") ? Number(req.query.get("limit")) : allowed[allowed.length - 1];
      if (!allowed.includes(limit)) throw new rt.HttpError(400, "invalid_input", `limit は ${allowed.join(" / ")} のいずれかです`);
      const base = req.query.has("base") ? (rt.decodeInput(req.query.get("base"), { kind: "datetime" }, schema, "base") as bigint) : BigInt(Date.now());
      const days = 30;
      const rows = await db.all(ctx, "SELECT \"商品ID\", SUM(\"売上金額\") AS \"売上金額\"\nFROM \"注文明細\"\nWHERE (\"状態\" = ?1)\n  AND \"確定日時\" >= ?2\n  AND \"確定日時\" < ?3\nGROUP BY \"商品ID\"\nORDER BY \"売上金額\" DESC, \"商品ID\" ASC\nLIMIT ?4;", ["確定", base - BigInt(days) * 86400000n, base, BigInt(limit)]);
      const w: any = {"fields":[{"name":"商品ID","type":{"kind":"id"}},{"name":"売上金額","type":{"currency":"JPY","kind":"money","minor_units":0}}],"kind":"record"};
      return { status: 200, body: rows.map((r) => rt.encode(r, w, schema)) };
    },
  },
  {
    id: "action.商品登録",
    method: "POST",
    segments: ["api","actions","商品登録"],
    hasId: false,
    access: {"capabilities":["商品.create"],"grant":"動作.商品登録","public":false},
    handle: async (ctx, req) => {
      const b = req.body as Record<string, unknown>;
      if (b === null || typeof b !== "object" || Array.isArray(b)) throw new rt.HttpError(400, "invalid_input", "本文はオブジェクトです");
      const extra = Object.keys(b).filter((k) => !["名前", "価格", "在庫"].includes(k));
      if (extra.length > 0) throw new rt.HttpError(400, "invalid_input", `未知の項目: ${extra.join(", ")}`);
      const v = await act_商品登録(ctx, rt.decodeInput(b["名前"], {"kind":"string"}, schema, "名前"), rt.decodeInput(b["価格"], {"currency":"JPY","kind":"money","minor_units":0}, schema, "価格"), rt.decodeInput(b["在庫"], {"kind":"int"}, schema, "在庫"));
      return { status: 200, body: { ok: rt.encode(v, {"kind":"id"}, schema) } };
    },
  },
  {
    id: "action.注文する",
    method: "POST",
    segments: ["api","actions","注文する"],
    hasId: false,
    access: {"capabilities":["商品.read","商品.update","注文.create"],"grant":"動作.注文する","public":false},
    handle: async (ctx, req) => {
      const b = req.body as Record<string, unknown>;
      if (b === null || typeof b !== "object" || Array.isArray(b)) throw new rt.HttpError(400, "invalid_input", "本文はオブジェクトです");
      const extra = Object.keys(b).filter((k) => !["商品ID", "数量"].includes(k));
      if (extra.length > 0) throw new rt.HttpError(400, "invalid_input", `未知の項目: ${extra.join(", ")}`);
      const v = await act_注文する(ctx, rt.decodeInput(b["商品ID"], {"kind":"id"}, schema, "商品ID"), rt.decodeInput(b["数量"], {"kind":"int"}, schema, "数量"));
      return { status: 200, body: { ok: rt.encode(v, {"kind":"id"}, schema) } };
    },
  },
];

export const app: App = { name: "商店", schema, routes, owners: {"注文":"注文者"}, roles: {} };
