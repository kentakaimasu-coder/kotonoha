// KOTONOHA web runtime: 共通 UI 部品(React)。SSR とブラウザの両方で同じコードを使う。
// React はテキストを常にエスケープする(SEC-05 / TC-008)。dangerouslySetInnerHTML は使わない。
import React from "react";

export const h = React.createElement;

export type Wire =
  | { kind: "bool" | "int" | "string" | "id" | "url" | "email" | "datetime" | "unit" }
  | { kind: "money"; currency: string; minor_units: number }
  | { kind: "enum"; name: string; variants?: string[] }
  | { kind: "entity"; name: string }
  | { kind: "optional"; of: Wire }
  | { kind: "list"; of: Wire }
  | { kind: "record"; fields: { name: string; type: Wire }[] };

export interface Field {
  name: string;
  type: Wire;
}

export interface Loader {
  key: string;
  segments: string[];
  withId: boolean;
  query?: Record<string, string>;
  /** 実験の割付は POST(露出の記録)。再読み込みでは呼び直さない */
  method?: "POST";
}

export interface LoadResult {
  ok: boolean;
  status: number;
  body: any;
}

export interface ScreenProps {
  data: Record<string, LoadResult>;
  id: string | null;
  tz: string;
  reload: () => Promise<void>;
}

export interface ScreenDef {
  name: string;
  input: string | null;
  loaders: Loader[];
  View: (p: ScreenProps) => React.ReactElement;
}

export interface Init {
  appName: string;
  screen: string;
  id: string | null;
  data: Record<string, LoadResult>;
  tz: string;
  nav: string[];
}

export function enc(s: string): string {
  return encodeURIComponent(s);
}

export function loaderUrl(l: Loader, id: string | null): string {
  let p = "/" + l.segments.map(enc).join("/");
  if (l.withId) p += "/" + enc(id ?? "");
  const qs = l.query ? new URLSearchParams(l.query).toString() : "";
  return qs ? `${p}?${qs}` : p;
}

export function screenHref(name: string, id?: string | null): string {
  return "/s/" + enc(name) + (id ? "/" + enc(id) : "");
}

export function formatValue(v: unknown, w: Wire, tz: string): string {
  if (v === null || v === undefined) return "—";
  switch (w.kind) {
    case "optional":
      return formatValue(v, w.of, tz);
    case "money":
      return w.currency === "JPY" ? "¥" + (v as number).toLocaleString("ja-JP") : `${v} ${w.currency}`;
    case "int":
      return (v as number).toLocaleString("ja-JP");
    case "bool":
      return v ? "はい" : "いいえ";
    case "datetime":
      return new Date(v as string).toLocaleString("ja-JP", { timeZone: tz });
    default:
      return String(v);
  }
}

// ---- API(同一オリジンの /api プロキシ経由。認証は HttpOnly Cookie) ----
export async function callApi(method: "GET" | "POST", url: string, body?: unknown): Promise<LoadResult> {
  const res = await fetch(url, {
    method,
    headers: method === "POST" ? { "Content-Type": "application/json", "X-CSRF": "1" } : { "X-CSRF": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: "same-origin",
  });
  let parsed: any = null;
  try {
    parsed = await res.json();
  } catch {
    parsed = null;
  }
  return { ok: res.ok, status: res.status, body: parsed };
}

/** 実験の変種(割付に失敗したら対照群 = 先頭。露出は記録されないので結果は偏らない) */
export function variantOf(r: LoadResult | undefined, variants: string[]): string {
  const v = r?.ok ? r.body?.variant : undefined;
  return typeof v === "string" && variants.includes(v) ? v : variants[0];
}

/** 拡張部品の描画失敗を画面全体に広げない */
export class ExtBoundary extends React.Component<{ name: string; children?: React.ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? h("p", { className: "notice", role: "alert" }, `部品「${this.props.name}」を表示できませんでした`) : this.props.children;
  }
}

/** 動作の成功結果の表示(K-13)。値が単純なときだけ添える。 */
export function okText(prefix: string, r: LoadResult): string {
  const v = r.body?.ok;
  if (typeof v === "string" || typeof v === "number" || typeof v === "boolean") return `${prefix}: 完了しました(結果: ${String(v)})`;
  return `${prefix}: 完了しました`;
}

