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

// 役割を書かなかった人(一般の利用者): ソースのすべての操作の権限(全件閲覧は除く)
const general = new Set<string>();
for (const e of api.endpoints) {
  for (const c of e.access.capabilities ?? []) if (!String(c).startsWith("全件閲覧.")) general.add(c);
  if (e.access.grant) general.add(e.access.grant);
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
  console.log(`確かめる「${sc.name}」`);
  for (const st of sc.steps) {
    let ok = false;
    let got = "";
    try {
      if (st.kind === "action") {
        const t = token(st.who);
        const headers: Record<string, string> = { "Content-Type": "application/json" };
        if (t) headers.Authorization = `Bearer ${t}`;
        const r = await fetch(`${url}/api/actions/${enc(st.action)}`, { method: "POST", headers, body: JSON.stringify(st.body) });
        const b: any = await r.json().catch(() => null);
        ok = r.status === st.expect.status && (st.expect.value === undefined || b?.value === st.expect.value);
        got = r.status === 200 ? "成功" : r.status === 422 ? `失敗(${show(b?.value)})` : `${r.status} ${b?.message ?? ""}`.trim();
      } else if (st.kind === "count") {
        const t = token(st.who);
        const r = await fetch(`${url}/api/data/${enc(st.data)}?limit=1000`, { headers: t ? { Authorization: `Bearer ${t}` } : {} });
        if (r.status === 404) got = `${st.data} の一覧の画面が無いため数えられません(画面に「${st.data}を一覧表示」を書いてください)`;
        else {
          const b: any = await r.json().catch(() => null);
          const n = Array.isArray(b) ? b.length : null;
          ok = n === st.count;
          got = n === null ? `${r.status} ${b?.message ?? ""}`.trim() : `${n}件`;
        }
      } else if (st.kind === "field") {
        const d = await openDb(db, tables);
        try {
          const rows: any[] = await d.conn.all(`SELECT ${q(st.field)} AS v FROM ${q(st.data)} WHERE "ID" = ?`, [st.id]);
          if (rows.length === 0) got = `${st.data}「${st.id}」がありません`;
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
  proc.kill();
  rmSync(dir, { recursive: true, force: true });
}
console.log(failed === 0 ? "すべて確かめました" : `${failed} 件が期待と違いました`);
process.exit(failed === 0 ? 0 : 1);
