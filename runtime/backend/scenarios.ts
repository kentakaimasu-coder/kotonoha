// 生成物: ソースの「確かめる」(ADR-0130)を実行する。node --no-warnings tools/scenarios.ts
// 確かめるごとに新しいデータベース(初期データ KOTONOHA_SEED があれば投入)でサーバーを起動し、
// 日本語で書いた手順を上から順に API で試して、結果を日本語で表示する。
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";
import { createServer } from "node:net";
import { signToken } from "../src/runtime/auth.ts";
import { openDb, q } from "../src/runtime/db.ts";
import { tables } from "../src/app.generated.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const scenarios = JSON.parse(readFileSync(join(root, "tests/scenarios.json"), "utf8"));
const api = JSON.parse(readFileSync(join(root, "api.json"), "utf8"));
const seed = process.env.KOTONOHA_SEED && existsSync(process.env.KOTONOHA_SEED) ? process.env.KOTONOHA_SEED : null;

// 役割を書かなかった人(一般の利用者)。役割の無いアプリでは、全件閲覧のほかの全部の権限。
// 役割のあるアプリでは、本番(役割の権限 + 認証基盤が直接渡す権限)に合わせて、どの役割にも書いていない権限だけ(ADR-0140)
const roleCaps = new Set<string>(Object.values((api.roles ?? {}) as Record<string, string[]>).flat());
const hasRoles = Object.keys(api.roles ?? {}).length > 0;
/** その動作ができる役割(直し方の案内に出す。先頭の役割を出していて、日報を書けない「所長」を勧めた。第 19 回評価) */
function roleFor(action: string): string {
  const e = api.endpoints.find((x: any) => x.path === `/api/actions/${action}`);
  const roles = Object.entries((api.roles ?? {}) as Record<string, string[]>);
  const fit = e && roles.find(([, cs]) => (e.access.grant && cs.includes(e.access.grant)) || (e.access.capabilities ?? []).every((c: string) => cs.includes(c)));
  return (fit ?? roles[0])[0];
}
const general = new Set<string>();
for (const e of api.endpoints) {
  const caps: string[] = e.access.capabilities ?? [];
  // 役割に関わる操作(必要な権限か動作の許可が、どれかの役割に書いてある)は、動作の許可も持たない
  const restricted = caps.some((c) => roleCaps.has(c)) || (e.access.grant && roleCaps.has(e.access.grant));
  for (const c of caps) if (!String(c).startsWith("全件閲覧.") && !roleCaps.has(c)) general.add(c);
  if (e.access.grant && !restricted) general.add(e.access.grant);
}
const port = () => new Promise<number>((ok) => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const p = (s.address() as { port: number }).port; s.close(() => ok(p)); }); });
const enc = encodeURIComponent;
const show = (v: unknown) => (typeof v === "string" ? v : JSON.stringify(v));

