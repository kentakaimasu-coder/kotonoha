// KOTONOHA Studio(ADR-0135): 画面でアプリを作るためのローカルサーバー。`koto studio` が起動する。
// - ソース(src/*.koto)が正。画面での操作は、すべてソースの行の追加・書き換えとして行い、koto outline で検査し直す
// - 日本語の指示は、決まった言い方(データを作る・項目を追加する など)なら操作に、それ以外は文としてソースに入れ、
//   コンパイラが読めなければ元に戻して「近い書き方」を返す(AI は使わない。解釈の判断はコンパイラだけ)
// - 127.0.0.1 だけで待ち受け、Host を確かめ、書き込みには起動時に作った合言葉(ヘッダ)を求める(他のサイトから操作させない)
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, realpathSync, readdirSync, appendFileSync, renameSync, utimesSync } from "node:fs";
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
  const r = spawnSync(KOTO, ["-C", PROJECT, ...args], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024, env: { ...process.env, KOTONOHA_USAGE_LOG: "" } });
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

// ---- 操作の記録(評価用。KOTONOHA_USAGE_LOG があるときだけ) ----
const USAGE = process.env.KOTONOHA_USAGE_LOG ?? "";
function usage(v: Record<string, unknown>): void {
  if (!USAGE) return;
  try {
    appendFileSync(USAGE, JSON.stringify({ kind: "studio", t: Math.floor(Date.now() / 1000), ...v }) + "\n");
  } catch {}
}
/** プロジェクトがまだ無い(「新しいアプリを作る」から始める) */
const isHome = () => !existsSync(join(PROJECT, "kotonoha.json"));
const TEMPLATES: { name: string; desc: string }[] = (() => {
  try {
    return JSON.parse(process.env.KOTONOHA_TEMPLATES ?? "[]");
  } catch {
    return [];
  }
})();

