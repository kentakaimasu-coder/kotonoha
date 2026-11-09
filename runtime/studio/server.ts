// KOTONOHA Studio(ADR-0135): 画面でアプリを作るためのローカルサーバー。`koto studio` が起動する。
// - ソース(src/*.koto)が正。画面での操作は、すべてソースの行の追加・書き換えとして行い、koto outline で検査し直す
// - 日本語の指示は、決まった言い方(データを作る・項目を追加する など)なら操作に、それ以外は文としてソースに入れ、
//   コンパイラが読めなければ元に戻して「近い書き方」を返す(AI は使わない。解釈の判断はコンパイラだけ)
// - 127.0.0.1 だけで待ち受け、Host を確かめ、書き込みには起動時に作った合言葉(ヘッダ)を求める(他のサイトから操作させない)
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, realpathSync } from "node:fs";
import { join, resolve, sep, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";
import { createServer as netServer } from "node:net";

const here = dirname(fileURLToPath(import.meta.url));
const KOTO = process.env.KOTO_BIN ?? "koto";
const PROJECT = realpathSync(process.env.KOTO_PROJECT ?? ".");
const PORT = Number(process.env.PORT ?? 4000);
const TOKEN = randomBytes(24).toString("hex");
const HOSTS = new Set([`127.0.0.1:${PORT}`, `localhost:${PORT}`]);

// ---- koto の呼び出し ----
function koto(args: string[]): { code: number; out: string; err: string } {
  const r = spawnSync(KOTO, ["-C", PROJECT, ...args], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  return { code: r.status ?? 1, out: r.stdout ?? "", err: r.stderr ?? "" };
}
function outline(): any {
  const r = koto(["outline"]);
  try {
    return JSON.parse(r.out);
  } catch {
    return { ok: false, decls: [], diagnostics: [{ severity: "error", code: "STUDIO", message_ja: r.err.trim() || "読み込めませんでした", file: "", span: { line: 0 }, hints: [] }], files: [], patterns: { screen: [], rules: [] }, types: [], enums: {} };
  }
}

// ---- ソースの読み書き(プロジェクトの中の .koto だけ) ----
function srcPath(file: string): string {
  const p = resolve(PROJECT, file);
  if (!(p.startsWith(PROJECT + sep) && p.endsWith(".koto"))) throw new Error("プロジェクトの外のファイルは扱えません");
  return p;
}
function mainFile(o: any): string {
  return o.files?.[0] ?? "src/app.koto";
}
const history: { file: string; text: string }[] = [];
function read(file: string): string {
  return readFileSync(srcPath(file), "utf8");
}
function write(file: string, text: string): void {
  history.push({ file, text: read(file) });
  if (history.length > 100) history.shift();
  writeFileSync(srcPath(file), text.endsWith("\n") ? text : text + "\n");
}
function lines(file: string): string[] {
  return read(file).split("\n");
}
/** n 行目(1 から)の後ろに行を差し込む */
function insertAfter(file: string, n: number, add: string[]): number {
  const ls = lines(file);
  ls.splice(n, 0, ...add);
  write(file, ls.join("\n"));
  return n + 1;
}

// ---- 名前と型の確かめ ----
const NAME = /^[^\s:：()（）「」"'`,、。=<>\[\]{}#.\-+*/\\]+$/u;
function okName(s: string, what: string): string {
  const t = (s ?? "").trim();
  if (!NAME.test(t)) throw new Error(`${what}「${t}」は使えません(空白や記号を含まない名前にします)`);
  return t;
}
const TYPE_WORDS: Record<string, string> = {
  文字: "文字列", 文字列: "文字列", 文章: "文字列", テキスト: "文字列", 名前: "文字列",
  数: "整数", 数字: "整数", 整数: "整数", 個数: "整数", 人数: "整数",
  真偽: "真偽", "はい/いいえ": "真偽", はいいいえ: "真偽", チェック: "真偽",
  日付: "日付", 日: "日付", 日時: "日時", 時刻: "日時",
  金額: "金額<JPY>", お金: "金額<JPY>", 円: "金額<JPY>", 価格: "金額<JPY>",
  メール: "メール", メールアドレス: "メール", URL: "URL", 識別子: "識別子", ID: "識別子",
};
function okType(t: string, o: any): string {
  let s = (t ?? "").trim() || "文字列";
  let optional = false;
  const m = /^選択<(.+)>$/.exec(s) ?? /^(.+?)(?:\(任意\)|(任意)|・任意|、任意)$/.exec(s);
  if (m) {
    optional = true;
    s = m[1].trim();
  }
  const base = TYPE_WORDS[s] ?? (o.enums && s in o.enums ? s : (o.types ?? []).includes(s) ? s : null);
  if (!base) throw new Error(`型「${t}」は分かりません(使える型: ${[...new Set(Object.values(TYPE_WORDS))].join(" / ")}${Object.keys(o.enums ?? {}).length ? " / 列挙: " + Object.keys(o.enums).join(" / ") : ""})`);
  return optional ? `選択<${base}>` : base;
}
function decl(o: any, kind: string, name?: string): any {
  return (o.decls ?? []).find((d: any) => d.kind === kind && (name === undefined || d.name === name));
}
function lastOf(o: any, kinds: string[]): any {
  const ds = (o.decls ?? []).filter((d: any) => kinds.includes(d.kind));
  return ds[ds.length - 1];
}
/** 新しい宣言を、同じ種類の最後の宣言の後ろ(無ければ近い種類の後ろ、無ければファイルの終わり)に入れる */
function addDecl(o: any, kinds: string[], block: string[]): { file: string; line: number } {
  const after = lastOf(o, kinds) ?? lastOf(o, ["data", "enum", "app"]);
  const file = after?.file ?? mainFile(o);
  const n = after ? after.end : lines(file).length;
  insertAfter(file, n, ["", ...block]);
  return { file, line: n + 2 };
}
function addLine(d: any, text: string): { file: string; line: number } {
  insertAfter(d.file, d.end, ["    " + text.trim()]);
  return { file: d.file, line: d.end + 1 };
}

// ---- 操作 ----
type Op = Record<string, any>;
function apply(op: Op): { file: string; line: number } | null {
  const o = outline();
  switch (op.op) {
    case "add_data": {
      const name = okName(op.name, "データの名前");
      if (decl(o, "data", name)) throw new Error(`データ「${name}」は既にあります`);
      const fields = (op.fields ?? []).filter((f: any) => (f.name ?? "").trim()).map((f: any) => `    ${okName(f.name, "項目の名前")}: ${okType(f.type, o)}`);
      if (fields.length === 0) throw new Error("項目を 1 つ以上書きます(例: 名前: 文字列)");
      return addDecl(o, ["data"], [`データ ${name}:`, "    ID: 識別子", ...fields]);
    }
    case "add_field": {
      const d = decl(o, "data", op.data);
      if (!d) throw new Error(`データ「${op.data}」がありません`);
      const name = okName(op.name, "項目の名前");
      if (d.fields.some((f: any) => f.name === name)) throw new Error(`「${op.data}」に項目「${name}」は既にあります`);
      return addLine(d, `${name}: ${okType(op.type, o)}`);
    }
    case "add_enum": {
      const name = okName(op.name, "列挙の名前");
      const values = (op.values ?? []).map((v: string) => okName(v, "値")).filter(Boolean);
      if (values.length === 0) throw new Error("値を 1 つ以上書きます");
      return addDecl(o, ["enum"], [`列挙 ${name}:`, ...values.map((v: string) => "    " + v)]);
    }
    case "add_screen": {
      const name = okName(op.name, "画面の名前");
      if (decl(o, "screen", name)) throw new Error(`画面「${name}」は既にあります`);
      const ls = (op.lines ?? []).map((l: string) => l.trim()).filter(Boolean);
      if (ls.length === 0) throw new Error("画面に出すものを 1 つ以上書きます(例: 注文を一覧表示)");
      return addDecl(o, ["screen"], [`画面 ${name}:`, ...ls.map((l: string) => "    " + l)]);
    }
    case "add_role": {
      const name = okName(op.name, "役割の名前");
      if (decl(o, "role", name)) throw new Error(`役割「${name}」は既にあります`);
      const caps = (op.caps ?? []).map((c: string) => c.trim()).filter(Boolean);
      if (caps.length === 0) throw new Error("権限を 1 つ以上選びます");
      return addDecl(o, ["role"], [`役割 ${name}:`, `    権限 = [${caps.join(", ")}]`]);
    }
    case "set_role_caps": {
      const d = decl(o, "role", op.name);
      if (!d) throw new Error(`役割「${op.name}」がありません`);
      const caps = (op.caps ?? []).map((c: string) => c.trim()).filter(Boolean);
      if (caps.length === 0) throw new Error("権限を 1 つ以上選びます");
      const ls = lines(d.file);
      ls.splice(d.line - 1, d.end - d.line + 1, `役割 ${d.name}:`, `    権限 = [${caps.join(", ")}]`);
      write(d.file, ls.join("\n"));
      return { file: d.file, line: d.line };
    }
    case "add_rule": {
      const d = decl(o, "rules");
      return d ? addLine(d, op.text) : addDecl(o, ["role", "screen"], ["約束:", "    " + op.text.trim()]);
    }
    case "add_action": {
      const name = okName(op.name, "動作の名前");
      if (decl(o, "action", name)) throw new Error(`動作「${name}」は既にあります`);
      const params = (op.params ?? []).filter((p: any) => (p.name ?? "").trim()).map((p: any) => `${okName(p.name, "引数の名前")}: ${okType(p.type, o)}`);
      const body = (op.body ?? []).map((l: string) => l.trim()).filter(Boolean);
      if (body.length === 0) throw new Error("処理を 1 行以上書きます");
      let fail = (op.fail ?? "").trim();
      const block: string[] = [];
      if (!fail) {
        fail = `${name}の失敗`;
        if (!o.enums?.[fail]) block.push(`列挙 ${fail}:`, "    不正", "");
      }
      block.push(`動作 ${name}(${params.join(", ")}) -> 結果<識別子, ${fail}>:`, ...body.map((l: string) => "    " + l));
      return addDecl(o, ["action"], block);
    }
    case "add_scenario": {
      const name = okName(op.name, "確かめるの名前");
      const ls = (op.lines ?? []).map((l: string) => l.trim()).filter(Boolean);
      if (ls.length === 0) throw new Error("確かめる手順を 1 行以上書きます");
      return addDecl(o, ["scenario"], [`確かめる ${name}:`, ...ls.map((l: string) => "    " + l)]);
    }
    case "add_line": {
      const d = (o.decls ?? []).find((x: any) => x.kind === op.kind && x.name === op.name);
      if (!d) throw new Error(`「${op.name}」が見つかりません`);
      return addLine(d, op.text);
    }
    case "replace_line":
    case "remove_line": {
      const ls = lines(op.file);
      if (ls[op.line - 1]?.trim() !== String(op.old ?? "").trim()) throw new Error("ソースが変わっています。もう一度読み込んでください");
      if (op.op === "remove_line") ls.splice(op.line - 1, 1);
      else ls[op.line - 1] = ls[op.line - 1].replace(/^(\s*).*$/, (_m, ind: string) => ind + String(op.text).trim());
      write(op.file, ls.join("\n"));
      return { file: op.file, line: op.line };
    }
    case "remove_decl": {
      const d = (o.decls ?? []).find((x: any) => x.kind === op.kind && x.name === op.name);
      if (!d) throw new Error(`「${op.name}」が見つかりません`);
      const ls = lines(d.file);
      let n = d.end - d.line + 1;
      if (ls[d.end] !== undefined && ls[d.end].trim() === "") n += 1;
      ls.splice(d.line - 1, n);
      write(d.file, ls.join("\n"));
      return null;
    }
    case "save_source": {
      write(op.file, String(op.text));
      return null;
    }
    case "set_theme": {
      const p = join(PROJECT, "kotonoha.json");
      const cfg = JSON.parse(readFileSync(p, "utf8"));
      const prev = JSON.stringify(cfg, null, 2) + "\n";
      cfg.theme = { color: String(op.color).trim() };
      writeFileSync(p, JSON.stringify(cfg, null, 2) + "\n");
      const chk = koto(["check"]);
      if (chk.code !== 0) {
        writeFileSync(p, prev);
        throw new Error((chk.err.split("\n").find((l) => l.includes("theme")) ?? "その色は使えません").replace(/^.*?エラー\[[A-Z0-9]+\]\s*/, ""));
      }
      return null;
    }
    default:
      throw new Error(`知らない操作です: ${op.op}`);
  }
}

// ---- 日本語の指示 ----
function similar(a: string, b: string): number {
  const bg = (s: string) => {
    const xs = new Set<string>();
    const cs = [...s];
    for (let i = 0; i < cs.length - 1; i++) xs.add(cs[i] + cs[i + 1]);
    return xs;
  };
  const x = bg(a), y = bg(b);
  let n = 0;
  for (const g of x) if (y.has(g)) n++;
  return (2 * n) / Math.max(1, x.size + y.size);
}
function splitList(s: string): string[] {
  return s.split(/[、,，・]/).map((x) => x.trim()).filter(Boolean);
}
/** 「名前: 文字列、価格(金額)、在庫数(数)」→ 項目の一覧 */
function parseFields(s: string): { name: string; type: string }[] {
  return splitList(s).map((x) => {
    const m = /^(.+?)\s*[:：]\s*(.+)$/.exec(x) ?? /^(.+?)[(（](.+?)[)）]$/.exec(x);
    return m ? { name: m[1].trim(), type: m[2].trim() } : { name: x, type: "文字列" };
  });
}
function instruct(text0: string, ctx: { screen?: string; scenario?: string }): any {
  const text = text0.trim().replace(/[。.]$/, "");
  if (!text) throw new Error("指示を書いてください");
  const o = outline();
  const cmd = (m: RegExpExecArray | null) => m;
  let m: RegExpExecArray | null;
  // 決まった言い方: 構成を変える操作
  if ((m = cmd(/^データ「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.+)[)）])?$/.exec(text)))) {
    const at = apply({ op: "add_data", name: m[1], fields: m[2] ? parseFields(m[2]) : [{ name: "名前", type: "文字列" }] });
    return { ok: true, done: `データ「${m[1]}」を作りました`, at };
  }
  if ((m = cmd(/^(.+?)に(?:項目)?「?(.+?)」?(?:[(（](.+?)[)）])?を(?:追加(?:する)?|足す|加える)$/.exec(text))) && decl(o, "data", m[1].trim())) {
    const at = apply({ op: "add_field", data: m[1].trim(), name: m[2], type: m[3] ?? "文字列" });
    return { ok: true, done: `「${m[1]}」に項目「${m[2]}」を足しました`, at };
  }
  if ((m = cmd(/^列挙「?(.+?)」?を(?:作る|作って|追加(?:する)?)[(（](.+)[)）]$/.exec(text)))) {
    const at = apply({ op: "add_enum", name: m[1], values: splitList(m[2]) });
    return { ok: true, done: `列挙「${m[1]}」を作りました`, at };
  }
  if ((m = cmd(/^画面「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.+)[)）])?$/.exec(text)))) {
    const ls = m[2] ? m[2].split(/[、;；]/).map((x) => x.trim()).filter(Boolean) : [];
    if (ls.length === 0) throw new Error(`画面に出すものも書いてください(例: 画面「${m[1]}」を作る(注文を新しい順に一覧表示))`);
    const at = apply({ op: "add_screen", name: m[1], lines: ls });
    return checked(at, `画面「${m[1]}」を作りました`, o, text);
  }
  if ((m = cmd(/^(?:色|テーマ色|テーマの色)を「?(.+?)」?に(?:する|変える)$/.exec(text)))) {
    apply({ op: "set_theme", color: m[1] });
    return { ok: true, done: `色を「${m[1]}」にしました` };
  }
  // 文: 約束・確かめる・画面
  if (/(閲覧可|重複不可|の順|はいつでも)/.test(text) || text.startsWith("約束")) {
    const at = apply({ op: "add_rule", text: text.replace(/^約束[::]?\s*/, "") });
    return checked(at, "約束に入れました", o, text);
  }
  if (/(→|->)/.test(text)) {
    const sc = (ctx.scenario && decl(o, "scenario", ctx.scenario)) || decl(o, "scenario");
    const at = sc ? apply({ op: "add_line", kind: "scenario", name: sc.name, text }) : apply({ op: "add_scenario", name: "動きの確認", lines: [text] });
    return checked(at, `確かめる「${sc?.name ?? "動きの確認"}」に入れました`, o, text);
  }
  const screens = (o.decls ?? []).filter((d: any) => d.kind === "screen").map((d: any) => d.name);
  const sc = ctx.screen && decl(o, "screen", ctx.screen);
  if (!sc) return { ok: false, need: "screen", screens, message: "どの画面に入れますか? 左の一覧で画面を選んでから、もう一度送ってください" };
  const at = apply({ op: "add_line", kind: "screen", name: sc.name, text });
  return checked(at, `画面「${sc.name}」に入れました`, o, text);
}
/** 入れた行に誤りがあれば元に戻して、理由と近い書き方を返す */
function checked(at: { file: string; line: number } | null, done: string, before: any, text: string): any {
  const o = outline();
  const bad = (o.diagnostics ?? []).filter((d: any) => d.severity === "error" && at && d.span?.line === at.line && d.file === at.file);
  if (bad.length === 0) return { ok: true, done, at };
  undo();
  const hints: string[] = bad.flatMap((d: any) => d.hints ?? []);
  const fromHints = hints.flatMap((h) => [...h.matchAll(/「([^」]{2,80})」|`([^`]{2,80})`/g)].map((x) => x[1] ?? x[2]));
  const pats = [...(before.patterns?.screen ?? []), ...(before.patterns?.rules ?? [])];
  // 書ける文のうち、書いた文に近いもの(文字の並びの重なりで比べる)
  const near = [...new Set(pats)].map((p: string) => ({ p, s: similar(text, p) })).sort((x, y) => y.s - x.s).slice(0, 5).map((x) => x.p);
  return { ok: false, errors: bad.map((d: any) => ({ code: d.code, message: d.message_ja, hints: d.hints ?? [] })), suggestions: [...new Set([...fromHints.filter((s) => !/[{}]/.test(s)), ...near])].slice(0, 6) };
}
function undo(): boolean {
  const h = history.pop();
  if (!h) return false;
  writeFileSync(srcPath(h.file), h.text);
  return true;
}

// ---- 見た目の確認(koto dev を動かしておく) ----
let previewPort = 0;
let preview: ChildProcess | null = null;
let devLog = "";
async function freePort(): Promise<number> {
  return new Promise((ok, ng) => {
    const s = netServer();
    s.once("error", ng);
    s.listen(0, "127.0.0.1", () => {
      const p = (s.address() as { port: number }).port;
      s.close(() => ok(p));
    });
  });
}
async function startPreview(): Promise<void> {
  previewPort = await freePort();
  preview = spawn(KOTO, ["-C", PROJECT, "dev", "--port", String(previewPort)], {
    env: { ...process.env, KOTONOHA_DEV_FRAME_ANCESTOR: `http://127.0.0.1:${PORT} http://localhost:${PORT}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const keep = (b: Buffer) => {
    devLog = (devLog + b.toString("utf8")).slice(-200_000);
  };
  preview.stdout?.on("data", keep);
  preview.stderr?.on("data", keep);
}
/** 最後に出た「確かめる」の結果(koto dev の出力から) */
function lastChecks(): { lines: string[]; ok: boolean | null } {
  const ends = [...devLog.matchAll(/すべて確かめました|\d+ 件が期待と違いました/g)];
  if (ends.length === 0) return { lines: [], ok: null };
  const last = ends[ends.length - 1];
  const prevEnd = ends.length > 1 ? (ends[ends.length - 2].index ?? 0) + ends[ends.length - 2][0].length : 0;
  const start = devLog.indexOf("確かめる「", prevEnd);
  const block = devLog.slice(start < 0 ? prevEnd : start, (last.index ?? 0) + last[0].length);
  return { lines: block.split("\n").map((l) => l.replace(/\s+$/, "")).filter(Boolean).slice(0, 200), ok: last[0] === "すべて確かめました" };
}