let failed = 0;
for (const sc of scenarios) {
  const dir = mkdtempSync(join(tmpdir(), "koto-check-"));
  const db = join(dir, "db.sqlite");
  const secret = randomBytes(32).toString("hex");
  const env = { ...process.env, KOTONOHA_DB: db, KOTONOHA_JWT_SECRET: secret, KOTONOHA_RATE_LIMIT_PER_MINUTE: "0" };
  if (seed) execFileSync(process.execPath, ["--no-warnings", "tools/seed.ts", seed], { cwd: root, env, stdio: "ignore" });
  const p = await port();
  const url = `http://127.0.0.1:${p}`;
  const proc: ChildProcess = spawn(process.execPath, ["--no-warnings", "src/main.ts"], { cwd: root, env: { ...env, PORT: String(p) }, stdio: "ignore" });
  for (let i = 0; i < 200; i++) {
    try { if ((await fetch(url + "/api/health")).ok) break; } catch {}
    await new Promise((r) => setTimeout(r, 50));
  }
  const token = (who: any): string | null => {
    if (!who.sub) return null;
    const claims: Record<string, unknown> = { sub: who.sub, exp: Math.floor(Date.now() / 1000) + 600 };
    if (who.role) claims.roles = [who.role];
    else claims.caps = [...general];
    return signToken(claims, secret);
  };
  // 「…から D が n件見える」は、この確かめるの中で作った行だけを数える(初期データの行は数えない)。
  // 初期データを入れ替えても結果が変わらないようにする(Studio と koto test で ✓ / ✗ が食い違った。第 9 回評価)
  const initial = new Map<string, Set<string>>();
  {
    const d = await openDb(db, tables);
    try {
      for (const t of tables) {
        try {
          const rows: any[] = await d.conn.all(`SELECT "ID" AS id FROM ${q(t.name)}`, []);
          initial.set(t.name, new Set(rows.map((r) => String(r.id))));
        } catch {}
      }
    } finally {
      await d.close();
    }
  }
  console.log(`確かめる「${sc.name}」`);
  // `→ 成功(名前)` で名付けた ID(後の行の {"$ref": 名前} を置き換える)
  const saved = new Map<string, unknown>();
  const resolve = (v: any): unknown => (v !== null && typeof v === "object" && typeof v.$ref === "string" ? saved.get(v.$ref) ?? `(${v.$ref} はまだ作られていません)` : v);
  for (const st of sc.steps) {
    let ok = false;
    let got = "";
    try {
      if (st.kind === "action") {
        const t = token(st.who);
        const headers: Record<string, string> = { "Content-Type": "application/json" };
        if (t) headers.Authorization = `Bearer ${t}`;
        const body = Object.fromEntries(Object.entries(st.body ?? {}).map(([k, v]) => [k, resolve(v)]));
        const r = await fetch(`${url}/api/actions/${enc(st.action)}`, { method: "POST", headers, body: JSON.stringify(body) });
        const b: any = await r.json().catch(() => null);
        if (st.expect.save && r.status === 200) saved.set(st.expect.save, b?.ok);
        ok = r.status === st.expect.status && (st.expect.value === undefined || b?.value === st.expect.value) && (st.expect.error === undefined || b?.error === st.expect.error);
        got = r.status === 200 ? "成功" : r.status === 422 ? `失敗(${show(b?.value)})` : r.status === 409 && b?.error === "invalid_transition" ? `順番が違う(${b?.message ?? ""})` : r.status === 409 ? `重複(${b?.message ?? ""})` : `${r.status} ${b?.message ?? ""}`.trim();
        // 役割を書かなかった人が権限で断られたとき: 役割のあるアプリでは、役割の権限を持たない(ADR-0140)
        if (!ok && r.status === 403 && st.who.sub && !st.who.role && hasRoles)
          got += `。役割を書かない人は、どの役割にも書いていない権限だけを持ちます(本番と同じ)。「${st.who.sub}(${roleFor(st.action)})が …」のように役割を書きます`;
      } else if (st.kind === "count") {
        const t = token(st.who);
        const r = await fetch(`${url}/api/data/${enc(st.data)}?limit=1000`, { headers: t ? { Authorization: `Bearer ${t}` } : {} });
        if (r.status === 404) got = `${st.data} の一覧の画面が無いため数えられません(画面に「${st.data}を一覧表示」を書いてください)`;
        else {
          const b: any = await r.json().catch(() => null);
          const before = initial.get(st.data) ?? new Set<string>();
          const n = Array.isArray(b) ? b.filter((x: any) => !before.has(String(x?.ID))).length : null;
          ok = n === st.count;
          got = n === null ? `${r.status} ${b?.message ?? ""}`.trim() : `${n}件`;
        }
      } else if (st.kind === "field") {
        const d = await openDb(db, tables);
        try {
          const id = resolve(st.id);
          const rows: any[] = await d.conn.all(`SELECT ${st.sql ?? q(st.field)} AS v FROM ${q(st.data)} WHERE "ID" = ?`, [id]);
          if (rows.length === 0) got = `${st.data}「${typeof st.id === "string" ? st.id : st.id.$ref}」がありません`;
          else {
            let v: unknown = rows[0].v;
            if (typeof v === "bigint") v = Number(v);
            if (typeof st.value === "boolean") v = v === 1 || v === true;
            ok = JSON.stringify(v) === JSON.stringify(st.value);
            got = show(v);
          }
        } finally {
          await d.close();
        }
      }
    } catch (e) {
      got = String((e as Error)?.message ?? e);
    }
    if (ok) console.log(`  ✓ ${st.text}`);
    else {
      failed++;
      console.log(`  ✗ ${st.text}  (実際は: ${got})`);
    }
  }
  // Windows ではサーバーが終わるまでデータベースのファイルが消せないため、終了を待ってから片付ける
  const exited = new Promise<void>((ok) => (proc.exitCode !== null ? ok() : proc.once("exit", () => ok())));
  proc.kill();
  await Promise.race([exited, new Promise((r) => setTimeout(r, 5000))]);
  try {
    rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  } catch {
    // 一時フォルダが残っても確かめた結果には関係しない
  }
}
console.log(failed === 0 ? "すべて確かめました" : `${failed} 件が期待と違いました`);
process.exit(failed === 0 ? 0 : 1);