// ---- ソースの読み書き(プロジェクトの中の .koto だけ) ----
function srcPath(file: string): string {
  const p = resolve(PROJECT, file);
  if (!(p.startsWith(PROJECT + sep) && p.endsWith(".koto"))) throw new Error("プロジェクトの外のファイルは扱えません");
  return p;
}
function mainFile(o: any): string {
  return o.files?.[0] ?? "src/app.koto";
}
// 取り消しは「1 回の指示・操作」単位(名前の変更などは複数のファイルを書き換えるので、まとめて戻す)
const history: { file: string; text: string; g: number; raw?: boolean }[] = [];
let group = 0;
function read(file: string): string {
  return readFileSync(srcPath(file), "utf8");
}
/** 書きかけのファイルを koto dev などが読まないよう、別の名前に書いてから置き換える */
function atomicWrite(p: string, text: string): void {
  const tmp = `${p}.${process.pid}.tmp`;
  writeFileSync(tmp, text);
  renameSync(tmp, p);
}
/** これまでに変えた名前(前の名前 → 新しい名前)。前の名前で指示されたときの読み替えに使う */
const renamedWords: [string, string][] = [];
/** 最後にソース・初期データを書き換えた時刻(確かめるの結果が古いかどうかの判断に使う) */
let wroteTime = 0;
function write(file: string, text: string): void {
  wroteTime = Date.now();
  queueMicrotask(retryAfterFix);
  history.push({ file, text: read(file), g: group });
  if (history.length > 300) history.shift();
  atomicWrite(srcPath(file), text.endsWith("\n") ? text : text + "\n");
}
/** 初期データ(seed*.json)の書き換え。プロジェクト直下の JSON だけ */
function seedFiles(): string[] {
  try {
    return readdirSync(PROJECT).filter((f) => /^seed.*\.json$/.test(f));
  } catch {
    return [];
  }
}
function writeRaw(file: string, text: string): void {
  wroteTime = Date.now();
  queueMicrotask(retryAfterFix);
  const p = join(PROJECT, file);
  history.push({ file: p, text: readFileSync(p, "utf8"), g: group, raw: true });
  atomicWrite(p, text);
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
  // 数字で始まる名前は書けない(「1日の数」が「厳密層」の分かりにくい誤りになった。第 5 回評価)
  if (/^[0-9０-９]/.test(t)) throw new Error(`${what}「${t}」は数字で始められません(例: 「一日の数」「1日の数」→「日ごとの数」)`);
  if (!NAME.test(t)) throw new Error(`${what}「${t}」は使えません(空白や記号を含まない名前にします)`);
  return t;
}
const TYPE_WORDS: Record<string, string> = {
  文字: "文字列", 文字列: "文字列", 文章: "文字列", テキスト: "文字列", 名前: "文字列",
  数: "整数", 数字: "整数", 整数: "整数", 個数: "整数", 人数: "整数",
  真偽: "真偽", "はい/いいえ": "真偽", はいいいえ: "真偽", チェック: "真偽",
  日付: "日付", 日: "日付", 日時: "日時", 時刻: "日時",
  金額: "金額", お金: "金額", 円: "金額", 価格: "金額", "金額<JPY>": "金額<JPY>",
  メール: "メール", メールアドレス: "メール", URL: "URL", 識別子: "識別子", ID: "識別子",
};
/** 型を書かなかった項目は、名前から型を決める(「物件ID」が文字列になった。第 5 回評価) */
function guessType(name: string, o: any): string {
  const n = name.trim();
  const ref = n.endsWith("ID") ? n.slice(0, -2) : "";
  if (n === "ID" || (ref && (o.decls ?? []).some((d: any) => d.kind === "data" && d.name === ref))) return "識別子";
  if (/日時$|時刻$/.test(n)) return "日時";
  if (/日$|日付$|期限$/.test(n)) return "日付";
  if (/(料金|金額|価格|単価|値段|家賃|代金|月謝|謝礼|会費|料$|費$|額$)/.test(n)) return "金額";
  if (/^(.{1,4}中|.+済み?|.+の有無|.+可否|有効|公開)$/.test(n) && !/(期間|時間|日時|日)$/.test(n)) return "真偽";
  if (/(納期|期日|締切|締め切り|予定日|誕生日)$/.test(n)) return "日付";
  // (電話番号・営業時間は文字のまま)
  if (/(人数|数$|数量|定員|回数|点数|年齢|順番|積載|重量|距離|作業時間|所要時間|勤務時間|在庫|件数)/.test(n)) return "整数";
  // 操作をした人(連絡者・予約者・記入者など)は、ログインした人の ID(連絡者が文字の欄になった。第 9 回評価)
  if (/(予約|申込|記入|送信|作成|登録|注文|依頼|連絡|投稿|報告|記録|受付|回答|訪問|発行|発券|入力|提出|申請|発注|購入|相談|応募|点検|貸出|借用)者$/.test(n) || /^(借りた人|借り手|借主|利用者|使用者)$/.test(n)) return "識別子";
  if (/メール/.test(n)) return "メール";
  if (o.enums && n in o.enums) return n;
  // 「状態」を足すとき、列挙「予約状態」「状態」があればそれにする(文字列になって登録が誤りになった。第 6 回評価)
  const en = Object.keys(o.enums ?? {}).filter((e) => e.endsWith(n) || n.endsWith(e));
  if (en.length === 1) return en[0];
  return "文字列";
}
function okType(t: string, o: any, name?: string): string {
  let s = (t ?? "").trim() || (name ? guessType(name, o) : "文字列");
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
  // 動作に「…を確認 または 失敗("X")」を足すとき: 登録・返す行より前に入れ、失敗を返せる動作にする
  // (後ろに入り、「結果<T, E> の動作でのみ使えます」の意味が分からなかった。第 9 回評価)
  if (d.kind === "action" && /を確認\s*または\s*失敗\(/.test(text)) {
    const vals = [...text.matchAll(/失敗\(\s*"([^"]+)"\s*\)/g)].map((m) => m[1]);
    let ls = lines(d.file);
    const head = ls[d.line - 1];
    const ret = /->\s*(.+?)\s*:\s*$/.exec(head)?.[1]?.trim();
    const res = ret && /^結果<\s*(.+?)\s*,\s*(.+?)\s*>$/.exec(ret);
    if (res) {
      // 失敗の列挙に、まだ無い値を足す
      const E = decl(outline(), "enum", res[2]);
      for (const v of vals) if (E && !(E.values ?? []).includes(v)) addLine(decl(outline(), "enum", res[2]), v);
    } else {
      const en = `${d.name}の失敗`;
      if (!decl(outline(), "enum", en)) apply({ op: "add_enum", name: en, values: vals.length ? vals : ["できない"] });
      const ok = ret || "識別子";
      const D2 = decl(outline(), "action", d.name);
      ls = lines(D2.file);
      ls[D2.line - 1] = ls[D2.line - 1].replace(/\s*(->\s*.+?)?\s*:\s*$/, ` -> 結果<${ok}, ${en}>:`);
      write(D2.file, ls.join("\n"));
    }
    const D3 = decl(outline(), "action", d.name);
    ls = lines(D3.file);
    let k = D3.line;
    while (k < D3.end && /^\s+\S+\s*=\s*\S+を取得\(/.test(ls[k] ?? "")) k++;
    insertAfter(D3.file, k, ["    " + text.trim()]);
    return { file: D3.file, line: k + 1 };
  }
  // 同じ動作のフォームの行(「注文するのフォームを表示」)が既にあれば、置き換える(2 つ並んだ。第 10 回評価)
  const fm = /^(.+?)のフォームを/.exec(text.trim());
  if (d.kind === "screen" && fm) {
    const ls = lines(d.file);
    for (let k = d.line; k < d.end; k++) {
      if (new RegExp(`^\\s+${reEsc(fm[1])}のフォームを`).test(ls[k] ?? "") && ls[k].trim() !== text.trim()) {
        ls[k] = ls[k].replace(/^(\s+).*$/, `$1${text.trim()}`);
        write(d.file, ls.join("\n"));
        return { file: d.file, line: k + 1 };
      }
    }
  }
  // 画面の「…を表示」は、ボタン(「…」を押すと)より上に入れる(ボタンの下に入って並べ直した。第 7 回評価)
  if (d.kind === "screen" && /を表示$/.test(text.trim()) && !/フォーム/.test(text)) {
    const ls = lines(d.file);
    for (let k = d.line; k < d.end; k++)
      if (/^\s+「.+」を押すと/.test(ls[k] ?? "")) {
        insertAfter(d.file, k, ["    " + text.trim()]);
        return { file: d.file, line: k + 1 };
      }
  }
  // 動作の処理の行は、最後の「…を返す」より前に入れる(後ろに入り、どこが誤りか分からなかった。第 9 回評価)
  if (d.kind === "action" && !/を返す\s*$/.test(text.trim())) {
    const ls = lines(d.file);
    for (let k = d.end - 1; k >= d.line; k--) {
      if (/^\s+.*を返す\s*$/.test(ls[k] ?? "")) {
        insertAfter(d.file, k, ["    " + text.trim()]);
        return { file: d.file, line: k + 1 };
      }
    }
  }
  insertAfter(d.file, d.end, ["    " + text.trim()]);
  return { file: d.file, line: d.end + 1 };
}

// ---- 名前の変更・まとめて消す・空でもよい項目(第 5 回評価で多かった要望) ----
const reEsc = (x: string) => x.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
// 項目名の前後に来てよい文字(日本語は空白で区切らないので、助詞・記号で区切りを見る)
const FIELD_BEFORE = "(?<=^|[\\s.(,、「のとはが\\[])";
const FIELD_AFTER = "(?=$|[\\s:：)\\],、.」をがのはでにと順ご+\\-*/=<>!])";
function srcFiles(o: any): string[] {
  return [...new Set<string>([...(o.files ?? []), ...(o.decls ?? []).map((x: any) => x.file)].filter(Boolean))];
}
/** 宣言(データ・画面・動作・役割・列挙・確かめる)の名前を、使っている所ごと変える。初期データの名前もそろえる */
function renameDecl(o: any, from: string, to: string): string {
  const d = (o.decls ?? []).find((x: any) => x.name === from && ["data", "screen", "action", "role", "enum", "scenario"].includes(x.kind));
  if (!d) throw new Error(`「${from}」という名前のデータ・画面・動作・役割・列挙はありません`);
  okName(to, "新しい名前");
  if ((o.decls ?? []).some((x: any) => x.name === to)) throw new Error(`「${to}」は既にあります`);
  // 直後がひらがな(助詞を除く)なら変えない(「問い合わせる」→「お迎え連絡る」になった。第 6 回評価)
  const re = new RegExp(reEsc(from) + "(?!(?![をのはがにでとへかもや])[ぁ-ゖ])", "g");
  for (const f of srcFiles(o)) {
    const t = read(f);
    // 注釈(# …)の行は変えない
    const u = t.split("\n").map((l) => (l.trim().startsWith("#") ? l : l.replace(re, to))).join("\n");
    if (u !== t) write(f, u);
  }
  if (d.kind === "data") renameSeed(from, to);
  return d.kind;
}
/** 初期データの JSON のキーを変える(データ名・データ名ID・項目名) */
function renameSeed(from: string, to: string, onlyData?: string): void {
  for (const f of seedFiles()) {
    let j: any;
    try {
      j = JSON.parse(readFileSync(join(PROJECT, f), "utf8"));
    } catch {
      continue;
    }
    if (!j || typeof j !== "object" || Array.isArray(j)) continue;
    const ren = (obj: any, k1: string, k2: string) => {
      if (obj && typeof obj === "object" && !Array.isArray(obj) && k1 in obj && !(k2 in obj)) {
        const out: any = {};
        for (const [k, v] of Object.entries(obj)) out[k === k1 ? k2 : k] = v;
        return out;
      }
      return obj;
    };
    let out: any = {};
    for (const [k, v] of Object.entries(j)) {
      const rows = Array.isArray(v) ? v : null;
      if (onlyData !== undefined) {
        // 項目の名前の変更: そのデータの行だけ
        out[k] = k === onlyData && rows ? rows.map((r) => ren(r, from, to)) : v;
      } else {
        // データの名前の変更: ソースと同じく、キーの中の名前をすべて変える(「予約数」→「申込数」も)
        const sub = (x: string) => x.split(from).join(to);
        out[sub(k)] = rows ? rows.map((r) => (r && typeof r === "object" && !Array.isArray(r) ? Object.fromEntries(Object.entries(r).map(([a, b]) => [sub(a), b])) : r)) : v;
      }
    }
    out = JSON.stringify(out, null, 2) + "\n";
    if (out !== readFileSync(join(PROJECT, f), "utf8")) writeRaw(f, out);
  }
}
/** データの項目の名前を変える(区切りを見て、その項目として使っている所だけ) */
function renameField(o: any, dn: string, from: string, to: string): void {
  const D = decl(o, "data", dn);
  if (!D) throw new Error(`データ「${dn}」はありません`);
  if (!D.fields.some((f: any) => f.name === from) && !(D.computed ?? []).some((f: any) => f.name === from)) throw new Error(`データ「${dn}」に項目「${from}」はありません(項目: ${D.fields.map((x: any) => x.name).join("、")})`);
  okName(to, "新しい名前");
  if (D.fields.some((f: any) => f.name === to)) throw new Error(`「${dn}」に項目「${to}」は既にあります`);
  const re = new RegExp(FIELD_BEFORE + reEsc(from) + FIELD_AFTER, "gu");
  for (const f of srcFiles(o)) {
    const t = read(f);
    // ほかのデータの同じ名前の項目(「名前」など)は変えない(全部変わって誤りになり、戻された。第 6 回評価)。
    // 変える所: そのデータの宣言の中、動作の中で v = データを取得(…) とした v の v.項目、データを登録(項目: …) の項目名、
    // そのデータの名前を含む画面・約束・確かめるの文。注釈・文字列("…")の中は変えない
    let kind = "";
    let block = "";
    let vars: string[] = [];
    const word = (l: string) => l.split('"').map((part, i) => (i % 2 === 1 ? part : part.replace(re, to))).join('"');
    // そのデータを登録する動作で、項目と同じ名前の引数(フォームの欄)も変える(欄の見出しが古い名前のままだった。第 7 回評価)
    const fullActs = new Set(
      (o.decls ?? [])
        .filter((a: any) => a.kind === "action" && a.file === f && (a.params ?? []).some((p: any) => p.name === from))
        .filter((a: any) => t.split("\n").slice(a.line - 1, a.end).join("\n").includes(`${dn}を登録(`))
        .map((a: any) => a.name),
    );
    const u = t
      .split("\n")
      .map((l) => {
        const tt = l.trim();
        if (/^\S/.test(l) && !tt.startsWith("#")) {
          [kind, block] = [tt.split(" ")[0], (tt.split(" ")[1] ?? "").split(/[:(\s]/)[0]];
          vars = [];
        }
        if (!tt || tt.startsWith("#")) return l;
        // 「日付: 日付」のように型と同じ名前の項目は、名前の側だけを変える(型まで変わり「型 訪問日 は定義されていません」になった。第 10 回評価)
        if (kind === "データ") {
          if (block !== dn) return l;
          const fm = /^(\s+)([^:=\s]+)(\s*:.*)$/.exec(l);
          return fm ? fm[1] + (fm[2] === from ? to : fm[2]) + fm[3] : word(l);
        }
        if (kind === "列挙" || kind === "役割") return l;
        if (kind === "動作" && fullActs.has(block)) {
          // 見出しの行(動作 名前(引数: 型)): 引数の名前だけを変える
          if (/^\S/.test(l)) return l.replace(new RegExp(`([(,]\\s*)${reEsc(from)}(\\s*:)`, "g"), `$1${to}$2`);
          return word(l);
        }
        if (kind === "動作" || kind === "関数") {
          if (!/^\s/.test(l)) return l;
          const mv = new RegExp(`^\\s*(\\S+)\\s*=\\s*${reEsc(dn)}を取得\\(`).exec(l);
          if (mv) vars.push(mv[1]);
          let x = l;
          for (const v of vars) x = x.split(`${v}.${from}`).join(`${v}.${to}`);
          if (x.includes(`${dn}を登録(`)) x = x.replace(new RegExp(`([(,]\\s*)${reEsc(from)}(\\s*:)`, "g"), `$1${to}$2`);
          return x;
        }
        return l.includes(dn) ? word(l) : l;
      })
      .join("\n");
    if (u !== t) write(f, u);
  }
  renameSeed(from, to, dn);
}
/** データを消す: そのデータを使う動作・確かめる、画面と約束の行、役割の権限、ほかのデータの「名前ID」項目もまとめて消す */
function removeDataCascade(o: any, name: string): string[] {
  const removed: string[] = [];
  const dead = new Set<string>([name]);
  // 「予約」を消すとき「予約数」「予約失敗」まで巻き込まないよう、語の区切りを見る(予約を…/予約の/予約ID/予約./予約( など)
  const ref = (n: string) => new RegExp(`(?<=^|[\\s.(,、「のとはが<=\\[])${reEsc(n)}(?=ID|[をのはがかへ(.\\s:：、,)\\]>」]|$)`, "mu");
  const uses = (t: string) => [...dead].some((n) => ref(n).test(t));
  const text = (d: any) => lines(d.file).slice(d.line - 1, d.end).join("\n");
  // 動作・確かめる・列挙(消すものだけが使う)を、増えなくなるまで集める
  const kill = new Set<any>([decl(o, "data", name)]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const d of o.decls ?? []) {
      if (kill.has(d) || !["action", "scenario"].includes(d.kind)) continue;
      if (uses(text(d))) {
        kill.add(d);
        dead.add(d.name);
        grew = true;
      }
    }
  }
  const usedElsewhere = (en: string) => (o.decls ?? []).some((d: any) => !kill.has(d) && d.name !== en && ref(en).test(text(d)));
  for (const d of o.decls ?? []) {
    // 消す動作の失敗の種類(-> 結果<…, 列挙>)だけを消す。自分で作った列挙(焼き方など)は残す(第 6 回評価)
    const failEnum = [...kill].some((k) => k.kind === "action" && new RegExp(`結果<[^>]*,\\s*${reEsc(d.name)}\\s*>`).test(lines(k.file)[k.line - 1] ?? ""));
    if (d.kind === "enum" && !kill.has(d) && failEnum && !usedElsewhere(d.name)) kill.add(d);
  }
  // ファイルごとに、後ろの行から消す
  const byFile = new Map<string, { from: number; to: number }[]>();
  const cut = (file: string, from: number, to: number) => byFile.set(file, [...(byFile.get(file) ?? []), { from, to }]);
  for (const d of kill) {
    cut(d.file, d.line, d.end);
    removed.push(`${({ data: "データ", action: "動作", scenario: "確かめる", enum: "列挙" } as any)[d.kind] ?? ""}「${d.name}」`);
  }
  for (const d of o.decls ?? []) {
    if (kill.has(d)) continue;
    const ls = lines(d.file);
    if (d.kind === "screen" || d.kind === "rules") {
      const body = ls.slice(d.line, d.end).map((l, i) => ({ l, n: d.line + 1 + i })).filter((x) => x.l.trim() && !x.l.trim().startsWith("#"));
      const bad = body.filter((x) => uses(x.l));
      if (bad.length && bad.length === body.length) {
        cut(d.file, d.line, d.end);
        removed.push(`${d.kind === "screen" ? "画面" : ""}「${d.name ?? "約束"}」`);
      } else bad.forEach((x) => cut(d.file, x.n, x.n));
    } else if (d.kind === "data") {
      // ほかのデータの「名前ID」項目
      ls.slice(d.line, d.end).forEach((l, i) => {
        if (new RegExp(`^\\s*${reEsc(name)}ID\\s*:`).test(l)) {
          cut(d.file, d.line + 1 + i, d.line + 1 + i);
          removed.push(`「${d.name}」の項目「${name}ID」`);
        }
      });
    }
  }
  for (const [file, cs] of byFile) {
    const ls = lines(file);
    const drop = new Set<number>();
    for (const c of cs) for (let n = c.from; n <= c.to; n++) drop.add(n);
    let out = ls.filter((_l, i) => !drop.has(i + 1));
    // 役割の権限から消したものを外す(全部外れたら役割ごと消す)
    out = out.map((l) =>
      /^\s*権限\s*=\s*\[/.test(l)
        ? l.replace(/\[(.*)\]/, (_m, xs: string) => "[" + xs.split(",").map((x) => x.trim()).filter((x) => x && ![...dead].some((n) => x === `動作.${n}` || x.startsWith(`${n}.`) || x.endsWith(`.${n}`))).join(", ") + "]")
        : l,
    );
    // 空になった権限の役割を消す・空行が 3 つ以上続くのを 1 つにする
    const res: string[] = [];
    for (let i = 0; i < out.length; i++) {
      if (/^役割 /.test(out[i]) && /^\s*権限\s*=\s*\[\s*\]/.test(out[i + 1] ?? "")) {
        removed.push(`役割「${out[i].replace(/^役割\s+|:\s*$/g, "")}」`);
        i++;
        continue;
      }
      if (out[i].trim() === "" && (res[res.length - 1] ?? "x").trim() === "") continue;
      res.push(out[i]);
    }
    write(file, res.join("\n"));
  }
  if (seedFiles().length) {
    for (const f of seedFiles()) {
      try {
        const j = JSON.parse(readFileSync(join(PROJECT, f), "utf8"));
        if (j && typeof j === "object" && name in j) {
          delete j[name];
          writeRaw(f, JSON.stringify(j, null, 2) + "\n");
        }
      } catch {}
    }
  }
  return removed;
}
/** 項目を空でもよい(選択<…>)にする。その項目を登録する動作の引数もそろえる(フォームで「任意」になる) */
function makeOptional(o: any, dn: string | undefined, fn: string): string[] {
  const ds = (o.decls ?? []).filter((d: any) => d.kind === "data" && (!dn || d.name === dn) && d.fields.some((f: any) => f.name === fn));
  if (ds.length === 0) throw new Error(`項目「${fn}」は${dn ? `データ「${dn}」に` : "どのデータにも"}ありません`);
  const done: string[] = [];
  for (const D of ds) {
    const f = D.fields.find((x: any) => x.name === fn);
    if (/^選択</.test(f.type)) continue;
    const ls = lines(D.file);
    const i = ls.findIndex((l, k) => k >= D.line && k < D.end && new RegExp(`^\\s*${reEsc(fn)}\\s*:`).test(l));
    if (i < 0) continue;
    ls[i] = ls[i].replace(/:\s*(.+?)\s*$/, `: 選択<${f.type}>`);
    write(D.file, ls.join("\n"));
    done.push(`${D.name}の${fn}`);
    // 「D を登録(… fn: fn …)」を含む動作の引数 fn も 選択<…> に
    const o2 = outline();
    for (const a of (o2.decls ?? []).filter((x: any) => x.kind === "action")) {
      const al = lines(a.file);
      const body = al.slice(a.line - 1, a.end).join("\n");
      if (!body.includes(`${D.name}を登録(`)) continue;
      const head = al[a.line - 1];
      const nh = head.replace(new RegExp(`([(,]\\s*${reEsc(fn)}\\s*:\\s*)(${reEsc(f.type)})(?=\\s*[,)])`), `$1選択<$2>`);
      if (nh !== head) {
        al[a.line - 1] = nh;
        // 「メモ != "" を確認 または 失敗(…)」のように、空でないことを確かめる行は、空でもよくなったので外す
        // (残すと「文字列 が必要ですが 選択<文字列> です」になり、意味が分からなかった。第 9 回評価)
        for (let k = a.end - 1; k >= a.line; k--)
          if (new RegExp(`^\\s*(?:${reEsc(fn)}|長さ\\(\\s*${reEsc(fn)}\\s*\\))\\s*(!=|==|>|>=)\\s*(""|0|1)\\s*を確認`).test(al[k] ?? "")) al.splice(k, 1);
        write(a.file, al.join("\n"));
      }
    }
  }
  return done;
}
/** 動作をある役割だけにしたとき、確かめるでその動作を役割の無い人が呼んでいる行を探す
 *  (役割の無い人のままで 403 になり、✗ の理由が分からなかった。第 7 回評価)。ソースは変えずに、直し方を伝える */
/** 役割の権限では使えない動作(ボタン・フォームが出ない)を、動作の中の文から見積もって知らせる
 *  (役割を作ると見るだけになり、ボタンが黙って出なかった。第 10 回評価で 4 人) */
function roleGapNote(roleName: string): string {
  const o = outline();
  const r = decl(o, "role", roleName);
  if (!r) return "";
  const caps: string[] = r.caps ?? [];
  const gaps: string[] = [];
  for (const a of (o.decls ?? []).filter((x: any) => x.kind === "action")) {
    if (caps.includes(`動作.${a.name}`)) continue;
    const need = new Set<string>();
    const vars = new Map<string, string>();
    for (const l of lines(a.file).slice(a.line, a.end)) {
      let mm = /^\s*(\S+)\s*=\s*(\S+?)を取得\(/.exec(l);
      if (mm) vars.set(mm[1], mm[2]);
      for (const g of l.matchAll(/(\S+?)を登録\(/g)) need.add(`${g[1].replace(/^.*[\s(=]/, "")}.作成`);
      for (const g of l.matchAll(/(\S+?)を削除\(/g)) need.add(`${g[1].replace(/^.*[\s(=]/, "")}.削除`);
      mm = /^\s*(\S+)\.\S+\s*=[^=]/.exec(l);
      if (mm && vars.has(mm[1])) need.add(`${vars.get(mm[1])}.更新`);
    }
    const lack = [...need].filter((c) => decl(o, "data", c.split(".")[0]) && !caps.includes(c));
    if (lack.length) gaps.push(`${a.name}(${lack.join("・")} が要る)`);
  }
  return gaps.length ? `役割「${r.name}」の人には、次の動作のボタン・フォームが出ません: ${gaps.slice(0, 4).join("、")}${gaps.length > 4 ? " ほか" : ""}。使わせるときは権限にチェックを付けます` : "";
}
function roleLessCallers(act: string): string[] {
  const o = outline();
  const out = new Set<string>();
  for (const sc of (o.decls ?? []).filter((d: any) => d.kind === "scenario")) {
    const ls = lines(sc.file);
    for (let k = sc.line; k < sc.end; k++) {
      const mm = new RegExp(`^\\s+([^\\s(（「」]+?)が\\s*${reEsc(act)}\\(`).exec(ls[k] ?? "");
      if (!mm || /(→|->)\s*権限がない/.test(ls[k]) || /ログインしていない/.test(mm[1])) continue;
      if (ls.slice(sc.line, sc.end).some((l) => new RegExp(`^\\s+${reEsc(mm[1])}[(（]`).test(l))) continue;
      out.add(mm[1]);
    }
  }
  return [...out];
}
/** 項目を足したら、そのデータの詳細を出している画面の「Dの A と B を表示」にも足す
 *  (足した項目が画面に出ず、表示の行を足すと同じ内容が 2 回出た。第 8 回評価) */
function showOnDetailScreens(dn: string, fn: string): void {
  const o = outline();
  const re = new RegExp(`^(\\s+)${reEsc(dn)}の(.+)を表示\\s*$`);
  for (const sc of (o.decls ?? []).filter((d: any) => d.kind === "screen")) {
    const ls = lines(sc.file);
    let touched = false;
    for (let k = sc.line; k < sc.end; k++) {
      const mm = re.exec(ls[k] ?? "");
      if (!mm || /フォーム/.test(mm[2])) continue;
      const parts = mm[2].split("と").map((x) => x.trim());
      if (parts.includes(fn)) continue;
      ls[k] = `${mm[1]}${dn}の${[...parts, fn].join("と")}を表示`;
      touched = true;
    }
    if (touched) write(sc.file, ls.join("\n"));
  }
}
/** 確かめるの呼び出しに入れる、型に合った仮の値 */
function sampleValue(o: any, type: string, name = ""): string {
  // 何の値か分かるように、文字は「…の例」、数は 1(0 だと「> 0 を確認」で止まる)。第 10 回評価: "" や 0 の意味が分からなかった
  return /^(整数|金額)/.test(type) ? "1" : type === "真偽" ? "偽" : type === "日時" ? "2026-12-01 10:00" : type === "日付" ? "2026-12-01" : o.enums?.[type] ? `"${o.enums[type][0]}"` : type === "メール" ? '"taro@example.com"' : type === "URL" ? '"https://example.com/"' : name ? `"${name}の例"` : '""';
}
/** 項目を足したとき、そのデータを名前付きで登録している動作に引数を足す(足さないと「引数がありません」で止まった。第 5 回評価) */
function extendRegistrations(dn: string, fn: string, type: string): string[] {
  const o = outline();
  const out: string[] = [];
  for (const a of (o.decls ?? []).filter((x: any) => x.kind === "action")) {
    const al = lines(a.file);
    const idx = al.findIndex((l, k) => k >= a.line && k < a.end && l.includes(`${dn}を登録(`) && /\w*[:：]/.test(l.split(`${dn}を登録(`)[1] ?? ""));
    if (idx < 0) continue;
    // ボタンが位置で呼んでいる動作は、引数を足すとボタンが壊れるので触らない
    const called = srcFiles(o).some((f) => read(f).includes(`を押すと ${a.name}(`));
    if (new RegExp(`[(,]\\s*${reEsc(fn)}\\s*:`).test(al[a.line - 1])) continue;
    // 後から足した「予約者」「登録日時」なども、作ったときと同じく自動で入れる(入力の欄になった。第 10 回評価)
    const auto = autoValue(o, fn, type);
    if (auto) {
      al[idx] = addParam(al[idx], `${fn}: ${auto}`, al[idx].indexOf(`${dn}を登録(`));
      write(a.file, al.join("\n"));
      out.push(a.name);
      continue;
    }
    if (called) continue;
    // 足した項目は登録の最後に足す(先頭に入って読みにくかった。第 8 回評価)
    al[idx] = addParam(al[idx], `${fn}: ${fn}`, al[idx].indexOf(`${dn}を登録(`));
    al[a.line - 1] = addParam(al[a.line - 1], `${fn}: ${type}`);
    write(a.file, al.join("\n"));
    out.push(a.name);
    // 確かめるの中の呼び出し(位置で渡している)にも、型に合った値を足す
    const sample = sampleValue(o, type, fn);
    for (const f of srcFiles(o)) {
      const t = read(f);
      const u = t
        .split("\n")
        .map((l) => {
          if (!/(→|->)/.test(l) || /^\s*(動作|関数|アダプタ|外部関数)\s/.test(l)) return l;
          const k = l.indexOf(`${a.name}(`);
          if (k < 0) return l;
          let depth = 0;
          for (let i = k + a.name.length; i < l.length; i++) {
            if (l[i] === "(") depth++;
            else if (l[i] === ")" && --depth === 0) {
              const inner = l.slice(k + a.name.length + 1, i).trim();
              return l.slice(0, i) + (inner ? ", " : "") + sample + l.slice(i);
            }
          }
          return l;
        })
        .join("\n");
      if (u !== t) write(f, u);
    }
  }
  return out;
}

// ---- 操作 ----
type Op = Record<string, any>;
let optionalNote = "";
function apply(op: Op): { file: string; line: number } | null {
  const o = outline();
  switch (op.op) {
    case "add_data": {
      const name = okName(op.name, "データの名前");
      if (decl(o, "data", name)) throw new Error(`データ「${name}」は既にあります`);
      const fields = (op.fields ?? []).filter((f: any) => (f.name ?? "").trim()).map((f: any) => `    ${okName(f.name, "項目の名前")}: ${okType(f.type, o, f.name)}`);
      if (fields.length === 0) throw new Error("項目を 1 つ以上書きます(例: 名前: 文字列)");
      return addDecl(o, ["data"], [`データ ${name}:`, "    ID: 識別子", ...fields]);
    }
    case "add_field": {
      const d = decl(o, "data", op.data);
      if (!d) throw new Error(`データ「${op.data}」がありません`);
      const name = okName(op.name, "項目の名前");
      if (d.fields.some((f: any) => f.name === name)) throw new Error(`「${op.data}」に項目「${name}」は既にあります`);
      let t = okType(op.type, o, name);
      // ボタンで呼んでいる登録の動作は引数を増やせない(ボタンに値が無い)。その項目は空でもよい項目として足す
      // (足すたびに「ほかの所が誤りになりました」が残った。第 6 回評価)
      // 自動で入れる値がある項目(状態 → 最初の値・予約者 → 本人 など)は、ボタンで登録していても必須のままにする
      // (状態が空でもよい項目になり、新しい予約の状態が空でボタンが出なかった。第 10 回評価)
      if (!/^選択</.test(t) && !autoValue(o, name, t)) {
        const srcAll = srcFiles(o).map((f) => read(f)).join("\n");
        const blocked = (o.decls ?? []).some((a: any) => {
          if (a.kind !== "action") return false;
          const body = lines(a.file).slice(a.line - 1, a.end).join("\n");
          return body.includes(`${d.name}を登録(`) && srcAll.includes(`を押すと ${a.name}(`);
        });
        if (blocked) {
          t = `選択<${t}>`;
          optionalNote = `(ボタンで登録しているので、空でもよい項目にしました)`;
        }
      }
      const at = addLine(d, `${name}: ${t}`);
      if (!/^選択</.test(t)) {
        extendRegistrations(d.name, name, t);
        fillSeed(o, d.name, name, t);
      }
      showOnDetailScreens(d.name, name);
      return at;
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
      const at = addDecl(o, ["role"], [`役割 ${name}:`, `    権限 = [${caps.join(", ")}]`]);
      return { ...at, note: roleGapNote(name) };
    }
    case "set_role_caps": {
      const d = decl(o, "role", op.name);
      if (!d) throw new Error(`役割「${op.name}」がありません`);
      const caps = (op.caps ?? []).map((c: string) => c.trim()).filter(Boolean);
      if (caps.length === 0) throw new Error("権限を 1 つ以上選びます");
      const ls = lines(d.file);
      ls.splice(d.line - 1, d.end - d.line + 1, `役割 ${d.name}:`, `    権限 = [${caps.join(", ")}]`);
      write(d.file, ls.join("\n"));
      return { file: d.file, line: d.line, note: roleGapNote(d.name) };
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
      const failVals = body.some((l: string) => /失敗\(/.test(l));
      if (!fail && !failVals) {
        // 失敗しない動作は、使わない失敗の列挙(「できない」)を作らない(第 6 回評価)。
        // 登録した・変えた物の ID は返す(返さないと、確かめるで名前を付けて次の行で使えなかった。第 7 回評価)
        const returns = body.some((l: string) => /を返す$/.test(l));
        block.push(`動作 ${name}(${params.join(", ")})${returns ? " -> 識別子" : ""}:`, ...body.map((l: string) => "    " + l));
        return addDecl(o, ["action"], block);
      }
      if (!fail) {
        // 失敗の種類は、処理に書いた 失敗("…") の値から作る(書いていなければ「できない」)。第 5 回評価
        fail = `${name}の失敗`;
        const values = [...new Set(body.flatMap((l: string) => [...l.matchAll(/失敗\(\s*"([^"]+)"\s*\)/g)].map((m) => m[1])))];
        if (!o.enums?.[fail]) block.push(`列挙 ${fail}:`, ...(values.length ? values : ["できない"]).map((v) => "    " + v), "");
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
      // 同じ行が既にあれば足さない(自動で入っていた行をもう一度足し、2 行になった。第 9 回評価)
      if (d.kind === "screen" && (d.lines ?? []).some((l: any) => l.text.trim() === String(op.text).trim()))
        throw new Error(`画面「${d.name}」には、もう「${String(op.text).trim()}」があります`);
      return addLine(d, op.text);
    }
    case "move_line": {
      // 行を 1 つ上・下へ(同じ宣言の中だけ。画面の部品の順番を画面で変えられなかった。第 7 回評価)
      const ls = lines(op.file);
      const i = op.line - 1;
      if (ls[i]?.trim() !== String(op.old ?? "").trim()) throw new Error("ソースが変わっています。もう一度読み込んでください");
      const step = op.dir === "up" ? -1 : 1;
      let j = i + step;
      while (j >= 0 && j < ls.length && (!ls[j].trim() || ls[j].trim().startsWith("#"))) j += step;
      if (j < 0 || j >= ls.length || !/^\s/.test(ls[j]) || !/^\s/.test(ls[i])) throw new Error("これ以上は動かせません");
      [ls[i], ls[j]] = [ls[j], ls[i]];
      write(op.file, ls.join("\n"));
      // データの項目を動かしたら、詳細の表示「Dの A と B を表示」の並びも項目の順にそろえる(詳細の並びが変わらなかった。第 9 回評価)
      const D = (o.decls ?? []).find((x: any) => x.kind === "data" && x.file === op.file && op.line > x.line && op.line <= x.end);
      if (D) {
        const D2 = decl(outline(), "data", D.name);
        const order = (D2?.fields ?? []).map((f: any) => f.name);
        const re = new RegExp(`^(\\s+)${reEsc(D.name)}の(.+)を表示\\s*$`);
        for (const f of srcFiles(o)) {
          const t = read(f);
          const u = t
            .split("\n")
            .map((l) => {
              const mm = re.exec(l);
              if (!mm || /フォーム/.test(mm[2])) return l;
              const parts = mm[2].split("と").map((x) => x.trim());
              if (!parts.every((p) => order.includes(p))) return l;
              parts.sort((a, b) => order.indexOf(a) - order.indexOf(b));
              return `${mm[1]}${D.name}の${parts.join("と")}を表示`;
            })
            .join("\n");
          if (u !== t) write(f, u);
        }
        // そのデータを登録する動作の受け取る値(登録フォームの欄)も、項目の順にそろえる
        // (一覧の列は動くのにフォームの欄の順が変わらなかった。第 10 回評価で 4 人)
        for (const a of (o.decls ?? []).filter((x: any) => x.kind === "action")) {
          const body = read(a.file).split("\n").slice(a.line - 1, a.end).join("\n");
          if (!body.includes(`${D.name}を登録(`)) continue;
          const names: string[] = (a.params ?? []).map((p: any) => p.name);
          const slots = names.map((n, k) => (order.includes(n) ? k : -1)).filter((k) => k >= 0);
          const want = slots.map((k) => names[k]).sort((x, y) => order.indexOf(x) - order.indexOf(y));
          for (let k = 0; k < slots.length; k++) {
            const cur = names.indexOf(want[k]);
            if (cur === slots[k]) continue;
            swapCallArgs(a.name, slots[k], cur, true);
            [names[slots[k]], names[cur]] = [names[cur], names[slots[k]]];
          }
        }
      }
      return { file: op.file, line: j + 1 };
    }
    case "add_param": {
      const a = (o.decls ?? []).find((x: any) => x.kind === "action" && x.name === op.action);
      if (!a) throw new Error(`動作「${op.action}」が見つかりません`);
      const name = okName(op.name, "受け取る値の名前");
      if ((a.params ?? []).some((p: any) => p.name === name)) throw new Error(`動作「${a.name}」は既に「${name}」を受け取ります`);
      const t = op.type ? okType(op.type, o) : guessType(name, o);
      const al = lines(a.file);
      al[a.line - 1] = addParam(al[a.line - 1], `${name}: ${t}`);
      // 登録しているデータに同じ名前の項目があれば、登録にも入れる
      for (let k = a.line; k < a.end; k++) {
        const mm = /([^\s(=]+)を登録\(/.exec(al[k] ?? "");
        const D = mm && decl(o, "data", mm[1]);
        if (D && D.fields.some((f: any) => f.name === name) && !new RegExp(`[(,]\\s*${reEsc(name)}\\s*:`).test(al[k])) al[k] = addParam(al[k], `${name}: ${name}`, al[k].indexOf(`${mm![1]}を登録(`));
      }
      write(a.file, al.join("\n"));
      appendCallArg(a.name, sampleValue(o, t, name), /ID$/.test(name) && decl(o, "data", name.slice(0, -2)) ? name.slice(0, -2) : "");
      return { file: a.file, line: a.line };
    }
    case "move_param": {
      // 動作の引数の順番を変える。ボタン・確かめるの呼び出しの値も同じように並べ替える
      const a = (o.decls ?? []).find((x: any) => x.kind === "action" && x.name === op.action);
      if (!a) throw new Error(`動作「${op.action}」が見つかりません`);
      const i = Number(op.index);
      const j = op.dir === "up" ? i - 1 : i + 1;
      if (!(i >= 0 && j >= 0 && i < a.params.length && j < a.params.length)) throw new Error("これ以上は動かせません");
      swapCallArgs(a.name, i, j, true);
      return { file: a.file, line: a.line };
    }
    case "replace_line":
    case "remove_line": {
      const ls = lines(op.file);
      if (ls[op.line - 1]?.trim() !== String(op.old ?? "").trim()) throw new Error("ソースが変わっています。もう一度読み込んでください");
      if (op.op === "remove_line") {
        // 宣言の最後の 1 行は消さない(画面などが空になって壊れ、足し直す欄も消えた。第 7 回評価)
        const o = outline();
        const d = (o.decls ?? []).find((x: any) => x.file === op.file && op.line > x.line && op.line <= x.end);
        if (d && ["screen", "rules", "scenario", "enum", "role"].includes(d.kind)) {
          const body = ls.slice(d.line, d.end).filter((l) => l.trim() && !l.trim().startsWith("#"));
          if (body.length <= 1) {
            const what = ({ screen: "画面", rules: "約束", scenario: "確かめる", enum: "列挙", role: "役割" } as any)[d.kind];
            throw new Error(`${what}「${d.name ?? "約束"}」の最後の 1 行なので消せません。先に新しい行を足すか、${what}ごと消すときは日本語の欄に「${d.name ?? "約束"}を消す」と書きます`);
          }
        }
        ls.splice(op.line - 1, 1);
      }
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
      // 集計(分析)を消したら、それを出している画面の行も消す。行が無くなった画面は、画面ごと消す
      // (空の画面が残り「ブロックが空です」になった。第 9 回評価)
      if (op.kind === "analysis") {
        const esc = String(op.name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        for (const f of new Set<string>((o.decls ?? []).map((x: any) => x.file))) {
          const t = read(f);
          const u = t.split("\n").filter((l) => !new RegExp(`^\\s+${esc}を表示\\s*$`).test(l)).join("\n");
          if (u !== t) write(f, u);
        }
        // (空の画面は読めないので、文字の並びで探す: 「画面 X:」の下に行が無い、または「この画面は…だけ」しか無い)
        for (const f of new Set<string>((o.decls ?? []).map((x: any) => x.file))) {
          const ls = lines(f);
          let touched = false;
          for (let k = ls.length - 1; k >= 0; k--) {
            if (!/^画面\s+\S+\s*:\s*$/.test(ls[k])) continue;
            let e = k + 1;
            while (e < ls.length && (/^\s+/.test(ls[e]) || ls[e].trim() === "")) e++;
            const body = ls.slice(k + 1, e).filter((l) => l.trim() && !l.trim().startsWith("#") && !/^\s+この画面は.+だけ\s*$/.test(l));
            if (body.length > 0) continue;
            // 前の空行も 1 つ消す
            const from = k > 0 && ls[k - 1].trim() === "" ? k - 1 : k;
            let to = e;
            while (to > k + 1 && ls[to - 1].trim() === "") to--;
            ls.splice(from, to - from);
            touched = true;
          }
          if (touched) write(f, ls.join("\n"));
        }
      }
      // 動作を消したら、その動作を呼ぶボタン・フォームの行と、役割の「動作.名前」も消す
      // (残すと消した直後にエラーになり、以降の操作が全部止まった)
      if (op.kind === "action") {
        const esc = String(op.name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const uses = new RegExp(`(を押すと\\s*${esc}\\(|^\\s*${esc}のフォームを表示)`);
        const cap = new RegExp(`,?\\s*動作\\.${esc}(?=\\s*[,\\]])`);
        for (const f of new Set<string>((o.decls ?? []).map((x: any) => x.file))) {
          const before = lines(f);
          const out = before.filter((l) => !uses.test(l)).map((l) => (/権限\s*=/.test(l) ? l.replace(cap, "").replace(/\[\s*,\s*/, "[") : l));
          if (out.join("\n") !== before.join("\n")) write(f, out.join("\n"));
        }
      }
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
      // 色そのものの誤りのときだけ戻す(ほかの所の誤りで色が変えられなかった。第 5 回評価)
      const chk = koto(["check"]);
      if (chk.code !== 0 && chk.err.includes("theme")) {
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
    return m ? { name: m[1].trim(), type: m[2].trim() } : { name: x, type: "" };
  });
}
function instruct(text0: string, ctx: { screen?: string; scenario?: string }): any {
  let text = text0.trim().replace(/[。.]$/, "");
  if (!text) throw new Error("指示を書いてください");
  const o = outline();
  // 「注文を新しい順に一覧表示」: 並べる項目が無いときは、日時(無ければ日付)の項目で並べる(第 7 回評価: 例のとおりに書いて誤りになった)
  {
    const nm = /^(.+?)(?:のうち(.+?)のもの)?を(新しい|古い)順に一覧表示$/.exec(text);
    const D = nm && decl(o, "data", nm[1].trim());
    if (D) {
      const f = D.fields.find((x: any) => x.type === "日時" && /(登録|作成|注文|予約|受付|申込|投稿|日時)/.test(x.name)) ?? D.fields.find((x: any) => x.type === "日時") ?? D.fields.find((x: any) => x.type === "日付");
      if (!f) throw new Error(`「${D.name}」には日時・日付の項目が無いので、新しい順に並べられません。「${D.name}に「登録日時」を追加」してから使います`);
      text = `${nm![1].trim()}${nm![2] ? `のうち${nm![2]}のもの` : ""}を${f.name}の${nm![3]}順に一覧表示`;
    }
  }
  // Excel の表(タブ区切りの何行か)を指示の欄に貼ったとき: 初期データの貼り付け欄を案内する(第 9 回評価)
  if (text0.includes("\t") && text0.trim().split("\n").length >= 2) {
    const head = text0.trim().split("\n")[0].split("\t").map((x) => x.trim());
    const D = (o.decls ?? []).filter((d: any) => d.kind === "data").find((d: any) => head.every((h) => d.fields.some((f: any) => f.name === h)));
    throw new Error(`これは表のようです。表は「動かす・初期データ・公開」の「Excel から貼り付け」の欄に入れます${D ? `(データ「${D.name}」を選んでから)` : ""}`);
  }
  // 宣言(分析・データ・動作 など)を何行か貼り付けたときは、そのままソースに入れて検査する
  // (チュートリアルの「分析」の 2 行を貼ると「意図行の中にブロックは書けません」になった。第 7 回評価)
  if (/^(データ|列挙|動作|画面|分析|役割|確かめる|関数) \S/.test(text0.trim()) && text0.trim().includes("\n")) {
    const block = text0.replace(/\r/g, "").split("\n").map((l) => l.replace(/\s+$/, "")).filter((l, i, a) => l || i < a.length - 1);
    if (!/:\s*$/.test(block[0])) throw new Error("1 行目は「分析 名前() -> …:」のように「:」で終わります");
    const body = block.slice(1).map((l) => (l.startsWith("    ") ? l : "    " + l.trim()));
    const at = addDecl(o, ["action", "data"], [block[0].trim(), ...body]);
    return checked(at, `ソースに入れました(${block[0].trim().split(/[ (]/)[0]} ${block[0].trim().split(/[ (:]/)[1] ?? ""})`, o, text);
  }
  const cmd = (m: RegExpExecArray | null) => m;
  let m: RegExpExecArray | null;
  // 動作に受け取る値を足す: 動作「予約する」に「目的」を追加(第 8 回評価: 読めず、ソースと確かめるを手で直した)
  if ((m = cmd(/^(?:動作)?「?(.+?)」?(?:の入力|の引数|の受け取る値)?に(?:入力|引数|受け取る値)?「?(.+?)」?(?:[(（](.+?)[)）])?を(?:追加(?:する)?|足す|加える)$/.exec(text))) && decl(o, "action", m[1].trim()) && !decl(o, "data", m[1].trim())) {
    const at = apply({ op: "add_param", action: m[1].trim(), name: m[2], type: m[3] ?? "" });
    return checked(at, `動作「${m[1].trim()}」に受け取る値「${m[2].trim()}」を足しました(ボタン・確かめるの呼び出しにも仮の値を入れました)`, o, text);
  }
  // 画面を役割だけにする: この画面は店長だけ / 画面「管理」は店長だけ(上の欄では読めなかった。第 8 回評価)
  {
    const mm = /^(?:画面「(.+?)」は|この画面は|(.+?)の画面は)「?(.+?)」?だけ(?:にする|が使える|が見られる)?$/.exec(text);
    if (mm) {
      const sname = (mm[1] ?? mm[2])?.trim() ?? ctx.screen;
      const roles = mm[3].split(/[と、・,]/).map((r) => r.trim()).filter(Boolean);
      const missing = roles.filter((r) => !decl(o, "role", r));
      if (missing.length) throw new Error(`役割「${missing.join("・")}」がありません(役割: ${(o.decls ?? []).filter((d: any) => d.kind === "role").map((d: any) => d.name).join("、") || "なし"})。先に 役割「${missing[0]}」を作る と書きます`);
      const screens = (o.decls ?? []).filter((d: any) => d.kind === "screen").map((d: any) => d.name);
      const sc = sname && decl(o, "screen", sname);
      if (!sc) return { ok: false, need: "screen", screens, message: "どの画面にしますか? 下の画面の名前を押すと、そこに入れます" };
      const ls = lines(sc.file);
      const line = `この画面は${roles.join("と")}だけ`;
      const k = ls.findIndex((l, i) => i >= sc.line && i < sc.end && /^\s+この画面は.+だけ\s*$/.test(l));
      if (k >= 0) {
        ls[k] = ls[k].replace(/この画面は.+だけ/, line);
        write(sc.file, ls.join("\n"));
        return checked({ file: sc.file, line: k + 1 }, `画面「${sc.name}」を ${roles.join("・")} だけにしました`, o, text);
      }
      const at = apply({ op: "add_line", kind: "screen", name: sc.name, text: line });
      return checked(at, `画面「${sc.name}」を ${roles.join("・")} だけにしました(ほかの人のメニューに出ません)`, o, text);
    }
  }
  // ボタンを足す: 「2名で予約」ボタンを足す(第 9 回評価: 読めず、専門の言葉の説明が出た)
  //   似たボタン(「1名で予約」)があれば、その行をまねて数を変える。動作の名前と同じなら、その動作を呼ぶ
  // 「イベント詳細に「2名で予約」ボタンを追加」のように画面の名前を前に書いてもよい(第 10 回評価)
  const btnScreen = /^(?:画面)?「?([^「」]+?)」?(?:の画面)?に「/.exec(text);
  if (btnScreen && decl(o, "screen", btnScreen[1].trim())) {
    ctx = { ...ctx, screen: btnScreen[1].trim() };
    text = text.slice(btnScreen[0].length - 1);
  }
  if ((m = cmd(/^「(.+?)」(?:の)?ボタンを(?:足す|付ける|つける|追加(?:する)?|作る)$/.exec(text) ?? /^(?:画面に)?ボタン「(.+?)」を(?:足す|付ける|つける|追加(?:する)?|作る)$/.exec(text)))) {
    const label = m[1].trim();
    const screens = (o.decls ?? []).filter((d: any) => d.kind === "screen");
    const cands = (ctx.screen ? screens.filter((d: any) => d.name === ctx.screen) : []).concat(screens);
    const num = /(\d+)/.exec(label)?.[1];
    for (const sc of cands) {
      const ls = lines(sc.file);
      for (let k = sc.line; k < sc.end; k++) {
        const mm = /^(\s+)(.*?)「(.+?)」を押すと\s*(.+?)\((.*)\)\s*$/.exec(ls[k] ?? "");
        if (!mm) continue;
        const oldNum = /(\d+)/.exec(mm[3])?.[1];
        // 数だけ違うボタン(1名で予約 → 2名で予約)
        if (num && oldNum && mm[3].replace(oldNum, num) === label) {
          const args = mm[5].split(",").map((x) => x.trim());
          const ai = args.findIndex((x) => x === oldNum);
          if (ai >= 0) args[ai] = num;
          const line = `${mm[2]}「${label}」を押すと ${mm[4]}(${args.join(", ")})`;
          if ((sc.lines ?? []).some((l: any) => l.text.trim() === line.trim())) throw new Error(`画面「${sc.name}」には、もう「${label}」のボタンがあります`);
          insertAfter(sc.file, k + 1, [mm[1] + line]);
          return checked({ file: sc.file, line: k + 2 }, `画面「${sc.name}」に「${label}」のボタンを足しました(「${mm[3]}」をまねて ${num} にしました)`, o, text);
        }
      }
    }
    const act = decl(o, "action", label);
    if (act) {
      const sc = (ctx.screen && decl(o, "screen", ctx.screen)) || null;
      if (!sc) return { ok: false, need: "screen", screens: screens.map((d: any) => d.name), message: "どの画面にボタンを足しますか? 下の画面の名前を押すと、そこに入れます" };
      const args = (act.params ?? []).map((p: any) => (p.name.endsWith("ID") && decl(o, "data", p.name.slice(0, -2)) ? `${p.name.slice(0, -2)}.ID` : /^(整数|金額)/.test(p.type) ? "1" : '""'));
      const at = apply({ op: "add_line", kind: "screen", name: sc.name, text: `「${label}」を押すと ${label}(${args.join(", ")})` });
      return checked(at, `画面「${sc.name}」に「${label}」のボタンを足しました`, o, text);
    }
    throw new Error(`「${label}」に似たボタンも、同じ名前の動作もありません。先に動作を作ります(例: 「${label}」で…を…にする)`);
  }
  // 入力の確認: 「注文する」で数量が0以下なら「数量不正」で止める(第 9 回評価: 読めず、ソースで書いた)
  if ((m = cmd(/^(?:動作)?(?:「(.+?)」で)?(.+?)が(-?\d+)(以下|未満|以上|より大きい|より小さい|を超える|を超えた)(?:なら|のとき|ときは)、?「?(.+?)」?(?:で|として|と出して)?(?:止める|失敗|失敗にする|断る|エラー)$/.exec(text)))) {
    const [, act0, fld, n, cmp, val] = m.map((x) => x?.trim());
    const acts = (o.decls ?? []).filter((d: any) => d.kind === "action" && (act0 ? d.name === act0 : (d.params ?? []).some((p: any) => p.name === fld)));
    if (acts.length === 0) throw new Error(act0 ? `動作「${act0}」が無いか、「${fld}」を受け取りません` : `「${fld}」を受け取る動作がありません`);
    if (acts.length > 1) throw new Error(`「${fld}」を受け取る動作が ${acts.map((a: any) => a.name).join("・")} の ${acts.length} つあります。「「${acts[0].name}」で${fld}が${n}${cmp}なら「${val}」で止める」のように書きます`);
    // 止める条件の反対が、通ってよい条件
    const okCond = ({ 以下: ">", 未満: ">=", 以上: "<", より大きい: "<=", より小さい: ">=", を超える: "<=", を超えた: "<=" } as Record<string, string>)[cmp];
    const at = apply({ op: "add_line", kind: "action", name: acts[0].name, text: `${fld} ${okCond} ${n} を確認 または 失敗("${val}")` });
    return checked(at, `動作「${acts[0].name}」で、${fld}が${n}${cmp}なら「${val}」で止めるようにしました`, o, text);
  }
  // アプリの名前: アプリの名前を「料理教室」にする(画面のいちばん上に出る)
  if ((m = cmd(/^アプリ(?:の名前)?を「?(.+?)」?に(?:する|変える)$/.exec(text)))) {
    const app = (o.decls ?? []).find((d: any) => d.kind === "app");
    if (!app) throw new Error("アプリの宣言が見つかりません");
    const nm = m[1].trim();
    if (!nm || /[「」\n]/.test(nm)) throw new Error("アプリの名前に「」は使えません");
    const ls = lines(app.file);
    ls[app.line - 1] = `アプリ「${nm}」`;
    write(app.file, ls.join("\n"));
    return changed(o, `アプリの名前を「${nm}」にしました`);
  }
  // 決まった名前の集計(API の名前・列の名前が決まっているとき。ADR-0133):
  //   分析「会場別の予約状況」を作る(イベントの予約数と定員を会場ごとに合計)
  //   列の名前を決めるとき: …(イベントの予約数と定員を会場ごとに合計、列: 会場・イベント数・予約数・定員)
  if ((m = cmd(/^(?:分析|集計)「(.+?)」を(?:作る|作って|追加(?:する)?)[(（](.+)[)）]$/.exec(text)) ?? cmd(/^「?(.+?)」?という(?:分析|集計)を(?:作る|作って)[(（](.+)[)）]$/.exec(text)))) {
    const an = okName(m[1], "分析の名前");
    let [sentence, colsPart] = m[2].split(/[、,]\s*列[::]\s*/);
    sentence = sentence.trim();
    const mm = /^(.+?)(?:のうち(.+?)のもの)?(?:の(.+?)を(.+?)ごとに合計|を(.+?)ごとに集計)$/.exec(sentence);
    if (!mm) throw new Error("集計の文は「イベントの予約数と定員を会場ごとに合計」か「予約を会場ごとに集計」の形で書きます");
    const dn = mm[1].trim();
    const D = decl(o, "data", dn);
    if (!D) throw new Error(`データ「${dn}」はありません`);
    const unit = (mm[4] ?? mm[5]).trim();
    const sums = mm[3] ? mm[3].split(/と/).map((x) => x.trim()) : [];
    const typeOf = (fn: string) => {
      const f = D.fields.find((x: any) => x.name === fn);
      if (!f) throw new Error(`データ「${dn}」に項目「${fn}」はありません(項目: ${D.fields.map((x: any) => x.name).join("、")})`);
      return f.type.replace(/^選択<(.+)>$/, "$1");
    };
    // 「…ごとに集計」は件数と、整数・金額の項目の合計も出す(画面の集計と同じ。ADR-0128)
    const autoSums = mm[3]
      ? []
      : D.fields.filter((f: any) => f.name !== unit && /^(整数|金額)/.test(f.type)).map((f: any) => f.name);
    const cols: [string, string][] = [
      [unit, typeOf(unit)],
      [`${dn}数`, "整数"],
      ...[...sums, ...autoSums].map((f): [string, string] => [mm[3] ? f : `${f}の合計`, /^金額/.test(typeOf(f)) ? "金額" : "整数"]),
    ];
    if (colsPart) {
      const names = splitList(colsPart);
      if (names.length !== cols.length) throw new Error(`列は ${cols.length} 個です(${cols.map((c) => c[0]).join("・")} の順。単位・件数・合計の順に当てはめます)`);
      names.forEach((n, i) => (cols[i][0] = okName(n, "列の名前")));
    }
    if ((o.decls ?? []).some((x: any) => x.name === an)) throw new Error(`「${an}」は既にあります`);
    const at = addDecl(o, ["action", "data"], [`分析 ${an}() -> 一覧<記録{${cols.map(([n, t]) => `${n}: ${t}`).join(", ")}}>:`, `    集計 = "${sentence}"`]);
    const sc = (ctx.screen && decl(outline(), "screen", ctx.screen)) || null;
    if (sc) apply({ op: "add_line", kind: "screen", name: sc.name, text: `${an}を表示` });
    return checked(at, `分析「${an}」を作りました(列: ${cols.map((c) => c[0]).join("・")})${sc ? `。画面「${sc.name}」に出しました` : "。画面を選んでから「" + an + "を表示」と書くと画面に出ます"}`, o, text);
  }
  // 列挙の値の名前を変える: 列挙「状況」の「受付」を「受付中」に変える
  if ((m = cmd(/^(?:列挙)?「?(.+?)」?の(?:値)?「(.+?)」を「(.+?)」に(?:変える|する|直す)$/.exec(text))) && decl(o, "enum", m[1].trim())) {
    const E = decl(o, "enum", m[1].trim());
    const [from, to] = [m[2].trim(), m[3].trim()];
    if (!(E.values ?? []).includes(from)) throw new Error(`列挙「${E.name}」に「${from}」はありません(値: ${(E.values ?? []).join("、")})`);
    okName(to, "値");
    for (const f of srcFiles(o)) {
      let inE = false;
      const t = read(f);
      const u = t
        .split("\n")
        .map((l) => {
          if (/^\S/.test(l)) inE = l.startsWith(`列挙 ${E.name}:`);
          if (inE && l.trim() === from) return l.replace(from, to);
          if (l.trim().startsWith("#") || /^\S/.test(l)) return l;
          // 文字列 "受付"、文の中の「が受付」「受付 →」など
          return l.split(`"${from}"`).join(`"${to}"`).replace(new RegExp(`(?<=[がはを\\s→、(])${reEsc(from)}(?=$|[\\s→、)のでと])`, "g"), to);
        })
        .join("\n");
      if (u !== t) write(f, u);
    }
    for (const f of seedFiles()) {
      const t = readFileSync(join(PROJECT, f), "utf8");
      const u = t.split(`"${from}"`).join(`"${to}"`);
      if (u !== t) writeRaw(f, u);
    }
    return changed(o, `列挙「${E.name}」の「${from}」を「${to}」に変えました`);
  }
  // 項目の型を変える: 予約の「状態」を「予約状態」にする / 「家賃」の型を金額にする
  if (
    (m = cmd(/^(?:(.+?)の)?「(.+?)」の(?:型|種類)を「?(.+?)」?に(?:する|変える)$/.exec(text))) ||
    (m = cmd(/^(?:([^「」]+?)の)?「?([^「」]+?)」?を(?:列挙)?「?([^「」]+?)」?(?:型)?に(?:する|変える)$/.exec(text)))
  ) {
    const [dn, fn] = [m[1]?.trim(), m[2].trim()];
    const ds = (o.decls ?? []).filter((d: any) => d.kind === "data" && (!dn || d.name === dn) && d.fields.some((f: any) => f.name === fn));
    let typeOk = true;
    try {
      // 今と同じ型なら型の変更ではない(「人数」を「数」にする は名前の変更)
      typeOk = ds.length === 1 && okType(m[3], o) !== ds[0].fields.find((f: any) => f.name === fn).type;
    } catch {
      typeOk = false;
    }
    if (ds.length === 1 && !typeOk && /にする$/.test(text) && !/^(?:空|任意|省|なくて|無くて|必須)/.test(m[3].trim())) {
      const cur = ds[0].fields.find((f: any) => f.name === fn).type;
      if (cur === m[3].trim()) return { ok: true, done: `「${ds[0].name}」の「${fn}」は既に ${cur} です` };
      throw new Error(`「${m[3].trim()}」という型はありません(今は ${cur})。選択肢の型にするなら、先に 列挙「${m[3].trim()}」を作る(…) と書きます。項目の名前を変えるときは「${ds[0].name}の「${fn}」を「${m[3].trim()}」に変える」と書きます`);
    }
    if (ds.length === 1 && typeOk) {
      const D = ds[0];
      const old = D.fields.find((f: any) => f.name === fn).type;
      const t = okType(m[3], o);
      const ls = lines(D.file);
      const i = ls.findIndex((l, k) => k >= D.line && k < D.end && new RegExp(`^\\s*${reEsc(fn)}\\s*:`).test(l));
      ls[i] = ls[i].replace(/:\s*(.+?)\s*$/, `: ${t}`);
      write(D.file, ls.join("\n"));
      // その項目を登録する動作の引数の型もそろえる
      for (const a of (outline().decls ?? []).filter((x: any) => x.kind === "action")) {
        const al = lines(a.file);
        if (!al.slice(a.line - 1, a.end).join("\n").includes(`${D.name}を登録(`)) continue;
        const h = al[a.line - 1].replace(new RegExp(`([(,]\\s*${reEsc(fn)}\\s*:\\s*)${reEsc(old)}(?=\\s*[,)])`), `$1${t}`);
        if (h !== al[a.line - 1]) {
          al[a.line - 1] = h;
          write(a.file, al.join("\n"));
          // 確かめるの呼び出しの値も、新しい型に合う値にする(列挙にしたら "" のままで通らなかった。第 7 回評価)
          const idx = (a.params ?? []).findIndex((p: any) => p.name === fn);
          const enumVals: string[] | undefined = o.enums?.[t];
          if (idx >= 0)
            dropScenarioArg(a.name, idx, (v) => {
              if (enumVals) return enumVals.includes(v.replace(/^"|"$/g, "")) ? v : sampleValue(o, t);
              if (/^(整数|金額)/.test(t)) return /^-?\d+$/.test(v) ? v : "0";
              if (t === "文字列") return /^".*"$/.test(v) ? v : `"${v.replace(/"/g, "")}"`;
              return sampleValue(o, t);
            });
        }
      }
      convertSeed(o, D.name, fn, t);
      return changed(o, `「${D.name}」の「${fn}」を ${t} にしました`);
    }
  }
  // 名前を変える(第 5 回評価: ひな形の「枠」を自分の言葉にする方法が無かった)
  //   枠の「定員」を「人数」に変える / 「枠」を「レッスン」に変える / 枠の名前をレッスンに変える
  if ((m = cmd(/^(?:データ)?「?(.+?)」?の(?:項目)?「(.+?)」を「(.+?)」に(?:名前を)?(?:変える|かえる|変更(?:する)?|直す)$/.exec(text))) && decl(o, "data", m[1].trim())) {
    const D = decl(o, "data", m[1].trim());
    const has = (n: string) => D.fields.some((f: any) => f.name === n);
    // 「枠」を「イベント」に変えると「枠ID」も「イベントID」に変わる。その後の「枠ID」を「イベントID」に変える は、もう済んでいると伝える(第 10 回評価)
    if (!has(m[2].trim()) && has(m[3].trim())) return { ok: true, done: `「${D.name}」の項目は、もう「${m[3].trim()}」になっています(前に名前を変えたときに一緒に変わりました)` };
    renameField(o, m[1].trim(), m[2].trim(), m[3].trim());
    return changed(o, `「${m[1].trim()}」の項目「${m[2].trim()}」を「${m[3].trim()}」に変えました`);
  }
  if (
    (m = cmd(/^(?:データ|画面|動作|役割|列挙|確かめる)?「?(.+?)」?の名前を「?(.+?)」?に(?:変える|かえる|変更(?:する)?|直す|する)$/.exec(text))) ||
    (m = cmd(/^(?:データ|画面|動作|役割|列挙|確かめる)?「([^「」]+?)」を「([^「」]+?)」に(?:名前を)?(?:変える|かえる|変更(?:する)?|直す)$/.exec(text))) ||
    (m = cmd(/^(?:データ|画面|動作|役割|列挙|確かめる)?「?(.+?)」?を「?(.+?)」?に(?:名前を)?(?:変える|かえる|変更(?:する)?|直す)$/.exec(text)))
  ) {
    let [from, to] = [m[1].trim(), m[2].trim()];
    // 前に変えた名前で書いたとき(「枠」を「イベント」に変えた後の「枠を追加」): 今の名前に読み替える(第 9 回評価)
    let note = "";
    // 変えたい名前が、もうある(前に同じ指示をした・前に名前を変えたときに一緒に変わった)ときは、済んでいると伝える(第 10 回評価)
    if (!(o.decls ?? []).some((x: any) => x.name === from) && (o.decls ?? []).some((x: any) => x.name === to))
      return { ok: true, done: `もう「${to}」になっています(前に名前を変えたときに一緒に変わりました)` };
    if (!(o.decls ?? []).some((x: any) => x.name === from)) {
      for (const [a, b] of renamedWords) {
        const cand = from.split(a).join(b);
        if (cand !== from && (o.decls ?? []).some((x: any) => x.name === cand)) {
          note = `(「${a}」は前に「${b}」に変えたので、今の名前は「${cand}」です)`;
          from = cand;
          break;
        }
      }
    }
    if ((o.decls ?? []).some((x: any) => x.name === from)) {
      renameDecl(o, from, to);
      renamedWords.unshift([from, to]);
      return changed(o, `「${from}」を「${to}」に変えました(使っている所と初期データもそろえました)${note}`);
    }
    const owners = (o.decls ?? []).filter((d: any) => d.kind === "data" && d.fields.some((f: any) => f.name === from));
    if (owners.length === 1) {
      renameField(o, owners[0].name, from, to);
      return changed(o, `「${owners[0].name}」の項目「${from}」を「${to}」に変えました`);
    }
    if (owners.length > 1) throw new Error(`項目「${from}」は ${owners.map((d: any) => d.name).join("・")} にあります。「${owners[0].name}の「${from}」を「${to}」に変える」のように書きます`);
    // 変えたい名前が、もうある(前に名前を変えたときに一緒に変わった)ときは、済んでいると伝える(第 10 回評価)
    if ((o.decls ?? []).some((x: any) => x.name === to)) return { ok: true, done: `もう「${to}」になっています(前に名前を変えたときに一緒に変わりました)` };
    const toOwners = (o.decls ?? []).filter((d: any) => d.kind === "data" && d.fields.some((f: any) => f.name === to));
    if (toOwners.length) return { ok: true, done: `「${toOwners[0].name}」の項目は、もう「${to}」になっています(前に名前を変えたときに一緒に変わりました)` };
    // 前に変えた名前を含む項目名(「予約」を「取り置き」に変えた後の「予約者」)は、今の項目名を教える
    for (const [a, b] of renamedWords) {
      const cand = from.split(a).join(b);
      const ow = cand !== from && (o.decls ?? []).find((d: any) => d.kind === "data" && d.fields.some((f: any) => f.name === cand));
      if (ow) {
        renameField(o, ow.name, cand, to);
        return changed(o, `「${ow.name}」の項目「${cand}」を「${to}」に変えました(「${a}」は前に「${b}」に変えたので、今の名前は「${cand}」でした)`);
      }
    }
    throw new Error(`「${from}」という名前はありません`);
  }
  // 権限を与える: 受付係に予約の更新を許可する / 受付係は予約を更新できる / 店長に動作「締める」を許可する(第 6 回評価)
  {
    const CAP: Record<string, string> = { 読取: "読取", 閲覧: "読取", 読み取り: "読取", 見る: "読取", 作成: "作成", 登録: "作成", 追加: "作成", 更新: "更新", 変更: "更新", 書き換え: "更新", 削除: "削除", 全件閲覧: "全件閲覧" };
    const role = (r: string) => decl(o, "role", r.replace(/^役割/, "").replace(/[「」]/g, "").trim());
    // 「店長に予約の読取と更新を許可する」のように 2 つ以上まとめても読む(第 9 回評価)
    const V = "読取|閲覧|読み取り|作成|登録|追加|更新|変更|書き換え|削除";
    let mm = new RegExp(`^(?:役割)?「?(.+?)」?に(.+?)の((?:${V})(?:(?:と|、|・)(?:${V}))*)(?:の権限)?を?(?:許可|与え|許)(?:する|る|す)?$`).exec(text);
    let caps: string[] = [];
    if (mm && role(mm[1]) && decl(o, "data", mm[2].trim())) caps = [...new Set(mm[3].split(/と|、|・/).map((v) => `${mm![2].trim()}.${CAP[v]}`))];
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?(?:は|が)(.+?)を(見|閲覧|作成|登録|追加|更新|変更|削除)(?:でき|られ)る(?:ように(?:する|したい))?$/.exec(text)) && role(mm[1]) && decl(o, "data", mm[2].trim()))
      caps = [`${mm[2].trim()}.${CAP[mm[3] === "見" ? "見る" : mm[3]]}`];
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?に(?:動作)?「(.+?)」を(?:許可|許)(?:する|す)?$/.exec(text)) && role(mm[1]) && decl(o, "action", mm[2].trim())) caps = [`動作.${mm[2].trim()}`];
    // 権限の名前をそのまま: 指導員に全件閲覧.欠席連絡を許可する / 受付係に予約.更新を許可する(第 7 回評価)
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?に「?((?:全件閲覧|動作)\.[^「」\s]+?|[^「」\s.]+\.(?:読取|作成|更新|削除))」?を(?:許可|与え|許)(?:する|る|す)?$/.exec(text)) && role(mm[1])) {
      const [kind, rest] = mm[2].split(".");
      const ok = kind === "全件閲覧" ? !!decl(o, "data", rest) : kind === "動作" ? !!decl(o, "action", rest) : !!decl(o, "data", kind);
      if (!ok) throw new Error(`権限「${mm[2]}」の名前が見つかりません(データ・動作の名前を確かめてください)`);
      caps = [mm[2]];
    }
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?に(.+?)の全員の分を見せる$/.exec(text)) && role(mm[1]) && decl(o, "data", mm[2].trim())) caps = [`全件閲覧.${mm[2].trim()}`];
    // 全件閲覧を日本語で: 課長に商談の全件閲覧を許可する / 監督は全員の日報を見られる(第 7 回評価: 読めなかった)
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?に「?(.+?)」?の(?:全件閲覧|全件の閲覧|全員分の閲覧|全部の閲覧)(?:の権限)?を?(?:許可|与え|許)(?:する|る|す)?$/.exec(text)) && role(mm[1]) && decl(o, "data", mm[2].trim())) caps = [`全件閲覧.${mm[2].trim()}`];
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?(?:は|が|に)(?:全員|みんな|全部|ほかの人|他の人)(?:の分)?の「?(.+?)」?(?:も)?を?(?:見|閲覧)(?:でき|られ|せ)る(?:ように(?:する|したい))?$/.exec(text)) && role(mm[1]) && decl(o, "data", mm[2].trim())) caps = [`全件閲覧.${mm[2].trim()}`];
    if (!caps.length && (mm = /^(?:役割)?「?(.+?)」?(?:は|が)「?(.+?)」?を(?:全件|全部|全員分)(?:見|閲覧)(?:でき|られ)る(?:ように(?:する|したい))?$/.exec(text)) && role(mm[1]) && decl(o, "data", mm[2].trim())) caps = [`全件閲覧.${mm[2].trim()}`];
    if (caps.length && mm) {
      const r = role(mm[1]);
      const now: string[] = r.caps ?? [];
      if (caps.every((c) => now.includes(c))) return { ok: true, done: `役割「${r.name}」は既に ${caps.join("・")} を持っています` };
      apply({ op: "set_role_caps", name: r.name, caps: [...new Set([...now, ...caps])] });
      const who = caps.filter((c) => c.startsWith("動作.")).flatMap((c) => roleLessCallers(c.slice(3)).map((a) => [a, c.slice(3)]));
      const note = who.length
        ? `。注意: 確かめるで役割の無い ${[...new Set(who.map((w) => w[0]))].join("・")} が「${[...new Set(who.map((w) => w[1]))].join("・")}」を行っています。この動作は ${r.name} だけになったので、そのままだと ✗(権限がない)になります。「${who[0][0]}(${r.name})が …」のように役割を書くか、${r.name} の人の行に書き換えてください`
        : "";
      return changed(o, `役割「${r.name}」に ${caps.join("・")} を足しました${note}`);
    }
  }
  // 役割を作る: 役割「店長」を作る(予約.読取、予約.更新)。権限を書かなければ、すべてのデータを見られるだけ
  if ((m = cmd(/^役割「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.+)[)）])?$/.exec(text)))) {
    const caps = m[2] ? splitList(m[2]) : (o.decls ?? []).filter((d: any) => d.kind === "data").map((d: any) => `${d.name}.読取`);
    const at = apply({ op: "add_role", name: m[1].trim(), caps });
    return changed(o, `役割「${m[1].trim()}」を作りました(権限: ${caps.join("、")})。左の「役割」で権限のチェックを変えられます${at?.note ? `。${at.note}` : ""}`);
  }
  // 項目を消す: 予約の「メモ」を消す(登録の動作の引数・確かめるの値もそろえる)
  if ((m = cmd(/^(.+?)の(?:項目)?「?(.+?)」?を(?:消す|消して|削除(?:する)?)$/.exec(text))) && decl(o, "data", m[1].trim())) {
    return deleteField(o, decl(o, "data", m[1].trim()), m[2].trim());
  }
  // 空でもよい項目にする: 「メモ」を空でもよいにする / 予約の要望を任意にする
  if ((m = cmd(/^(?:(.+?)の)?「?(.+?)」?(?:を|は)(?:空でもよい|空でも良い|空でもいい|空欄でもよい|空欄可|任意|省ける|省略できる|なくてもよい|無くてもよい|必須でない)(?:の)?(?:項目|欄)?(?:に(?:する|したい|して))?$/.exec(text)))) {
    const fn = m[2].trim();
    const inData = (o.decls ?? []).some((d: any) => d.kind === "data" && (!m![1] || d.name === m![1].trim()) && d.fields.some((f: any) => f.name === fn));
    if (!inData) {
      // データの項目でなく、動作の入力だけのとき(フォームの欄): 動作の引数を 選択<…> にする
      const acts = (o.decls ?? []).filter((d: any) => d.kind === "action" && (!m![1] || d.name === m![1].trim()) && (d.params ?? []).some((p: any) => p.name === fn && !/^選択</.test(p.type)));
      if (acts.length === 0) throw new Error(`項目「${fn}」は${m[1] ? `「${m[1].trim()}」に` : "どのデータ・動作にも"}ありません`);
      for (const a of acts) {
        const al = lines(a.file);
        const p = a.params.find((x: any) => x.name === fn);
        al[a.line - 1] = al[a.line - 1].replace(new RegExp(`([(,]\\s*${reEsc(fn)}\\s*:\\s*)${reEsc(p.type)}(?=\\s*[,)])`), `$1選択<${p.type}>`);
        write(a.file, al.join("\n"));
      }
      return changed(o, `動作 ${acts.map((a: any) => a.name).join("・")} の入力「${fn}」を空でもよい欄にしました`);
    }
    const done = makeOptional(o, m[1]?.trim(), fn);
    if (done.length === 0) return { ok: true, done: `「${m[2].trim()}」は既に空でもよい項目です` };
    return changed(o, `${done.join("・")}を空でもよい項目にしました(入力欄の「必須」が外れます)`);
  }
  // 動作「X」を作る(項目、…): 項目をすべて持つデータがあれば、その登録の動作にする
  if ((m = cmd(/^動作「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.*)[)）])?$/.exec(text)))) {
    const act = okName(m[1], "動作の名前");
    const names = m[2] ? parseFields(m[2]).map((f) => f.name) : [];
    const datas = (o.decls ?? []).filter((d: any) => d.kind === "data");
    const D = names.length ? datas.find((d: any) => names.every((n) => d.fields.some((f: any) => f.name === n))) : null;
    if (D) return registration(o, ctx, text, D, act, names);
    const target = names.length ? datas.find((d: any) => names.some((n) => n === `${d.name}ID`)) : null;
    const ex = target ?? datas[0];
    const f1 = ex?.fields.find((f: any) => o.enums?.[f.type]);
    const n1 = ex?.fields.find((f: any) => /^(整数|金額)/.test(f.type));
    const sug = ex
      ? [
          f1 ? `「${act}」で${ex.name}の${f1.name}を「${o.enums[f1.type][0]}」にする` : "",
          n1 ? `「${act}」で${ex.name}の${n1.name}を1増やす` : "",
          `「${act}」で${ex.name}を登録する`,
        ].filter(Boolean)
      : [];
    return {
      ok: false,
      errors: [{ code: "STUDIO", message: `動作「${act}」が何をするのかを、決まった言い方で書きます(登録する・項目を変える・数を増やす/減らす)。それ以外の処理は左の「+ 新しい動作」で書きます`, hints: [] }],
      suggestions: sug,
    };
  }
  // 決まった言い方: 構成を変える操作
  if ((m = cmd(/^データ「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.+)[)）])?$/.exec(text)))) {
    const at = apply({ op: "add_data", name: m[1], fields: m[2] ? parseFields(m[2]) : [{ name: "名前", type: "文字列" }] });
    return { ok: true, done: `データ「${m[1]}」を作りました`, at };
  }
  // 列挙に値を足す: 「列挙「状況」に「保留」を追加」/「状況に「保留」を追加」
  if ((m = cmd(/^(?:列挙)?「?(.+?)」?に(?:値)?「?(.+?)」?を(?:追加(?:する)?|足す|加える)$/.exec(text))) && decl(o, "enum", m[1].trim())) {
    const at = apply({ op: "add_line", kind: "enum", name: m[1].trim(), text: okName(m[2], "値") });
    return checked(at, `列挙「${m[1].trim()}」に「${m[2].trim()}」を足しました`, o, text);
  }
  // 計算項目: 「枠に「残席 = 定員 - 予約数」を追加」(ADR-0136)
  if ((m = cmd(/^(.+?)に(?:計算項目)?「?(.+?)\s*=\s*(.+?)」?を(?:追加(?:する)?|足す|加える)$/.exec(text))) && decl(o, "data", m[1].trim())) {
    const at = apply({ op: "add_line", kind: "data", name: m[1].trim(), text: `${okName(m[2], "計算項目の名前")} = ${m[3].trim()}` });
    return checked(at, `「${m[1].trim()}」に計算項目「${m[2].trim()}」を足しました`, o, text);
  }
  if ((m = cmd(/^(.+?)に(?:項目)?「?(.+?)」?(?:[(（](.+?)[)）])?を(?:追加(?:する)?|足す|加える)$/.exec(text) ?? /^(.+?)に(?:項目)?「?(.+?)」?を(?:追加(?:する)?|足す|加える)[(（](.+?)[)）]$/.exec(text))) && decl(o, "data", m[1].trim())) {
    // 「参加費(金額)」「参加費: 金額」のように名前の中に型を書いたとき(読めなかった。第 8・9 回評価)
    const inner = /^(.+?)[(（](.+?)[)）]$/.exec(m[2].trim()) ?? /^(.+?)\s*[:：]\s*(.+)$/.exec(m[2].trim());
    if (inner && !m[3]) [m[2], m[3]] = [inner[1], inner[2]];
    optionalNote = "";
    const at = apply({ op: "add_field", data: m[1].trim(), name: m[2], type: m[3] ?? "" });
    const D2 = decl(outline(), "data", m[1].trim());
    const f = D2?.fields.find((x: any) => x.name === m![2].trim());
    // 「残り」「残席」は、ほかの項目から計算で出すことが多い。その書き方を添える(文字の項目になった。第 10 回評価)
    const ints = (D2?.fields ?? []).filter((x: any) => /^(整数|金額)/.test(x.type)).map((x: any) => x.name);
    const calcHint = /^(残り|残席|残数|残量|在庫残|空き)/.test(m[2].trim()) && ints.length >= 2 ? `。計算で出すなら「1 つ前に戻す」を押してから「${m[1].trim()}に「${m[2].trim()} = ${ints[0]} - ${ints[1]}」を追加」と書きます` : "";
    return checked(at, `「${m[1]}」に項目「${m[2]}」(${f?.type ?? "?"})を足しました${optionalNote}${calcHint}`, o, text);
  }
  if ((m = cmd(/^列挙「?(.+?)」?を(?:作る|作って|追加(?:する)?)[(（](.+)[)）]$/.exec(text)))) {
    const at = apply({ op: "add_enum", name: m[1], values: splitList(m[2]) });
    return { ok: true, done: `列挙「${m[1]}」を作りました`, at };
  }
  if ((m = cmd(/^画面「?(.+?)」?を(?:作る|作って|追加(?:する)?)(?:[(（](.+)[)）])?$/.exec(text)))) {
    // 画面の名前の空白は詰める(「先生用 予約の一覧」が断られた。第 10 回評価)
    m[1] = m[1].trim().replace(/[\s　]+/g, "");
    const ls = m[2] ? m[2].split(/[、;；]/).map((x) => x.trim()).filter(Boolean) : [];
    if (ls.length === 0) throw new Error(`画面に出すものも書いてください(例: 画面「${m[1]}」を作る(注文を新しい順に一覧表示))`);
    const at = apply({ op: "add_screen", name: m[1], lines: ls });
    return checked(at, `画面「${m[1]}」を作りました`, o, text);
  }
  if ((m = cmd(/^(?:色|テーマ色|テーマの色)を「?(.+?)」?に(?:する|変える)$/.exec(text)))) {
    apply({ op: "set_theme", color: m[1] });
    return { ok: true, done: `色を「${m[1]}」にしました` };
  }
  // 消す: 「枠」を消す / 画面「管理」を消す / データ「枠」を削除
  if ((m = cmd(/^(?:データ|画面|動作|役割|列挙|確かめる)?「?(.+?)」?を(?:消す|消して|削除(?:する)?)$/.exec(text)))) {
    const name = m[1].trim();
    const d = (o.decls ?? []).find((x: any) => x.name === name && ["data", "screen", "action", "role", "enum", "scenario"].includes(x.kind));
    if (!d) {
      // 項目の名前なら項目を消す(「作業時間を消す」が通じなかった。第 6 回評価)
      const owners = (o.decls ?? []).filter((x: any) => x.kind === "data" && x.fields.some((f: any) => f.name === name));
      if (owners.length === 1) return deleteField(o, owners[0], name);
      if (owners.length > 1) throw new Error(`項目「${name}」は ${owners.map((x: any) => x.name).join("・")} にあります。「${owners[0].name}の${name}を消す」のように書きます`);
      // 約束・画面などの 1 行そのもの(「「生徒は誰でも閲覧可」を消す」が通じなかった。第 10 回評価)
      const hits: { file: string; line: number; old: string }[] = [];
      for (const f of srcFiles(o)) read(f).split("\n").forEach((l, k) => /^\s/.test(l) && l.trim() === name && hits.push({ file: f, line: k + 1, old: l }));
      if (hits.length === 1) {
        apply({ op: "remove_line", ...hits[0] });
        return changed(o, `「${name}」の行を消しました`);
      }
      if (hits.length > 1) throw new Error(`「${name}」の行が ${hits.length} か所にあります。左の一覧で消したい行の「消す」を押します`);
      throw new Error(`「${name}」という名前のデータ・画面・動作・役割・項目はありません`);
    }
    if (d.kind === "data") {
      // データは、使っている動作・画面の行・確かめる・権限もまとめて消す(1 つずつ消すとエラーが 12 件残った。第 5 回評価)
      const removed = removeDataCascade(o, name);
      const after = outline();
      const errs = (after.diagnostics ?? []).filter((x: any) => x.severity === "error").length;
      return { ok: true, done: `消しました: ${removed.join("、")}${errs ? `(まだ ${errs} か所エラーがあります。「取り消す」で全部戻せます)` : ""}` };
    }
    apply({ op: "remove_decl", kind: d.kind, name });
    const after = outline();
    const errs = (after.diagnostics ?? []).filter((x: any) => x.severity === "error").length;
    return { ok: true, done: `「${name}」を消しました${errs ? `(使っている所が ${errs} か所エラーになりました。「取り消す」で戻せます)` : ""}` };
  }
  // 動作のひな形(第 5 回評価: 日本語で動作を作れない が最も多かった)
  // 「呼び出す」で整理券の状態を呼出中にする
  if ((m = cmd(/^「(.+?)」で(.+?)の(.+?)を「?(.+?)」?に(?:する|変える)$/.exec(text)))) {
    const [, act, dn, fn, v] = m.map((x) => x?.trim());
    return actionTemplate(o, text, act, dn, (D) => {
      const f = D.fields.find((x: any) => x.name === fn);
      if (!f) throw new Error(`データ「${dn}」に項目「${fn}」はありません(項目: ${D.fields.map((x: any) => x.name).join("、")})`);
      const lit = /^(整数|金額|選択<整数>)/.test(f.type) ? v : f.type === "真偽" ? (/^(はい|真|済|オン)$/.test(v) ? "真" : "偽") : `"${v}"`;
      // 借りる・引き受けるなどの動作では、した人と日時も入れる(「借りる」で状態を貸出中にしても、借りた人が残らなかった。第 10 回評価)。
      // 登録した人の項目(予約者・受付者など)は書き換えない(「呼び出す」で予約者が受付係に変わり、権限がなくなった。第 11 回評価)
      const extra: string[] = [];
      if (/借り|貸出|貸し出|引き受け|受け持/.test(act) && !/返|戻|取り?消|キャンセル|終/.test(act)) {
        const regSets = (n: string) =>
          (o.decls ?? []).some((a: any) => a.kind === "action" && lines(a.file).slice(a.line, a.end).some((l) => l.includes(`${dn}を登録(`) && new RegExp(`[(,]\\s*${reEsc(n)}\\s*:\\s*本人`).test(l)));
        for (const g of D.fields) {
          if (g.name === fn) continue;
          const t = g.type.replace(/\(任意\)$/, "").replace(/^選択<(.+)>$/, "$1");
          if (t === "識別子" && /^(借りた人|借り手|借主|貸出先|担当者|借用者|貸出者)$/.test(g.name) && !regSets(g.name)) extra.push(`件.${g.name} = 本人`);
          else if (t === "日時" && /^(貸出|借用|借りた|貸し出し)(日時|時刻)$/.test(g.name)) extra.push(`件.${g.name} = 現在日時()`);
        }
      }
      return [`件 = ${dn}を取得(${dn}ID)`, `件.${fn} = ${lit}`, ...extra, `${dn}ID を返す`];
    });
  }
  // 取り消し: 予約を取り消せるようにしたい(第 9 回評価: 読めず、取り消しを作れなかった)
  //   状態の項目(取消・キャンセルの値を持つ列挙)が無ければ作る。登録の動作には最初の値を入れる。
  //   「枠ID」のように親を指し、親に「予約数」、自分に「人数」があれば、親の予約数を戻す。2 回目は「取消済み」で断る
  if ((m = cmd(/^(?:「(.+?)」で)?(.+?)を(?:取り消せる|取り消しできる|キャンセルできる)ように(?:する|したい|して)?$/.exec(text) ?? /^(?:「(.+?)」で)?(.+?)を取り消す$/.exec(text))) && decl(o, "data", m[2].trim())) {
    const dn = m[2].trim();
    const act = m[1]?.trim() || `${dn}を取り消す`;
    if (decl(o, "action", act)) throw new Error(`動作「${act}」は既にあります`);
    let D = decl(o, "data", dn);
    let sf = D.fields.find((f: any) => o.enums?.[f.type.replace(/^選択<(.+)>$/, "$1")]?.some((v: string) => /取消|取り消し|キャンセル/.test(v)));
    let cancelV = "";
    if (sf) cancelV = o.enums[sf.type.replace(/^選択<(.+)>$/, "$1")].find((v: string) => /取消|取り消し|キャンセル/.test(v));
    else {
      // 状態の列挙と項目を足す
      const en = decl(o, "enum", `${dn}状態`) ? `${dn}の状態` : `${dn}状態`;
      apply({ op: "add_enum", name: en, values: ["有効", "取消済み"] });
      const o2 = outline();
      const D2 = decl(o2, "data", dn);
      addLine(D2, `状態: ${en}`);
      // 登録の行には「有効」を入れる(フォームの入力は増やさない)
      for (const a of (outline().decls ?? []).filter((x: any) => x.kind === "action")) {
        const al = lines(a.file);
        let hit = false;
        for (let k = a.line; k < a.end; k++) {
          const at0 = (al[k] ?? "").indexOf(`${dn}を登録(`);
          if (at0 >= 0 && /\w*[:：]/.test(al[k].slice(at0)) && !/[(,]\s*状態\s*:/.test(al[k])) {
            al[k] = addParam(al[k], `状態: "有効"`, at0);
            hit = true;
          }
        }
        if (hit) write(a.file, al.join("\n"));
      }
      fillSeed(outline(), dn, "状態", en);
      sf = { name: "状態", type: en };
      cancelV = "取消済み";
    }
    D = decl(outline(), "data", dn);
    const parent = D.fields.map((f: any) => f.name).find((n: string) => n.endsWith("ID") && n !== "ID" && decl(o, "data", n.slice(0, -2)));
    const P = parent && decl(outline(), "data", parent.slice(0, -2));
    const cnt = P?.fields.find((f: any) => /(予約数|申込数|人数の合計|使用数|貸出数)$/.test(f.name))?.name;
    const qty = D.fields.find((f: any) => /^(人数|数量|数|個数|台数)$/.test(f.name))?.name;
    // 失敗の理由の列挙(既にあれば値を足す。無ければ作る)
    const failEnum = (outline().decls ?? []).find((d: any) => d.kind === "enum" && /失敗$/.test(d.name) && (d.values ?? []).length);
    let fe = failEnum?.name;
    if (fe && !failEnum.values.includes("取消済み")) apply({ op: "add_line", kind: "enum", name: fe, text: "取消済み" });
    if (!fe) {
      fe = `${act}の失敗`;
      apply({ op: "add_enum", name: fe, values: ["取消済み"] });
    }
    const body = [
      `件 = ${dn}を取得(${dn}ID)`,
      `件.${sf.name} != "${cancelV}" を確認 または 失敗("取消済み")`,
      `件.${sf.name} = "${cancelV}"`,
      ...(P && cnt && qty ? [`親 = ${P.name}を取得(件.${parent})`, `親.${cnt} = 親.${cnt} - 件.${qty}`] : []),
      `${dn}ID を返す`,
    ];
    const at = apply({ op: "add_action", name: act, params: [{ name: `${dn}ID`, type: "識別子" }], body, fail: fe });
    // 詳細の画面があれば、取り消していないときだけボタンを出す
    const ds = detailScreen(dn);
    const detail = ds?.d;
    if (detail) apply({ op: "add_line", kind: "screen", name: detail.name, text: `${dn}の${sf.name}が${cancelV}でないとき「${act}」を押すと ${act}(${dn}.ID)` });
    return checked(
      at,
      `動作「${act}」を作りました(状態を「${cancelV}」にする${P && cnt && qty ? `・${P.name}の${cnt}を戻す` : ""}・2 回目は「取消済み」で断る)` +
        (detail ? `。${ds!.made ? `詳細の画面「${detail.name}」を作り、` : `画面「${detail.name}」に`}ボタンを出しました` : `。ボタンを出すには、${dn}の詳細の画面を作ります(画面「${dn}詳細」を作る(${dn}の…を表示))`),
      o,
      text,
    );
  }
  // 「来店」で予約の人数を1増やす / 減らす
  if ((m = cmd(/^「(.+?)」で(.+?)の(.+?)を(\d*)(増やす|減らす)$/.exec(text)))) {
    const [, act, dn, fn, n, dir] = m.map((x) => x?.trim());
    return actionTemplate(o, text, act, dn, () => [`件 = ${dn}を取得(${dn}ID)`, `件.${fn} = 件.${fn} ${dir === "増やす" ? "+" : "-"} ${n || 1}`, `${dn}ID を返す`]);
  }
  // 「受け付ける」で問い合わせを登録する / 配車を登録できるようにする
  const regText = text.replace(/^(?:もう\s*1\s*つ|もうひとつ|別に)[、\s]*/, "");
  const reg = cmd(/^「(.+?)」で(.+?)を(?:登録|追加)(?:する)?$/.exec(regText)) ?? cmd(/^(.+?)を(?:登録|追加)(?:できるように(?:する|したい)?|したい)$/.exec(regText));
  // 「注文できるようにしたい」: データ名 + できるように → 「注文する」(第 5 回評価)
  const can = !reg ? cmd(/^(.+?)(?:が|を)?できるように(?:する|したい|して)?$/.exec(text)) : null;
  if ((reg && decl(o, "data", (reg[2] ?? reg[1]).trim())) || (can && decl(o, "data", can[1].trim()))) {
    const dn = reg ? (reg[2] ?? reg[1]).trim() : can![1].trim();
    const act = reg?.[2] ? reg[1].trim() : can ? `${dn}する` : `${dn}を追加`;
    return registration(o, ctx, text, decl(o, "data", dn), act);
  }
  // 見られる人の約束: 同じデータの前の約束(誰でも / ログインした人だけ / 本人だけ)を置き換える
  // (足すと「約束が矛盾しています」になり、消し間違えて電話番号が誰でも見える状態に戻った。第 6 回評価)
  {
    let vis: string | null = null;
    let dn = "";
    let mm: RegExpExecArray | null;
    if ((mm = /^(.+?)は(誰でも閲覧可|ログインした人だけ閲覧可|.+?が本人のものだけ閲覧可)$/.exec(text)) && decl(o, "data", mm[1].trim())) [dn, vis] = [mm[1].trim(), text];
    else if ((mm = /^(.+?)(?:を|は)(誰でも見られる|ログインした人だけ|本人だけ)(?:が見られる)?(?:ように)?(?:する|にする|に変える)?$/.exec(text)) && decl(o, "data", mm[1].trim())) {
      dn = mm[1].trim();
      const D = decl(o, "data", dn);
      const owner = D.fields.find((f: any) => f.type === "識別子" && !(o.decls ?? []).some((x: any) => x.kind === "data" && `${x.name}ID` === f.name) && f.name !== "ID");
      if (mm[2] === "本人だけ" && !owner) throw new Error(`「${dn}」に、だれのものかを表す項目(予約者・担当者 など。型は 識別子)がありません。先に「${dn}に「記入者」(識別子)を追加」としてください`);
      vis = mm[2] === "誰でも見られる" ? `${dn}は誰でも閲覧可` : mm[2] === "ログインした人だけ" ? `${dn}はログインした人だけ閲覧可` : `${dn}は${owner.name}が本人のものだけ閲覧可`;
    }
    if (vis) {
      // 前の約束の行をその場で置き換える(先に消すと、約束がその 1 行だけのとき中身の無い「約束:」が残った。第 7 回評価)
      const R = decl(o, "rules");
      if (R) {
        const ls = lines(R.file);
        const re = new RegExp(`^(\\s*)${reEsc(dn)}は(誰でも閲覧可|ログインした人だけ閲覧可|.+?が本人のものだけ閲覧可)\\s*$`);
        const hits = ls.map((l, k) => (k >= R.line && k < R.end && re.test(l) ? k : -1)).filter((k) => k >= 0);
        if (hits.length) {
          ls[hits[0]] = ls[hits[0]].replace(re, `$1${vis}`);
          const out = ls.filter((_l, k) => !hits.slice(1).includes(k));
          write(R.file, out.join("\n"));
          return checked({ file: R.file, line: hits[0] + 1 }, `約束を「${vis}」にしました(前の見られる人の約束と置き換えました)`, o, text);
        }
      }
      const at = apply({ op: "add_rule", text: vis });
      return checked(at, `約束を「${vis}」にしました`, o, text);
    }
  }
  // 文: 約束・確かめる・画面
  if (/(閲覧可|重複不可|の順|はいつでも)/.test(text) || text.startsWith("約束")) {
    const rule = text.replace(/^約束[::]?\s*/, "");
    const at = apply({ op: "add_rule", text: rule });
    // 同じデータの「重複不可」が前からあるときは知らせる(古い約束が残り、同じ日に 2 件書けなかった。第 8 回評価)
    const ud = /^(.+?)は.+の組で重複不可|^(.+?)は.+?が重複不可/.exec(rule);
    const dn = (ud?.[1] ?? ud?.[2])?.trim();
    const olds = dn
      ? (o.decls ?? []).filter((d: any) => d.kind === "rules").flatMap((d: any) => d.lines.map((l: any) => l.text.trim())).filter((t: string) => t.startsWith(`${dn}は`) && /重複不可$/.test(t) && t !== rule)
      : [];
    return checked(at, olds.length ? `約束に入れました。前からある「${olds.join("」「")}」も効いています(いらなければ、左の「約束」で消します)` : "約束に入れました", o, text);
  }
  // 確かめるの文: 「→」のある文に加えて、「連絡(連1)の状況は 確認済み」「花子から 連絡が 1件見える」も、
  // 確かめるを選んでいるときの文も、確かめるに入れる(上の欄に書くと読めなかった。第 10・11 回評価)
  if (/(→|->)/.test(text) || /^\S+[(（]\S+[)）]の\S+は\s*\S+$/.test(text) || /^\S+から\s*\S+が\s*\d+\s*件見える$/.test(text) || (ctx.scenario && decl(o, "scenario", ctx.scenario))) {
    const sc = (ctx.scenario && decl(o, "scenario", ctx.scenario)) || decl(o, "scenario");
    const at = sc ? apply({ op: "add_line", kind: "scenario", name: sc.name, text }) : apply({ op: "add_scenario", name: "動きの確認", lines: [text] });
    return checked(at, `確かめる「${sc?.name ?? "動きの確認"}」に入れました`, o, text);
  }
  const screens = (o.decls ?? []).filter((d: any) => d.kind === "screen").map((d: any) => d.name);
  const sc = ctx.screen && decl(o, "screen", ctx.screen);
  // 画面の文らしくなければ「どの画面?」と聞かず、言える言い方を示す(的外れに聞き返された。第 6 回評価)
  if (!sc && !/(表示|を押すと|を選ぶと|フォーム|集計|合計|一覧)/.test(text)) {
    const d0 = (o.decls ?? []).find((d: any) => d.kind === "data")?.name ?? "予約";
    const r0 = (o.decls ?? []).find((d: any) => d.kind === "role")?.name ?? "受付係";
    const f0 = (o.decls ?? []).find((d: any) => d.kind === "data")?.fields?.find((f: any) => f.name !== "ID")?.name ?? "名前";
    return {
      ok: false,
      errors: [{ code: "STUDIO", message: "この言い方は読めませんでした。下の言い方なら、そのまま操作になります(押すと欄に入ります)", hints: [] }],
      suggestions: [`${d0}に「メモ」を追加`, `${d0}の「${f0}」を「新しい名前」に変える`, `${d0}を登録できるようにしたい`, `${r0}に${d0}の更新を許可する`, `「${f0}」を空でもよいにする`, `${d0}の「メモ」を消す`],
    };
  }
  // 画面を選んでいなくても、文のデータを出している画面が 1 つに決まればそこに入れる(先に画面を選ぶと分からなかった。第 7 回評価)
  let target = sc;
  let guessed = false;
  if (!target) {
    const dn = (o.decls ?? []).filter((d: any) => d.kind === "data").map((d: any) => d.name).sort((a: string, b: string) => b.length - a.length).find((n: string) => text.startsWith(n));
    const cands = dn ? (o.decls ?? []).filter((d: any) => d.kind === "screen" && (d.lines ?? []).some((l: any) => l.text.startsWith(dn))) : [];
    if (cands.length === 1) {
      target = cands[0];
      guessed = true;
    }
  }
  if (!target) return { ok: false, need: "screen", screens, message: "どの画面に入れますか? 下の画面の名前を押すと、そこに入れます" };
  // 同じデータ・同じ絞り込みの一覧があるなら、並び方だけを変える(一覧が 2 つになった。第 7 回評価)
  const listOf = (t: string) => /^(.+?)(?:のうち(.+?)のもの)?を(?:.+?順に)?一覧表示$/.exec(t);
  const nl = listOf(text);
  if (nl) {
    const old = (target.lines ?? []).find((l: any) => {
      const ol = listOf(l.text);
      return ol && ol[1] === nl[1] && (ol[2] ?? "") === (nl[2] ?? "") && l.text !== text;
    });
    if (old) {
      const ls = lines(target.file);
      ls[old.line - 1] = ls[old.line - 1].replace(old.text, text);
      write(target.file, ls.join("\n"));
      return checked({ file: target.file, line: old.line }, `画面「${target.name}」の一覧を「${text}」に変えました(前の一覧と置き換えました)`, o, text);
    }
  }
  const at = apply({ op: "add_line", kind: "screen", name: target.name, text });
  return checked(at, `画面「${target.name}」に入れました${guessed ? "(別の画面に入れたいときは、左の一覧で画面を選んでから送ってください)" : ""}`, o, text);
}
/** 必須の項目を足したら、初期データの行に型に合った仮の値を入れる(入れないとアプリが起動できなかった。第 7 回評価) */
function fillSeed(o: any, dn: string, fn: string, t: string): void {
  const sp = join(PROJECT, "seed.json");
  if (!existsSync(sp)) return;
  let seed: any;
  try {
    seed = JSON.parse(readFileSync(sp, "utf8"));
  } catch {
    return;
  }
  const rows = seed?.[dn];
  if (!Array.isArray(rows) || rows.length === 0) return;
  const today = new Date(Date.now() + 9 * 3600_000).toISOString().slice(0, 10);
  const v = /^(整数|金額)/.test(t) ? 0 : t === "真偽" ? false : t === "日付" ? today : t === "日時" ? `${today}T09:00:00+09:00` : o.enums?.[t] ? o.enums[t][0] : t === "メール" ? "example@example.com" : t === "URL" ? "https://example.com/" : "未設定";
  let changed = false;
  for (const r of rows) if (r && typeof r === "object" && r[fn] === undefined) {
    r[fn] = v;
    changed = true;
  }
  if (changed) writeRaw("seed.json", JSON.stringify(seed, null, 2) + "\n");
}
/** 型を変えたら、初期データのその項目の値も新しい型に合わせる(合わない値は仮の値に)
 *  (型を変えると「初期データの誤り…整数が必要です」でアプリが起動しなかった。第 8 回評価) */