// ---- HTTP ----
const STATIC: Record<string, [string, string]> = {
  "/": ["index.html", "text/html; charset=utf-8"],
  "/studio.js": ["studio.js", "text/javascript; charset=utf-8"],
  "/studio.css": ["studio.css", "text/css; charset=utf-8"],
};
const CSP = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-src http://127.0.0.1:* http://localhost:*; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'";
function send(res: ServerResponse, status: number, body: unknown, type = "application/json; charset=utf-8"): void {
  res.writeHead(status, { "Content-Type": type, "Content-Security-Policy": CSP, "X-Content-Type-Options": "nosniff", "X-Frame-Options": "DENY", "Referrer-Policy": "no-referrer", "Cache-Control": "no-store" });
  res.end(typeof body === "string" ? body : JSON.stringify(body));
}
async function body(req: IncomingMessage): Promise<any> {
  const chunks: Buffer[] = [];
  let n = 0;
  for await (const c of req) {
    n += (c as Buffer).length;
    if (n > 2 * 1024 * 1024) throw new Error("大きすぎます");
    chunks.push(c as Buffer);
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}
function state(): any {
  const o = outline();
  const sources: Record<string, string> = {};
  for (const f of o.files ?? []) {
    try {
      sources[f] = read(f);
    } catch {}
  }
  let theme = "";
  try {
    theme = JSON.parse(readFileSync(join(PROJECT, "kotonoha.json"), "utf8")).theme?.color ?? "";
  } catch {}
  return { outline: o, sources, previewPort, checks: lastChecks(), canUndo: history.length > 0, theme };
}

const server = createServer(async (req, res) => {
  try {
    if (!HOSTS.has(req.headers.host ?? "")) return send(res, 421, { error: "host" });
    const url = new URL(req.url ?? "/", `http://${req.headers.host}`);
    if (req.method === "GET" && STATIC[url.pathname]) {
      const [f, type] = STATIC[url.pathname];
      let text = readFileSync(join(here, f), "utf8");
      if (f === "index.html") text = text.replace("__TOKEN__", TOKEN);
      return send(res, 200, text, type);
    }
    if (req.method === "GET" && url.pathname === "/api/state") return send(res, 200, state());
    if (req.method === "POST" && url.pathname.startsWith("/api/")) {
      // 他のサイトからの書き込みを防ぐ: 合言葉のヘッダと JSON を求める
      if (req.headers["x-studio-token"] !== TOKEN || !String(req.headers["content-type"] ?? "").startsWith("application/json")) return send(res, 403, { error: "forbidden" });
      const b = await body(req);
      try {
        if (url.pathname === "/api/op") {
          const at = apply(b);
          return send(res, 200, { ok: true, at, ...state() });
        }
        if (url.pathname === "/api/instruct") {
          const r = instruct(String(b.text ?? ""), { screen: b.screen, scenario: b.scenario });
          return send(res, 200, { result: r, ...state() });
        }
        if (url.pathname === "/api/undo") {
          const ok = undo();
          return send(res, 200, { ok, ...state() });
        }
      } catch (e) {
        return send(res, 200, { ok: false, error: String((e as Error).message ?? e), ...state() });
      }
    }
    send(res, 404, { error: "not found" });
  } catch (e) {
    send(res, 500, { error: String((e as Error)?.message ?? e) });
  }
});

const parent = Number(process.env.KOTONOHA_EXIT_WITH_PARENT ?? 0);
const stop = () => {
  try {
    preview?.kill();
  } catch {}
  process.exit(0);
};
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
if (parent > 0) {
  setInterval(() => {
    let alive = process.ppid === parent;
    try {
      process.kill(parent, 0);
    } catch {
      alive = false;
    }
    if (!alive) stop();
  }, 500).unref();
}
if (existsSync(join(PROJECT, "kotonoha.json"))) await startPreview();
server.listen(PORT, "127.0.0.1", () => {
  console.error(`KOTONOHA Studio: http://127.0.0.1:${PORT}/   (止めるとき: Ctrl+C)`);
});
