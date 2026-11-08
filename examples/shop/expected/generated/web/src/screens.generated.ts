// このファイルは koto build --target web が生成しました。手で編集しないでください(正本は KOTONOHA ソース)。
import { h, Page, DataList, Detail, ActionButton, ActionForm, AnalysisQuery, ExtBoundary, variantOf, screenHref, type ScreenDef, type ScreenProps } from "./runtime/ui.ts";
import type { WebApp } from "./runtime/server.ts";

function Screen_商品一覧(p: ScreenProps) {
  return h(Page, { title: "商品一覧" },
    h(DataList, { key: 0, r: p.data["list:商品"], fields: [{"name":"名前","type":{"kind":"string"}},{"name":"価格","type":{"currency":"JPY","kind":"money","minor_units":0}},{"name":"在庫","type":{"kind":"int"}}], tz: p.tz, caption: "商品の一覧", link: (row: any) => screenHref("商品詳細", row["ID"]) })
  );
}

function Screen_商品詳細(p: ScreenProps) {
  return h(Page, { title: "商品詳細" },
    h(Detail, { key: 0, r: p.data["get:商品"], fields: [{"name":"名前","type":{"kind":"string"}},{"name":"価格","type":{"currency":"JPY","kind":"money","minor_units":0}},{"name":"在庫","type":{"kind":"int"}}], tz: p.tz }),
    h(ActionButton, { key: 1, label: "購入", action: "注文する", reload: p.reload, need: {"caps":["商品.read","商品.update","注文.create"],"grant":"動作.注文する"}, caps: p.caps, args: () => { const r = p.data["get:商品"]; if (!r?.ok) return null; const cur: any = r.body; return { "商品ID": cur["ID"], "数量": 1 }; } })
  );
}

function Screen_商品管理(p: ScreenProps) {
  return h(Page, { title: "商品管理" },
    h(ActionForm, { key: 0, action: "商品登録", fields: [{"name":"名前","type":{"kind":"string"}},{"name":"価格","type":{"currency":"JPY","kind":"money","minor_units":0}},{"name":"在庫","type":{"kind":"int"}}], reload: p.reload, tz: p.tz, fixed: null, need: {"caps":["商品.create"],"grant":"動作.商品登録"}, caps: p.caps }),
    h(DataList, { key: 1, r: p.data["list:商品"], fields: [{"name":"名前","type":{"kind":"string"}},{"name":"価格","type":{"currency":"JPY","kind":"money","minor_units":0}},{"name":"在庫","type":{"kind":"int"}}], tz: p.tz, caption: "商品の一覧", link: (row: any) => screenHref("商品詳細", row["ID"]) })
  );
}

function Screen_注文履歴(p: ScreenProps) {
  return h(Page, { title: "注文履歴" },
    h(DataList, { key: 0, r: p.data["list:注文"], fields: [{"name":"商品ID","type":{"kind":"id"}},{"name":"数量","type":{"kind":"int"}},{"name":"注文者","type":{"kind":"id"}}], tz: p.tz, caption: "注文の一覧", refs: { "商品ID": { r: p.data["list:商品"], label: "名前", title: "商品" } } })
  );
}

function Screen_売れ筋(p: ScreenProps) {
  return h(Page, { title: "売れ筋" },
    h(DataList, { key: 0, r: p.data["query:売れ筋商品:20"], fields: [{"name":"商品ID","type":{"kind":"id"}},{"name":"売上金額","type":{"currency":"JPY","kind":"money","minor_units":0}}], tz: p.tz, caption: "売れ筋商品(上位20件)" })
  );
}

export const screens: Record<string, ScreenDef> = {
  "商品一覧": { name: "商品一覧", input: null, loaders: [{"key":"list:商品","segments":["api","data","商品"],"withId":false}], View: Screen_商品一覧 },
  "商品詳細": { name: "商品詳細", input: "商品", loaders: [{"key":"get:商品","segments":["api","data","商品"],"withId":true}], View: Screen_商品詳細 },
  "商品管理": { name: "商品管理", input: null, loaders: [{"key":"list:商品","segments":["api","data","商品"],"withId":false}], View: Screen_商品管理 },
  "注文履歴": { name: "注文履歴", input: null, loaders: [{"key":"list:注文","segments":["api","data","注文"],"withId":false},{"key":"list:商品","segments":["api","data","商品"],"withId":false}], View: Screen_注文履歴 },
  "売れ筋": { name: "売れ筋", input: null, loaders: [{"key":"query:売れ筋商品:20","query":{"limit":"20"},"segments":["api","queries","売れ筋商品"],"withId":false}], View: Screen_売れ筋 },
};

export const webApp: WebApp = { appName: "商店", entry: "商品一覧", nav: ["商品一覧","商品管理","注文履歴","売れ筋"], screens, roles: {} };