function convertSeed(o: any, dn: string, fn: string, t: string): void {
  const sp = join(PROJECT, "seed.json");
  if (!existsSync(sp)) return;
  let seed: any;
  try {
    seed = JSON.parse(readFileSync(sp, "utf8"));
  } catch {
    return;
  }
  const rows = seed?.[dn];
  if (!Array.isArray(rows)) return;
  const base = t.replace(/^選択<(.+)>$/, "$1");
  const enumVals: string[] | undefined = o.enums?.[base] ?? outline().enums?.[base];
  const today = new Date(Date.now() + 9 * 3600_000).toISOString().slice(0, 10);
  const conv = (v: any): any => {
    if (v === null || v === undefined) return v;
    if (/^(整数|金額)/.test(base)) {
      const n = Number(String(v).replace(/[,円¥\s]/g, ""));
      return Number.isFinite(n) ? Math.trunc(n) : 0;
    }
    if (base === "真偽") return v === true || /^(true|はい|真|1|yes)$/i.test(String(v));
    if (base === "日付") return /^\d{4}-\d{2}-\d{2}/.test(String(v)) ? String(v).slice(0, 10) : today;
    if (base === "日時") return /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}/.test(String(v)) ? String(v) : `${today}T09:00:00+09:00`;
    if (enumVals) return enumVals.includes(String(v)) ? String(v) : enumVals[0];
    if (base === "文字列" || base === "メール" || base === "URL") return String(v);
    return v;
  };
  let changedAny = false;
  for (const r of rows) {
    if (!r || typeof r !== "object" || !(fn in r)) continue;
    const nv = conv(r[fn]);
    if (JSON.stringify(nv) !== JSON.stringify(r[fn])) {
      r[fn] = nv;
      changedAny = true;
    }
  }
  if (changedAny) writeRaw("seed.json", JSON.stringify(seed, null, 2) + "\n");
}
/** 動作のすべての呼び出し(ボタン・確かめる・ほかの動作の中)の最後に値を足す */
function appendCallArg(act: string, value: string, idOf = ""): void {
  const o = outline();
  for (const f of srcFiles(o)) {
    const t = read(f);
    const all = t.split("\n");
    // 画面のボタンでは、ほかのデータの ID は画面に出ているデータから取る(「メニューIDの例」が入った。第 11 回評価)
    //   X の詳細の画面 → X.ID、X の ID を持つ Y の詳細の画面 → Y.XID
    const screenValue = (k: number): string | null => {
      if (!idOf) return null;
      let h = k;
      while (h > 0 && !/^\S/.test(all[h])) h--;
      if (!/^画面\s/.test(all[h])) return null;
      const body = [];
      for (let x = h + 1; x < all.length && !/^\S/.test(all[x]); x++) body.push(all[x].trim());
      if (body.some((b) => b.startsWith(`${idOf}の`) && b.endsWith("を表示"))) return `${idOf}.ID`;
      for (const b of body) {
        const mm = /^(\S+?)の.+を表示$/.exec(b);
        const Y = mm && decl(o, "data", mm[1]);
        if (Y && Y.fields.some((x: any) => x.name === `${idOf}ID`)) return `${Y.name}.${idOf}ID`;
      }
      return null;
    };
    const u = all
      .map((l, i) => {
        if (/^\S/.test(l)) return l;
        const m = new RegExp(`(^|[\\s(、,])${reEsc(act)}\\(`).exec(l);
        if (!m) return l;
        const k = m.index + m[1].length;
        return addParam(l, (/を押すと/.test(l) && screenValue(i)) || value, k);
      })
      .join("\n");
    if (u !== t) write(f, u);
  }
}
/** カッコの中を、外側の「,」で分ける(文字列・入れ子のカッコ・<…> の中は分けない) */
function splitTop(x: string): string[] {
  const parts: string[] = [];
  let cur = "";
  let d = 0;
  let q = false;
  for (const ch of x) {
    if (ch === '"') q = !q;
    if (!q && (ch === "(" || ch === "<" || ch === "{" || ch === "[")) d++;
    if (!q && (ch === ")" || ch === ">" || ch === "}" || ch === "]")) d--;
    if (!q && d === 0 && ch === ",") {
      parts.push(cur);
      cur = "";
    } else cur += ch;
  }
  parts.push(cur);
  return parts.map((p) => p.trim());
}
/** 動作の i 番目と j 番目の引数を入れ替える(見出しと、すべての呼び出し) */
function swapCallArgs(act: string, i: number, j: number, header: boolean): void {
  const o = outline();
  const swapAt = (l: string, k: number): string => {
    const start = k + act.length + 1;
    let depth = 0;
    for (let x = start - 1; x < l.length; x++) {
      if (l[x] === "(") depth++;
      else if (l[x] === ")" && --depth === 0) {
        const parts = splitTop(l.slice(start, x));
        if (Math.max(i, j) >= parts.length) return l;
        [parts[i], parts[j]] = [parts[j], parts[i]];
        return l.slice(0, start) + parts.join(", ") + l.slice(x);
      }
    }
    return l;
  };
  for (const f of srcFiles(o)) {
    const t = read(f);
    const u = t
      .split("\n")
      .map((l) => {
        const isHead = new RegExp(`^動作\\s+${reEsc(act)}\\(`).test(l);
        if (isHead) return header ? swapAt(l, l.indexOf(`${act}(`)) : l;
        if (/^\S/.test(l)) return l;
        // 呼び出し(ボタン・確かめる・ほかの動作の中)。名前の前が区切りのときだけ
        const m = new RegExp(`(^|[\\s(、,])${reEsc(act)}\\(`).exec(l);
        return m ? swapAt(l, m.index + m[1].length) : l;
      })
      .join("\n");
    if (u !== t) write(f, u);
  }
}
/** 確かめるの行の「動作名(…)」の呼び出しから、i 番目の値を外す(replace を渡すと、外さずに置き換える) */
function dropScenarioArg(act: string, idx: number, replace?: (v: string) => string): void {
  const o = outline();
  for (const f of srcFiles(o)) {
    const t = read(f);
    const u = t
      .split("\n")
      .map((l) => {
        if (!/(→|->)/.test(l) || /^\s*(動作|関数|アダプタ|外部関数)\s/.test(l)) return l;
        const k = l.indexOf(`${act}(`);
        if (k < 0) return l;
        let depth = 0;
        const start = k + act.length + 1;
        for (let i = start - 1; i < l.length; i++) {
          if (l[i] === "(") depth++;
          else if (l[i] === ")" && --depth === 0) {
            const parts: string[] = [];
            let cur = "";
            let d = 0;
            let q = false;
            for (const ch of l.slice(start, i)) {
              if (ch === '"') q = !q;
              if (!q && ch === "(") d++;
              if (!q && ch === ")") d--;
              if (!q && d === 0 && ch === ",") {
                parts.push(cur);
                cur = "";
              } else cur += ch;
            }
            parts.push(cur);
            if (idx >= parts.length) return l;
            if (replace === undefined) parts.splice(idx, 1);
            else parts[idx] = replace(parts[idx].trim());
            return l.slice(0, start) + parts.map((x) => x.trim()).join(", ") + l.slice(i);
          }
        }
        return l;
      })
      .join("\n");
    if (u !== t) write(f, u);
  }
}
/** 動作の見出しの引数の終わり(対応する「)」)に引数を足す(「文字列(任意)」の中に差し込んで壊した。第 7 回評価) */
function addParam(head: string, param: string, from = 0): string {
  const open = head.indexOf("(", from);
  if (open < 0) return head;
  let depth = 0;
  for (let i = open; i < head.length; i++) {
    if (head[i] === "(") depth++;
    else if (head[i] === ")" && --depth === 0) {
      const inner = head.slice(open + 1, i).trim();
      return head.slice(0, open + 1) + (inner ? inner + ", " : "") + param + head.slice(i);
    }
  }
  return head;
}
/** 動作の見出しの行から、名前がちょうど fn の引数を外す(「数」を外すとき「人数」を壊さない) */
function dropParam(head: string, fn: string): string {
  const open = head.indexOf("(");
  if (open < 0) return head;
  let depth = 0;
  let close = -1;
  for (let i = open; i < head.length; i++) {
    if (head[i] === "(" || head[i] === "<") depth++;
    else if ((head[i] === ")" || head[i] === ">") && --depth === 0 && head[i] === ")") {
      close = i;
      break;
    }
  }
  if (close < 0) return head;
  const parts: string[] = [];
  let cur = "";
  let d = 0;
  for (const ch of head.slice(open + 1, close)) {
    if (ch === "<" || ch === "(") d++;
    if (ch === ">" || ch === ")") d--;
    if (ch === "," && d === 0) {
      parts.push(cur);
      cur = "";
    } else cur += ch;
  }
  if (cur.trim()) parts.push(cur);
  const kept = parts.filter((x) => x.split(":")[0].trim() !== fn).map((x) => x.trim());
  return head.slice(0, open + 1) + kept.join(", ") + head.slice(close);
}
/** 項目を消す。登録の動作の項目名と、使われなくなった引数もそろえる */
function deleteField(o: any, D: any, fn: string): any {
  if (!D.fields.some((f: any) => f.name === fn)) throw new Error(`「${D.name}」に項目「${fn}」はありません(項目: ${D.fields.map((x: any) => x.name).join("、")})`);
  const ls = lines(D.file);
  const i = ls.findIndex((l, k) => k >= D.line && k < D.end && new RegExp(`^\\s*${reEsc(fn)}\\s*:`).test(l));
  ls.splice(i, 1);
  write(D.file, ls.join("\n"));
  for (const an of (outline().decls ?? []).filter((x: any) => x.kind === "action").map((x: any) => x.name)) {
    // 前の動作の行を消すと位置がずれるので、動作ごとに読み直す
    const a = (outline().decls ?? []).find((x: any) => x.kind === "action" && x.name === an);
    if (!a) continue;
    const al = lines(a.file);
    let hit = false;
    for (let k = a.line; k < a.end; k++) {
      if (!al[k]?.includes(`${D.name}を登録(`)) continue;
      const nl = al[k].replace(new RegExp(`(\\(|,\\s*)${reEsc(fn)}\\s*:\\s*[^,)]+(?:\\(\\))?`), (_x, p1: string) => (p1 === "(" ? "(" : "")).replace("(, ", "(");
      if (nl !== al[k]) {
        al[k] = nl;
        hit = true;
      }
    }
    // 引数がもう使われていなければ、引数も消す。確かめるの呼び出しからも、その位置の値を外す
    // (外さないと「引数は 6 個です」で消せなかった。第 7 回評価)
    // その引数を確かめるだけの行(「作業時間 > 0 を確認 または 失敗(…)」)も、項目と一緒に外す(残った。第 8 回評価)
    let removed = 0;
    if (hit) {
      for (let k = a.end - 1; k >= a.line; k--)
        // 「作業時間 > 0 を確認 …」のように、その値だけを確かめる行に限る(満席などほかの確認は残す)
        if (new RegExp(`^\\s*${reEsc(fn)}\\s*(>=|<=|>|<|==|!=|=)\\s*[^\\s]+\\s*を確認`).test(al[k] ?? "")) {
          al.splice(k, 1);
          removed++;
        }
    }
    const body = al.slice(a.line, a.end - removed).join("\n");
    let dropIdx = -1;
    if (hit && !new RegExp(`(^|[^\\p{L}\\p{N}_])${reEsc(fn)}([^\\p{L}\\p{N}_]|$)`, "u").test(body)) {
      dropIdx = (a.params ?? []).findIndex((p: any) => p.name === fn);
      al[a.line - 1] = dropParam(al[a.line - 1], fn);
    }
    if (hit) write(a.file, al.join("\n"));
    if (dropIdx >= 0) dropScenarioArg(a.name, dropIdx);
  }
  // 初期データからも、その項目の値を外す(残すと初期データの表を保存できなかった。第 8 回評価)
  {
    const sp = join(PROJECT, "seed.json");
    try {
      const seed = JSON.parse(readFileSync(sp, "utf8"));
      if (Array.isArray(seed?.[D.name]) && seed[D.name].some((r: any) => r && fn in r)) {
        for (const r of seed[D.name]) if (r && typeof r === "object") delete r[fn];
        writeRaw("seed.json", JSON.stringify(seed, null, 2) + "\n");
      }
    } catch {}
  }
  // 詳細の表示「Dの A と B を表示」からも外す(項目が 1 つだけの行は、その画面の最後の行でなければ消す)
  const o2 = outline();
  const re = new RegExp(`^(\\s+)${reEsc(D.name)}の(.+)を表示\\s*$`);
  for (const sc of (o2.decls ?? []).filter((d: any) => d.kind === "screen")) {
    const sl = lines(sc.file);
    let touched = false;
    for (let k = sc.end - 1; k >= sc.line; k--) {
      const mm = re.exec(sl[k] ?? "");
      if (!mm || /フォーム/.test(mm[2])) continue;
      const parts = mm[2].split("と").map((x) => x.trim());
      if (!parts.includes(fn)) continue;
      const rest = parts.filter((x) => x !== fn);
      const others = sl.slice(sc.line, sc.end).filter((l, j) => j + sc.line !== k && l.trim() && !l.trim().startsWith("#")).length;
      if (rest.length) sl[k] = `${mm[1]}${D.name}の${rest.join("と")}を表示`;
      else if (others > 0) sl.splice(k, 1);
      else continue;
      touched = true;
    }
    if (touched) write(sc.file, sl.join("\n"));
  }
  return changed(o, `「${D.name}」の項目「${fn}」を消しました`);
}
/** 名前の変更などの後: 誤りが増えたら全部戻して理由を返す */
function changed(before: any, done: string): any {
  const errs = (x: any) => (x.diagnostics ?? []).filter((d: any) => d.severity === "error");
  const was = new Set(errs(before).map((d: any) => d.message_ja));
  const added = errs(outline()).filter((d: any) => !was.has(d.message_ja));
  if (added.length === 0) return { ok: true, done };
  undo();
  return { ok: false, errors: added.slice(0, 3).map((d: any) => ({ code: d.code, message: `${d.file}:${d.span?.line} ${d.message_ja}`, hints: d.hints ?? [] })), suggestions: [] };
}
/** 登録のときに自動で入れる値(操作をした人 → 本人、登録日時 → 現在日時()、状況 → 最初の値)。無ければ null */
function autoValue(o: any, name: string, type: string): string | null {
  const VERB = "予約|申込|申し込み|記入|送信|作成|登録|注文|依頼|連絡|投稿|報告|記録|受付|出品|回答|訪問|発行|発券|入力|提出|申請|発注|購入|相談|応募|点検|実施|貸出|借用";
  // 操作をした人(お客様・利用者など、データを登録する本人を表す名前も。お客様が入力欄になった。第 9 回評価)
  const actor = new RegExp(`(${VERB})者$|^(本人|記入者|送信者|作成者|お客様|お客さん|客|利用者|会員|患者|申込者|参加者)$`).test(name);
  if (type === "識別子" && actor && !(o.decls ?? []).some((d: any) => d.kind === "data" && `${d.name}ID` === name)) return "本人";
  if (type === "日時" && new RegExp(`(${VERB})(日時|時刻)$`).test(name)) return "現在日時()";
  if (o.enums?.[type] && /(状況|状態|ステータス)$/.test(name)) return `"${o.enums[type][0]}"`;
  return null;
}
/** データ D を登録する動作(とフォーム)。本人・登録日時・状況は自動で入れ、ほかは入力にする */
function registration(o: any, ctx: { screen?: string }, text: string, D: any, act: string, only?: string[]): any {
  // 同じデータを、確認(満席・人数など)つきで登録している動作が既にあるときは、もう 1 つ作らない
  // (確認の無い登録の動作ができ、定員を超えて予約できた。第 8 回評価)。「もう 1 つ」と書けば作る
  // (動作の名前と項目をはっきり書いて作るときは、そのとおりに作る)
  if (!only && !/もう\s*1\s*つ|もうひとつ|別に/.test(text)) {
    const guarded = (o.decls ?? []).filter((a: any) => {
      if (a.kind !== "action") return false;
      const body = lines(a.file).slice(a.line, a.end).join("\n");
      return body.includes(`${D.name}を登録(`) && /を確認|失敗\(/.test(body);
    });
    if (guarded.length > 0 && !guarded.some((a: any) => a.name === act)) {
      const g = guarded[0].name;
      throw new Error(
        `「${D.name}」は、もう動作「${g}」で登録しています(満席などの確認つき)。新しい登録の動作を作ると、その確認を通らずに登録できてしまいます。` +
          `項目を足すときは「${D.name}に「メモ」を追加」、入力欄を足すときは「動作「${g}」に「メモ」を追加」と書きます(どうしても別に作るときは「もう 1 つ ${D.name}を登録できるようにしたい」)`,
      );
    }
  }
  const params: { name: string; type: string }[] = [];
  const args: string[] = [];
  for (const f of D.fields.filter((x: any) => x.name !== "ID")) {
    // 本人を自動で入れるのは、その操作をした人を表す名前の項目だけ(担当者・割当先は入力にする。第 7 回評価)
    const auto = autoValue(o, f.name, f.type);
    if (auto && !(only ?? []).includes(f.name)) args.push(`${f.name}: ${auto}`);
    else if (!only || only.includes(f.name) || !/^選択</.test(f.type)) {
      params.push({ name: f.name, type: f.type });
      args.push(`${f.name}: ${f.name}`);
    }
  }
  const at = apply({ op: "add_action", name: act, params, body: [`${D.name}を登録(${args.join(", ")}) を返す`] });
  const sc = (ctx.screen && decl(outline(), "screen", ctx.screen)) || null;
  if (sc) apply({ op: "add_line", kind: "screen", name: sc.name, text: `${act}のフォームを表示` });
  return checked(at, `動作「${act}」を作りました${sc ? `(画面「${sc.name}」にフォームを出しました)` : "。画面を選んでから「" + act + "のフォームを表示」と書くとフォームが出ます"}`, o, text);
}
/** D の詳細画面(「D の…を表示」がある画面)を探す。無くて、D を一覧表示している画面があれば作る
 *  (詳細の画面が無いうちに動作を作ると、ボタンがどこにも出なかった。第 10 回評価で 5 人)。
 *  一覧からのつながり(「D を選ぶと 詳細 へ」)は checked() の autoNav が足す */
function detailScreen(dn: string): { d: any; made: boolean } | null {
  const o = outline();
  const has = (d: any) => d.kind === "screen" && (d.lines ?? []).some((l: any) => l.text.startsWith(`${dn}の`) && l.text.endsWith("を表示") && !/フォーム/.test(l.text));
  const found = (o.decls ?? []).find(has);
  if (found) return { d: found, made: false };
  const listed = (o.decls ?? []).some((d: any) => d.kind === "screen" && (d.lines ?? []).some((l: any) => new RegExp(`^${reEsc(dn)}(を|のうち).*一覧表示`).test(l.text)));
  const D = decl(o, "data", dn);
  if (!listed || !D) return null;
  let name = `${dn}詳細`;
  for (let k = 2; decl(o, "screen", name); k++) name = `${dn}詳細${k}`;
  const shown = D.fields.map((f: any) => f.name).filter((n: string) => n !== "ID" && !/ID$/.test(n)).slice(0, 5);
  if (!shown.length) return null;
  apply({ op: "add_screen", name, lines: [`${dn}の${shown.join("と")}を表示`] });
  const d = decl(outline(), "screen", name);
  return d ? { d, made: true } : null;
}
/** 「…」で D の…: データ D の 1 件を ID で受け取る動作を作り、D の詳細画面にボタンを出す */
function actionTemplate(o: any, text: string, act: string, dn: string, body: (D: any) => string[]): any {
  const D = decl(o, "data", dn);
  if (!D) throw new Error(`データ「${dn}」はありません(データ: ${(o.decls ?? []).filter((d: any) => d.kind === "data").map((d: any) => d.name).join("、")})`);
  const at = apply({ op: "add_action", name: act, params: [{ name: `${dn}ID`, type: "識別子" }], body: body(D) });
  // 詳細画面(「D の…を表示」がある画面)にボタンを出す。無ければ作る
  const ds = detailScreen(dn);
  const detail = ds?.d;
  if (detail) apply({ op: "add_line", kind: "screen", name: detail.name, text: `「${act}」を押すと ${act}(${dn}.ID)` });
  return checked(at, `動作「${act}」を作りました${detail ? `(${ds!.made ? `詳細の画面「${detail.name}」を作り、` : `画面「${detail.name}」に`}ボタンを出しました)` : `。ボタンを出すには、${dn}の詳細の画面を作ります(画面「${dn}詳細」を作る(${dn}の…を表示))`}`, o, text);
}

/** 詳細画面の「表示する X がどれか決まりません」: X を一覧表示している画面に「X を選ぶと 詳細画面 へ」を足す
 * (詳細画面を作るたびにこの誤りが出て、直し方が分かりにくかった。第 7 回評価) */
function autoNav(): string[] {
  const added: string[] = [];
  for (let round = 0; round < 3; round++) {
    const o = outline();
    const need = (o.diagnostics ?? [])
      .map((d: any) => /画面 `(.+?)` で表示する (.+?) がどれか決まりません/.exec(d.message_ja ?? ""))
      .filter(Boolean) as RegExpExecArray[];
    if (need.length === 0) break;
    let progressed = false;
    for (const [, scr, dn] of need) {
      const lists = (o.decls ?? []).filter((d: any) => d.kind === "screen" && d.name !== scr && (d.lines ?? []).some((l: any) => new RegExp(`^${reEsc(dn)}(を|のうち).*一覧表示`).test(l.text)));
      const L = lists[0];
      if (!L || (L.lines ?? []).some((l: any) => l.text.startsWith(`${dn}を選ぶと `))) continue;
      apply({ op: "add_line", kind: "screen", name: L.name, text: `${dn}を選ぶと ${scr} へ` });
      added.push(`画面「${L.name}」に「${dn}を選ぶと ${scr} へ」`);
      progressed = true;
    }
    if (!progressed) break;
  }
  return added;
}
/** 入れた行に誤りがあれば元に戻して、理由と近い書き方を返す */
function checked(at: { file: string; line: number } | null, done: string, before: any, text: string): any {
  const nav = autoNav();
  if (nav.length) done += `(一覧から開けるように、${nav.join("、")}を足しました)`;
  const o = outline();
  const bad = (o.diagnostics ?? []).filter((d: any) => d.severity === "error" && at && d.span?.line === at.line && d.file === at.file);
  if (bad.length === 0) {
    // 入れた行は正しいが、ほかの所が誤りになった(項目を足したら登録の動作に値が要る など)。黙らずに知らせる(第 5 回評価)
    const errs = (x: any) => (x.diagnostics ?? []).filter((d: any) => d.severity === "error");
    const was = new Set(errs(before).map((d: any) => `${d.file}:${d.span?.line}:${d.message_ja}`));
    const added = errs(o).filter((d: any) => !was.has(`${d.file}:${d.span?.line}:${d.message_ja}`));
    if (added.length === 0) return { ok: true, done, at };
    const first = added[0];
    const tip = /がありません/.test(first.message_ja) && /登録/.test(first.message_ja) ? "足した項目を空でもよい項目(型の後ろに「(任意)」)にするか、登録の行にもその項目を書きます。" : "「取り消す」で戻せます。";
    return { ok: true, done: `${done}。ただし、ほかの ${added.length} か所が誤りになりました: ${first.file}:${first.span?.line} ${first.message_ja} — ${tip}`, at };
  }
  undo();
  const hints: string[] = bad.flatMap((d: any) => d.hints ?? []);
  const fromHints = hints.flatMap((h) => [...h.matchAll(/「([^」]{2,80})」|`([^`]{2,80})`/g)].map((x) => x[1] ?? x[2]));
  const pats = [...(before.patterns?.screen ?? []), ...(before.patterns?.rules ?? [])];
  // 書ける文のうち、書いた文に近いもの(文字の並びの重なりで比べる)
  const near = [...new Set(pats)].map((p: string) => ({ p, s: similar(text, p) })).sort((x, y) => y.s - x.s).slice(0, 5).map((x) => x.p);
  return { ok: false, errors: bad.map((d: any) => ({ code: d.code, message: d.message_ja, hints: d.hints ?? [] })), suggestions: [...new Set([...fromHints.filter((s) => !/[{}]/.test(s)), ...near])].slice(0, 6) };
}
function undo(): boolean {
  const last = history[history.length - 1];
  if (!last) return false;
  while (history.length && history[history.length - 1].g === last.g) {
    const h = history.pop()!;
    wroteTime = Date.now();
    queueMicrotask(retryAfterFix);
    atomicWrite(h.raw ? h.file : srcPath(h.file), h.text);
  }
  return true;
}

// ---- 見た目の確認(koto dev を動かしておく) ----
let previewPort = 0;
let preview: ChildProcess | null = null;
let devLog = "";
/** 確かめるの結果(または誤り)が最後に出た時刻 */
let checksTime = 0;
let lastRetrigger = 0;
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
  // 起こし直すときも同じポートを使う(番号が変わって、開いていたタブがつながらなくなった。第 7 回評価)
  if (!previewPort) previewPort = await freePort();
  preview = spawn(KOTO, ["-C", PROJECT, "dev", "--port", String(previewPort)], {
    // 止めるときに裏のアプリ(backend・画面)ごと止められるよう、プロセスのまとまりにする(止めても残った。第 7 回評価)
    detached: process.platform !== "win32",
    // Studio が自分で起動する koto dev は、利用者が打ったコマンドとして記録しない(第 7 回の記録で、黒い画面を使ったように見えた)
    env: { ...process.env, KOTONOHA_USAGE_LOG: "", KOTONOHA_DEV_FRAME_ANCESTOR: `http://127.0.0.1:${PORT} http://localhost:${PORT}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let runLog = "";
  devFailure = "";
  const keep = (b: Buffer) => {
    const chunk = b.toString("utf8");
    devLog = (devLog + chunk).slice(-200_000);
    runLog = (runLog + chunk).slice(-20_000);
    if (/すべて確かめました|\d+ 件が期待と違いました|エラー\[/.test(chunk)) checksTime = Date.now();
    if (/すべて確かめました|\d+ 件が期待と違いました|http:\/\/127\.0\.0\.1:\d+\//.test(chunk)) restarts = 0;
  };
  preview.stdout?.on("data", keep);
  preview.stderr?.on("data", keep);
  previewAlive = true;
  // 表示用のサーバーが止まったら起こし直す(止まったまま「試しています…」で固まった。第 5 回評価)
  preview.once("exit", (code) => {
    previewAlive = false;
    if (stopping) return;
    // 初期データの誤りや生成の誤りで止まったときは、起こし直しても同じ所で止まる(2 秒ごとに起動を繰り返し、
    // 「動いています」と「起動しています…」が入れ替わり続けた。第 7 回評価)。理由を出し、次に直したときに起こす
    // 分かる言葉の「初期データの誤り: …」があればそれを、無ければ検査の誤りを出す
    const seedErr = runLog.lastIndexOf("初期データの誤り");
    const i = seedErr >= 0 ? seedErr : runLog.lastIndexOf("エラー[");
    if (code && i >= 0) {
      devFailure = runLog
        .slice(i)
        .split("\n")
        .filter((l) => l.trim() && !/^\s+at /.test(l) && !/^Node\.js v/.test(l) && !/^\(エラーの詳しい説明|^エラー \d+ 件、警告/.test(l.trim()) && !(seedErr >= 0 && /INT001/.test(l)))
        .slice(0, 3)
        .join(" ")
        .slice(0, 400);
      devLog += "\n(アプリを起動できませんでした。直すと起動し直します)\n";
      return;
    }
    devLog += "\n(表示用のサーバーが止まりました。起動し直します)\n";
    restarts += 1;
    setTimeout(() => void startPreview(), Math.min(10_000, 1000 * restarts));
  });
}
/** koto dev と、その下で動くアプリ(backend・画面)をまとめて止める */
function killPreview(): void {
  if (!preview?.pid) return;
  try {
    if (process.platform !== "win32") process.kill(-preview.pid, "SIGTERM");
    else spawnSync("taskkill", ["/pid", String(preview.pid), "/T", "/F"]);
  } catch {
    try {
      preview.kill();
    } catch {}
  }
}
let previewAlive = false;
let restarts = 0;
/** 起動できずに待っている理由(空なら待っていない) */
let devFailure = "";
let retryTimer: ReturnType<typeof setTimeout> | null = null;
/** ソースや初期データを直したら、起動できずに待っているアプリを起こし直す */
function retryAfterFix(): void {
  if (!devFailure || stopping || previewAlive || retryTimer) return;
  retryTimer = setTimeout(() => {
    retryTimer = null;
    if (devFailure && !stopping && !previewAlive) void startPreview();
  }, 400);
}
let stopping = false;
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
function seedState(o: any): any {
  let seed: any = {};
  try {
    seed = JSON.parse(readFileSync(join(PROJECT, "seed.json"), "utf8"));
  } catch {}
  return (o.decls ?? [])
    .filter((d: any) => d.kind === "data")
    .map((d: any) => ({ data: d.name, fields: d.fields.map((f: any) => ({ name: f.name, type: f.type })), rows: Array.isArray(seed[d.name]) ? seed[d.name] : [] }));
}
function state(): any {
  if (isHome()) return { home: true, folder: PROJECT.split(sep).pop(), templates: TEMPLATES };
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
  // 消した「確かめる」の古い結果は出さない(消した後も ✗ が残った。第 6 回評価)
  const names = new Set((o.decls ?? []).filter((d: any) => d.kind === "scenario").map((d: any) => d.name));
  const raw = lastChecks();
  const keep: string[] = [];
  let on = true;
  for (const l of raw.lines) {
    const h = /^確かめる「(.+?)」/.exec(l.trim());
    if (h) on = names.has(h[1]);
    if (on) keep.push(l);
  }
  // アプリの起動中に試した行(fetch failed)は、失敗ではなく「起動中」と出す(✗ が並んで不安になった。第 7 回評価)
  const starting = keep.some((l) => /fetch failed|ECONNREFUSED/.test(l));
  // アプリの起動中に試して失敗したまま、試し直されないことがある(「起動し直しています…」で止まった。第 9 回評価)。
  // 少し待っても新しい結果が無ければ、ソースの更新時刻だけを進めて koto dev に試し直させる(中身は変えない)
  if (starting && previewAlive && Date.now() - checksTime > 15_000 && Date.now() - lastRetrigger > 30_000) {
    lastRetrigger = Date.now();
    try {
      const f = (o.files ?? [])[0];
      if (f) utimesSync(srcPath(f), new Date(), new Date());
    } catch {}
  }
  const shown = starting ? ["アプリを起動し直しています。少し待つと、もう一度試します…"] : keep;
  // 直した後、試し直すまでの間は前の結果だと分かるようにする(名前を変えた後に古い名前の結果が残った。第 7 回評価)
  // アプリが起動できずに止まっている間も、前の結果だと分かるようにする(全部 ✓ に見えた。第 8 回評価)
  const stale = (previewAlive && o.ok && wroteTime > checksTime && Date.now() - wroteTime < 90_000) || (!!devFailure && !previewAlive);
  const checks = names.size === 0 ? { lines: [], ok: null } : { lines: shown, stale, ok: starting || stale ? null : keep.some((l) => l.trim().startsWith("✗")) ? false : raw.ok };
  // アプリが起動に失敗しているとき(初期データの誤りなど)は、その理由を出す。「動いています」と出していた(第 7 回評価)
  const lastErr = Math.max(devLog.lastIndexOf("エラー["), devLog.lastIndexOf("初期データの誤り"));
  // 正常に動き出した印(確かめるの結果・待ち受けの表示)の、いちばん後ろの位置
  // (最初の URL の行しか見ておらず、確かめるが無いアプリでは直っても「起動できません」が消えなかった。第 8 回評価)
  const lastOk = Math.max(devLog.lastIndexOf("確かめる「"), devLog.lastIndexOf("すべて確かめました"), devLog.lastIndexOf("件が期待と違いました"), devLog.lastIndexOf("web listening on"), devLog.lastIndexOf("  画面:   http://"));
  const devError = devFailure && !previewAlive ? devFailure : previewAlive && lastErr >= 0 && lastErr > Math.max(lastOk, devLog.lastIndexOf("ソースの変更を"))
    ? devLog.slice(lastErr).split("\n").filter((l) => l.trim() && !/^\s+at /.test(l) && !/koto explain|^エラー \d+ 件、警告|^Node\.js v/.test(l.trim())).slice(0, 3).join(" ").slice(0, 400)
    : "";
  return { outline: o, sources, devError, previewPort: previewAlive ? previewPort : 0, previewNote: previewAlive ? "" : stopping ? "アプリは止めてあります(「動かす」で動きます)" : devFailure ? "アプリを起動できません。直すと起動し直します" : "表示用のサーバーを起動しています…", running: previewAlive, checks, canUndo: history.length > 0, theme, seed: seedState(o), published: existsSync(join(PROJECT, "公開用", "点検の結果.md")) ? readFileSync(join(PROJECT, "公開用", "点検の結果.md"), "utf8") : "" };
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
      group += 1;
      try {
        if (url.pathname === "/api/op") {
          // ソースを直接直した行数(手で書いた量)を記録する
          let edited = 0;
          if (b.op === "save_source") {
            try {
              const before = new Map<string, number>();
              for (const l of read(String(b.file)).split("\n")) before.set(l, (before.get(l) ?? 0) + 1);
              for (const l of String(b.text ?? "").split("\n")) {
                const n = before.get(l) ?? 0;
                if (n > 0) before.set(l, n - 1);
                else if (l.trim()) edited++;
              }
            } catch {}
          }
          const at = apply(b);
          usage({ api: "op", op: b.op, ok: true, edited });
          return send(res, 200, { ok: true, at, ...state() });
        }
        if (url.pathname === "/api/instruct") {
          const r = instruct(String(b.text ?? ""), { screen: b.screen, scenario: b.scenario });
          usage({ api: "instruct", ok: !!r.ok, need: r.need ?? null });
          return send(res, 200, { result: r, ...state() });
        }
        if (url.pathname === "/api/undo") {
          const ok = undo();
          usage({ api: "undo", ok });
          return send(res, 200, { ok, ...state() });
        }
        // 新しいアプリを作る(プロジェクトが無いフォルダで開いたとき)
        if (url.pathname === "/api/create") {
          if (!isHome()) throw new Error("このフォルダには、もうアプリがあります");
          const name = String(b.name ?? "").trim();
          if (!name || /[「」"\n\\]/.test(name) || name.length > 40) throw new Error("アプリの名前を 40 文字までで入れてください(「」は使えません)");
          const tpl = String(b.template ?? "");
          if (tpl && !TEMPLATES.some((t) => t.name === tpl)) throw new Error("そのひな形はありません");
          const r = spawnSync(KOTO, ["init", PROJECT, "--name", name, ...(tpl ? ["--template", tpl] : [])], { encoding: "utf8", env: { ...process.env, KOTONOHA_USAGE_LOG: "" } });
          usage({ api: "create", template: tpl, ok: r.status === 0 });
          if (r.status !== 0) throw new Error((r.stderr || r.stdout || "作れませんでした").trim());
          await startPreview();
          return send(res, 200, { ok: true, ...state() });
        }
        // アプリを動かす・止める
        if (url.pathname === "/api/app") {
          if (b.action === "stop") {
            stopping = true;
            killPreview();
            previewAlive = false;
          } else if (b.action === "start" && !previewAlive) {
            stopping = false;
            restarts = 0;
            await startPreview();
          }
          usage({ api: "app", action: b.action });
          return send(res, 200, { ok: true, ...state() });
        }
        // 初期データを表で直す(seed.json。Excel から貼り付けた表も受け取る)
        if (url.pathname === "/api/seed") {
          const o = outline();
          const D = decl(o, "data", String(b.data ?? ""));
          if (!D) throw new Error(`データ「${b.data}」はありません`);
          const rows = Array.isArray(b.rows) ? b.rows : [];
          const types: Record<string, string> = Object.fromEntries(D.fields.map((f: any) => [f.name, f.type]));
          const clean = rows.map((r: any, i: number) => {
            const out: any = {};
            for (const [k, v0] of Object.entries(r ?? {})) {
              if (!(k in types)) throw new Error(`「${D.name}」に項目「${k}」はありません`);
              const v = typeof v0 === "string" ? v0.trim() : v0;
              if (v === "" || v === null || v === undefined) continue;
              const t = types[k].replace(/^選択<(.+)>$/, "$1");
              if (/^(整数|金額)/.test(t)) {
                const n = Number(String(v).replace(/[,¥円]/g, ""));
                if (!Number.isInteger(n)) throw new Error(`${i + 1} 行目の「${k}」は整数で入れます(入っていた値: ${v})`);
                out[k] = n;
              } else if (t === "真偽") out[k] = v === true || /^(true|真|はい|○|1|済|有)$/.test(String(v));
              else if (t === "日時" || t === "日付") {
                // Excel の書き方(2026/11/15 11:05・2026年11月15日)も受け取る(「タイムゾーン付きの ISO 8601」と出て分からなかった。第 8 回評価)
                const sv = String(v);
                if (t === "日時" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?([+-]\d{2}:\d{2}|Z)$/.test(sv)) {
                  out[k] = sv;
                  continue;
                }
                const mm = /^(\d{4})\s*[-/年.]\s*(\d{1,2})\s*[-/月.]\s*(\d{1,2})\s*日?(?:[ T　]+(\d{1,2})\s*[:時]\s*(\d{1,2})\s*分?(?::(\d{1,2}))?)?$/.exec(sv);
                if (!mm) throw new Error(`${i + 1} 行目の「${k}」は ${t === "日時" ? "2026/12/05 19:00" : "2026/12/05"} のような形で入れます(入っていた値: ${sv})`);
                const p2 = (x: string | undefined) => String(x ?? "0").padStart(2, "0");
                const day = `${mm[1]}-${p2(mm[2])}-${p2(mm[3])}`;
                out[k] = t === "日付" ? day : `${day}T${p2(mm[4])}:${p2(mm[5])}:${p2(mm[6])}+09:00`;
              } else out[k] = String(v);
            }
            if (Object.keys(out).length === 0) return null;
            if (!out.ID) out.ID = `${D.name}${i + 1}`;
            return out;
          }).filter((x: any) => x !== null);
          let seed: any = {};
          const sp = join(PROJECT, "seed.json");
          try {
            seed = JSON.parse(readFileSync(sp, "utf8"));
          } catch {}
          seed[D.name] = clean;
          if (existsSync(sp)) writeRaw("seed.json", JSON.stringify(seed, null, 2) + "\n");
          else writeFileSync(sp, JSON.stringify(seed, null, 2) + "\n");
          usage({ api: "seed", rows: clean.length });
          return send(res, 200, { ok: true, ...state() });
        }
        // 公開の準備
        if (url.pathname === "/api/publish") {
          const r = koto(["publish"]);
          usage({ api: "publish", ok: r.code === 0 });
          if (r.code !== 0) throw new Error((r.err.split("\n").filter((l) => /エラー|できません|合格/.test(l)).slice(-3).join(" ") || "公開の準備ができませんでした").trim());
          return send(res, 200, { ok: true, done: r.out.trim(), ...state() });
        }
      } catch (e) {
        usage({ api: url.pathname.slice(5), ok: false, error: String((e as Error).message ?? e).slice(0, 200) });
        let st: any = {};
        try {
          st = state();
        } catch {}
        return send(res, 200, { ok: false, error: friendlyError(e), ...st });
      }
    }
    send(res, 404, { error: "not found" });
  } catch (e) {
    send(res, 500, { error: friendlyError(e) });
  }
});

/** OS の英語の誤り(ENOSPC など)を、分かる言葉にする(英語の「ENOSPC」で止まった。第 8 回評価) */
function friendlyError(e: unknown): string {
  const code = (e as { code?: string })?.code ?? "";
  const msg = String((e as Error)?.message ?? e);
  if (code === "ENOSPC" || /ENOSPC/.test(msg)) return "パソコンの空き容量が足りないため、保存できませんでした。いらないファイルを消して空きを作ってから、もう一度押してください(ソースは前のまま残っています)";
  if (code === "EACCES" || code === "EPERM" || /EACCES|EPERM/.test(msg)) return "このフォルダに書き込む許可がないため、保存できませんでした。フォルダの場所や許可を確かめてください";
  if (code === "ENOENT" && /seed|\.koto|\.json/.test(msg)) return `ファイルが見つかりません(${msg.split("'")[1] ?? msg})。Studio を開き直してください`;
  return msg;
}
const parent = Number(process.env.KOTONOHA_EXIT_WITH_PARENT ?? 0);
const stop = () => {
  stopping = true;
  killPreview();
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
  // ダブルクリックで開いたときは、ブラウザも開く
  if (process.env.KOTONOHA_OPEN_BROWSER === "1") {
    const u = `http://127.0.0.1:${PORT}/`;
    const [cmd, args] = process.platform === "darwin" ? ["open", [u]] : process.platform === "win32" ? ["cmd", ["/c", "start", "", u]] : ["xdg-open", [u]];
    try {
      spawn(cmd, args as string[], { stdio: "ignore", detached: true }).unref();
    } catch {}
  }
});