export function errorText(r: LoadResult): string {
  if (r.status === 401) return "ログインが必要です";
  if (r.status === 403) return "権限がありません";
  if (r.status === 404) return "見つかりません";
  if (r.status === 429) return "要求が多すぎます。しばらく待ってから再試行してください";
  if (r.status === 422 && r.body?.error === "failure") return `失敗しました: ${String(r.body.value)}`;
  if (r.body?.message) return String(r.body.message);
  return `エラー(${r.status})`;
}

function Notice(p: { r: LoadResult }) {
  return h(
    "p",
    { className: "notice", role: "alert" },
    errorText(p.r),
    p.r.status === 401 ? h(React.Fragment, null, " ", h("a", { href: "/login" }, "ログイン")) : null,
  );
}

// ---- 部品 ----
export function Page(p: { title: string; children?: React.ReactNode }) {
  return h("section", { className: "page", "aria-labelledby": "page-title" }, h("h1", { id: "page-title" }, p.title), p.children);
}

export function DataList(p: { r: LoadResult | undefined; fields: Field[]; tz: string; link?: (row: any) => string; caption: string }) {
  if (!p.r) return null;
  if (!p.r.ok) return h(Notice, { r: p.r });
  const rows: any[] = p.r.body ?? [];
  if (rows.length === 0) return h("p", { className: "empty" }, "データがありません");
  return h(
    "table",
    { className: "list" },
    h("caption", null, p.caption),
    h("thead", null, h("tr", null, p.fields.map((f) => h("th", { key: f.name, scope: "col" }, f.name)))),
    h(
      "tbody",
      null,
      rows.map((row, i) =>
        h(
          "tr",
          { key: row.ID ?? i },
          p.fields.map((f, j) => {
            const text = formatValue(row[f.name], f.type, p.tz);
            return h("td", { key: f.name }, j === 0 && p.link ? h("a", { href: p.link(row) }, text) : text);
          }),
        ),
      ),
    ),
  );
}

export function Detail(p: { r: LoadResult | undefined; fields: Field[]; tz: string }) {
  if (!p.r) return null;
  if (!p.r.ok) return h(Notice, { r: p.r });
  const e = p.r.body ?? {};
  return h("dl", { className: "detail" }, p.fields.map((f) => h(React.Fragment, { key: f.name }, h("dt", null, f.name), h("dd", null, formatValue(e[f.name], f.type, p.tz)))));
}

function useMessage(): [{ kind: "ok" | "error"; text: string; login?: boolean } | null, (m: { kind: "ok" | "error"; text: string; login?: boolean } | null) => void] {
  return React.useState<{ kind: "ok" | "error"; text: string; login?: boolean } | null>(null);
}

function MessageView(p: { m: { kind: "ok" | "error"; text: string; login?: boolean } | null }) {
  return h(
    "p",
    { className: p.m ? `msg ${p.m.kind}` : "msg", role: "status", "aria-live": "polite" },
    p.m ? p.m.text : "",
    p.m?.login ? h(React.Fragment, null, " ", h("a", { href: "/login" }, "ログイン")) : null,
  );
}

export function ActionButton(p: { label: string; action: string; args: () => Record<string, unknown> | null; reload: () => Promise<void> }) {
  const [m, setM] = useMessage();
  const [busy, setBusy] = React.useState(false);
  const onClick = async () => {
    const args = p.args();
    if (args === null) return;
    setBusy(true);
    try {
      const r = await callApi("POST", "/api/actions/" + enc(p.action), args);
      if (r.ok) {
        setM({ kind: "ok", text: okText(p.label, r) });
        await p.reload();
      } else setM({ kind: "error", text: errorText(r), login: r.status === 401 });
    } finally {
      setBusy(false);
    }
  };
  return h("div", { className: "action" }, h("button", { type: "button", onClick, disabled: busy }, p.label), h(MessageView, { m }));
}

function inputFor(f: Field, id: string) {
  const base = f.type.kind === "optional" ? f.type.of : f.type;
  const common = { id, name: f.name, required: f.type.kind !== "optional" };
  switch (base.kind) {
    case "int":
    case "money":
      // type="number" は全角数字を受け付けないブラウザがあるため、text + inputMode で受けて正規化する
      return h("input", { ...common, type: "text", inputMode: "numeric", autoComplete: "off", pattern: "[-－−ー]?[0-9０-９,、 ]+" });
    case "bool":
      return h("input", { id, name: f.name, type: "checkbox" });
    case "enum":
      return h("select", common, (base.variants ?? []).map((v) => h("option", { key: v, value: v }, v)));
    case "datetime":
      return h("input", { ...common, type: "datetime-local" });
    case "email":
      return h("input", { ...common, type: "email" });
    case "url":
      return h("input", { ...common, type: "url" });
    default:
      return h("input", { ...common, type: "text", maxLength: base.kind === "id" ? 256 : 10000 });
  }
}

