// verify_android_jvm.sh の補助: 見本の backend を生成・起動し、一覧と集計の JSON を保存する
import { execFileSync, spawn } from "node:child_process";
import { writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";

const work = process.argv[2];
const root = resolve(import.meta.dirname, "..");
const dir = mkdtempSync(join(tmpdir(), "kotonoha-android-be-"));
execFileSync(join(root, "target/debug/koto"), ["-C", join(root, "examples/shop"), "build", "--target", "backend", "--out", dir], { stdio: "ignore" });
const port = await new Promise((ok) => { const s = createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => ok(p)); }); });
const env = { ...process.env, KOTONOHA_JWT_SECRET: "android-jvm-verify-secret-0123456789", KOTONOHA_DB: join(dir, "db.sqlite"), PORT: String(port) };
const base = Date.parse("2026-11-02T15:00:00Z");
writeFileSync(join(dir, "seed.json"), JSON.stringify({
  商品: [{ ID: "P1", 名前: "緑茶", 価格: 1200, 在庫: 3 }, { ID: "P2", 名前: "抹茶", 価格: 98000, 在庫: 0 }],
  注文明細: [{ ID: "L1", 商品ID: "P1", 売上金額: 2400, 状態: "確定", 確定日時: new Date(base - 86400000).toISOString() }],
}));
execFileSync(process.execPath, ["--no-warnings", "tools/seed.ts", "seed.json"], { cwd: dir, env });
const p = spawn(process.execPath, ["--no-warnings", "src/main.ts"], { cwd: dir, env, stdio: "ignore" });
try {
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; i < 100; i++) { try { if ((await fetch(url + "/api/health")).ok) break; } catch {} await new Promise((r) => setTimeout(r, 50)); }
  const tok = execFileSync(process.execPath, ["--no-warnings", "tools/dev-token.ts", "--sub", "a", "--caps", "注文明細.read"], { cwd: dir, env }).toString().trim();
  writeFileSync(join(work, "products.json"), await (await fetch(url + "/api/data/" + encodeURIComponent("商品"))).text());
  writeFileSync(join(work, "ranking.json"), await (await fetch(url + "/api/queries/" + encodeURIComponent("売れ筋商品") + "?base=2026-11-02T15:00:00Z", { headers: { Authorization: "Bearer " + tok } })).text());
} finally {
  p.kill();
}
