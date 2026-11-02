// E2E 共通: koto で生成 → 起動 → 終了
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";

export const ROOT = resolve(import.meta.dirname, "../..");
export const KOTO = process.env.KOTO_BIN ?? join(ROOT, "target/debug/koto");
export const SECRET = "e2e-secret-0123456789abcdef0123456789abcdef";

export function freePort(): Promise<number> {
  return new Promise((ok) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => {
      const p = (s.address() as { port: number }).port;
      s.close(() => ok(p));
    });
  });
}

export function build(project: string, target: string, out: string): void {
  execFileSync(KOTO, ["-C", join(ROOT, project), "build", "--target", target, "--out", out], { stdio: "pipe" });
}

export function tmp(name: string): string {
  return mkdtempSync(join(tmpdir(), `kotonoha-${name}-`));
}

export async function waitHttp(url: string, ms = 20000): Promise<void> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try {
      const r = await fetch(url);
      if (r.status < 500) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timeout waiting ${url}`);
}

export function start(cwd: string, args: string[], env: Record<string, string>): ChildProcess {
  const p = spawn(process.execPath, ["--no-warnings", ...args], { cwd, env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"] });
  p.stdout?.on("data", (d) => process.env.E2E_VERBOSE && process.stdout.write(d));
  p.stderr?.on("data", (d) => process.stderr.write(d));
  return p;
}

export function seedFile(dir: string, data: unknown): string {
  const p = join(dir, "seed.json");
  writeFileSync(p, JSON.stringify(data));
  return p;
}

export function runNode(cwd: string, args: string[], env: Record<string, string>): string {
  return execFileSync(process.execPath, ["--no-warnings", ...args], { cwd, env: { ...process.env, ...env } }).toString("utf8").trim();
}

/** 基準日時 2026-11-03T00:00:00+09:00 */
export const BASE = "2026-11-02T15:00:00.000Z";
const BASE_MS = Date.parse(BASE);
const DAY = 86400000;
const iso = (ms: number) => new Date(ms).toISOString();

export function shopSeed() {
  const products = [
    { ID: "P01", 名前: "緑茶", 価格: 300, 在庫: 10 },
    { ID: "P02", 名前: "ほうじ茶", 価格: 350, 在庫: 1 },
    { ID: "P03", 名前: "<script>alert(1)</script><img src=x onerror=alert(2)>", 価格: 100, 在庫: 5 },
  ];
  const items = [
    { ID: "L1", 商品ID: "P01", 売上金額: 3000, 状態: "確定", 確定日時: iso(BASE_MS - 2 * DAY) },
    { ID: "L2", 商品ID: "P02", 売上金額: 9000, 状態: "確定", 確定日時: iso(BASE_MS - 30 * DAY) },
    { ID: "L3", 商品ID: "P03", 売上金額: 99999, 状態: "取消", 確定日時: iso(BASE_MS - 1 * DAY) },
    { ID: "L4", 商品ID: "P03", 売上金額: 88888, 状態: "確定", 確定日時: iso(BASE_MS - 31 * DAY) },
    { ID: "L5", 商品ID: "P03", 売上金額: 500, 状態: "確定", 確定日時: iso(BASE_MS - 3 * DAY) },
  ];
  return { 商品: products, 注文明細: items, 注文: [] };
}

export function exists(p: string): boolean {
  return existsSync(p);
}