/** 数値欄の入力を正規化する: 全角数字・全角マイナス(－ − ー)・桁区切りのカンマ・前後の空白を受け付ける(NR-01) */
export function normalizeNumber(raw: string): string {
  return raw
    .normalize("NFKC")
    .replace(/[\u2212\u30FC\uFF0D\u2010-\u2015]/g, "-")
    .replace(/[,、\s]/g, "");
}

function readForm(form: HTMLFormElement, fields: Field[]): Record<string, unknown> | string {
  const out: Record<string, unknown> = {};
  for (const f of fields) {
    const el = form.elements.namedItem(f.name) as HTMLInputElement | null;
    if (!el) continue;
    const optional = f.type.kind === "optional";
    const base = optional ? (f.type as any).of : f.type;
    const raw = el.value;
    if (base.kind === "bool") {
      out[f.name] = el.checked;
      continue;
    }
    if (raw === "" && optional) {
      out[f.name] = null;
      continue;
    }
    if (base.kind === "int" || base.kind === "money") {
      const text = normalizeNumber(raw);
      const n = /^-?\d+$/.test(text) ? Number(text) : NaN;
      if (!Number.isSafeInteger(n)) return `${f.name} は整数で入力してください`;
      out[f.name] = n;
    } else if (base.kind === "datetime") {
      const t = new Date(raw);
      if (Number.isNaN(t.getTime())) return `${f.name} の日時が不正です`;
      out[f.name] = t.toISOString();
    } else out[f.name] = raw;
  }
  return out;
}

export function ActionForm(p: { action: string; fields: Field[]; reload: () => Promise<void> }) {
  const [m, setM] = useMessage();
  const [busy, setBusy] = React.useState(false);
  const formId = "form-" + p.action;
  const onSubmit = async (ev: any) => {
    ev.preventDefault();
    const form = ev.currentTarget as HTMLFormElement;
    const v = readForm(form, p.fields);
    if (typeof v === "string") return setM({ kind: "error", text: v });
    setBusy(true);
    try {
      const r = await callApi("POST", "/api/actions/" + enc(p.action), v);
      if (r.ok) {
        setM({ kind: "ok", text: okText(p.action, r) });
        form.reset();
        await p.reload();
      } else setM({ kind: "error", text: errorText(r), login: r.status === 401 });
    } finally {
      setBusy(false);
    }
  };
  // IME の変換確定の Enter でフォームを送信しない(NR-01)
  const onKeyDown = (ev: any) => {
    if (ev.key === "Enter" && (ev.nativeEvent?.isComposing || ev.keyCode === 229)) ev.preventDefault();
  };
  return h(
    "form",
    { className: "form", onSubmit, onKeyDown, "aria-labelledby": formId + "-title" },
    h("h2", { id: formId + "-title" }, p.action),
    p.fields.map((f) => h("div", { className: "field", key: f.name }, h("label", { htmlFor: `${formId}-${f.name}` }, f.name), inputFor(f, `${formId}-${f.name}`))),
    h("button", { type: "submit", disabled: busy }, "送信"),
    h(MessageView, { m }),
  );
}

// ---- アプリ外枠 ----
export function App(p: { init: Init; screens: Record<string, ScreenDef> }) {
  const def = p.screens[p.init.screen];
  const [data, setData] = React.useState(p.init.data);
  const reload = async () => {
    const next: Record<string, LoadResult> = {};
    for (const l of def.loaders) next[l.key] = l.method === "POST" ? data[l.key] : await callApi("GET", loaderUrl(l, p.init.id));
    setData(next);
  };
  return h(
    "div",
    { className: "app" },
    h(
      "header",
      null,
      h("a", { className: "brand", href: "/" }, p.init.appName),
      h("nav", { "aria-label": "画面" }, p.init.nav.map((n) => h("a", { key: n, href: screenHref(n), "aria-current": n === p.init.screen ? "page" : undefined }, n))),
      h("a", { className: "login", href: "/login" }, "ログイン"),
    ),
    h("main", null, h(def.View, { data, id: p.init.id, tz: p.init.tz, reload })),
  );
}
