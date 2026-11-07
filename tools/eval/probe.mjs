// 評価課題(docs/evaluation/persona/task.md)の採点スクリプト。A(KOTONOHA)と B(普段の技術)を同じ基準で採点する。
// 使い方: node tools/eval/probe.mjs --api http://127.0.0.1:8787 --web http://127.0.0.1:3000 --secret <KOTONOHA_JWT_SECRET>
// 前提: task.md の初期データ(e1/e2/e3)を投入した直後の状態で起動していること。
import { createHmac } from "node:crypto";
import { parseArgs } from "node:util";

const { values: o } = parseArgs({ options: { api: { type: "string" }, web: { type: "string" }, secret: { type: "string" } } });
const API = o.api.replace(/\/$/, ""), WEB = (o.web ?? o.api).replace(/\/$/, ""), SECRET = o.secret;
const enc = encodeURIComponent;
const b64 = (x) => Buffer.from(typeof x === "string" ? x : JSON.stringify(x)).toString("base64url");
const jwt = (sub, caps, secret = SECRET) => {
  const h = b64({ alg: "HS256", typ: "JWT" }), p = b64({ sub, caps, iat: Math.floor(Date.now() / 1000), exp: Math.floor(Date.now() / 1000) + 600 });
  return `${h}.${p}.${createHmac("sha256", secret).update(`${h}.${p}`).digest("base64url")}`;
};
const USER = ["予約.create", "イベント.read", "イベント.update"];
const ORG = ["イベント.create"];
async function req(method, path, { token, body, raw } = {}) {
  const headers = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  try {
    const r = await fetch(API + path, { method, headers, body: raw ?? (body === undefined ? undefined : JSON.stringify(body)), signal: AbortSignal.timeout(10000) });
    let j = null;
    try { j = await r.json(); } catch {}
    return { status: r.status, body: j };
  } catch (e) {
    return { status: 0, body: String(e) };
  }
}
const ev = (id) => `/api/data/${enc("イベント")}/${enc(id)}`;
const reserve = (token, イベントID, 人数) => req("POST", `/api/actions/${enc("予約する")}`, { token, body: { イベントID, 人数 } });
const register = (token, x) => req("POST", `/api/actions/${enc("イベント登録")}`, { token, body: x });

const results = [];
async function check(id, title, f) {
  let ok = false, note = "";
  try {
    const r = await f();
    ok = r === true || (typeof r === "object" && r?.ok === true);
    note = typeof r === "object" && r?.note ? r.note : "";
  } catch (e) {
    note = String(e?.message ?? e).slice(0, 200);
  }
  results.push({ id, title, ok, note });
}

await check("F01", "イベント一覧は誰でも見られる", async () => {
  const r = await req("GET", `/api/data/${enc("イベント")}`);
  const e1 = Array.isArray(r.body) && r.body.find((x) => x.ID === "e1");
  return { ok: r.status === 200 && e1 && e1.名前 && e1.定員 === 3 && e1.参加費 === 1000, note: `status ${r.status}` };
});
await check("F02", "詳細は 200、無いものは 404", async () => {
  const a = await req("GET", ev("e2")), b = await req("GET", ev("nope-404"));
  return { ok: a.status === 200 && a.body?.会場 === "大阪" && b.status === 404, note: `${a.status}/${b.status}` };
});
await check("F03", "予約: トークンなしは 401", async () => (await reserve(null, "e2", 1)).status === 401);
await check("F04", "予約: 偽造トークン(別の秘密)は 401", async () => (await reserve(jwt("x", USER, "wrong-secret-0123456789abcdef0123456789"), "e2", 1)).status === 401);
await check("F05", "予約: 権限不足は 403", async () => (await reserve(jwt("x", ["イベント.read"]), "e2", 1)).status === 403);
await check("F06", "予約できて予約数が増える", async () => {
  const before = (await req("GET", ev("e2"))).body?.予約数;
  const r = await reserve(jwt("alice", USER), "e2", 2);
  const after = (await req("GET", ev("e2"))).body?.予約数;
  return { ok: r.status === 200 && typeof r.body?.ok === "string" && after === before + 2, note: `${r.status} ${before}->${after}` };
});
await check("F07", "人数 0 は 422「人数不正」", async () => {
  const r = await reserve(jwt("alice", USER), "e2", 0);
  return { ok: r.status === 422 && r.body?.value === "人数不正", note: `${r.status} ${JSON.stringify(r.body)}` };
});
await check("F08", "入力の形が不正(人数が文字列)は 400", async () => (await reserve(jwt("alice", USER), "e2", "abc")).status === 400);
await check("F09", "残席不足は 422「満席」", async () => {
  const r = await reserve(jwt("bob", USER), "e3", 5);
  return { ok: r.status === 422 && r.body?.value === "満席", note: `${r.status} ${JSON.stringify(r.body)}` };
});
await check("F10", "同時予約でも定員を超えない(定員 1 に 8 人同時)", async () => {
  const c = await register(jwt("org", ORG), { 名前: "残り1席", 会場: "福岡", 開始日時: "2026-12-25T10:00:00+09:00", 参加費: 0, 定員: 1 });
  const id = c.body?.ok;
  if (!id) return { ok: false, note: `登録できない ${c.status}` };
  const rs = await Promise.all([...Array(8)].map((_, i) => reserve(jwt("race" + i, USER), id, 1)));
  const n = rs.filter((r) => r.status === 200).length;
  const after = (await req("GET", ev(id))).body?.予約数;
  return { ok: n === 1 && after === 1, note: `成功 ${n} 件・予約数 ${after}` };
});
await check("F11", "自分の予約には本人の分だけ", async () => {
  await reserve(jwt("carol", USER), "e2", 1);
  const a = await req("GET", `/api/data/${enc("予約")}`, { token: jwt("carol", USER) });
  const d = await req("GET", `/api/data/${enc("予約")}`, { token: jwt("dave", USER) });
  const mine = Array.isArray(a.body) && a.body.length >= 1 && a.body.every((x) => x.参加者 === "carol");
  return { ok: a.status === 200 && mine && Array.isArray(d.body) && d.body.length === 0, note: `${a.status} carol=${a.body?.length} dave=${d.body?.length}` };
});
await check("F12", "自分の予約はログイン必須(401)", async () => (await req("GET", `/api/data/${enc("予約")}`)).status === 401);
await check("F13", "イベント登録: 主催者は 200、権限なしは 403、定員 0 は 422", async () => {
  const x = { 名前: "登録テスト", 会場: "仙台", 開始日時: "2026-12-26T10:00:00+09:00", 参加費: 500, 定員: 5 };
  const a = await register(jwt("org", ORG), x), b = await register(jwt("u", USER), x), c = await register(jwt("org", ORG), { ...x, 定員: 0 });
  return { ok: a.status === 200 && b.status === 403 && c.status === 422, note: `${a.status}/${b.status}/${c.status}` };
});
await check("F14", "会場別の集計が一覧と一致する", async () => {
  const list = (await req("GET", `/api/data/${enc("イベント")}`)).body;
  const r = await req("GET", `/api/analyses/${enc("会場別の予約状況")}`);
  if (!Array.isArray(r.body)) return { ok: false, note: `status ${r.status}` };
  const want = {};
  for (const e of list) {
    want[e.会場] ??= { イベント数: 0, 予約数: 0, 定員: 0 };
    want[e.会場].イベント数 += 1;
    want[e.会場].予約数 += e.予約数;
    want[e.会場].定員 += e.定員;
  }
  const ok = Object.entries(want).every(([k, v]) => r.body.some((x) => x.会場 === k && x.イベント数 === v.イベント数 && x.予約数 === v.予約数 && x.定員 === v.定員));
  return { ok: ok && r.body.length === Object.keys(want).length, note: JSON.stringify(r.body).slice(0, 150) };
});

// 画面(HTML)
async function html(path) {
  let url = WEB + path;
  for (let i = 0; i < 5; i++) {
    const r = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(10000) });
    if (r.status >= 300 && r.status < 400 && r.headers.get("location")) {
      url = new URL(r.headers.get("location"), url).toString();
      continue;
    }
    return { status: r.status, headers: r.headers, text: await r.text() };
  }
  throw new Error("redirect loop");
}
await check("U01", "一覧画面に日本の表記(¥1,000・19:00)で表示", async () => {
  const h = await html("/");
  return { ok: h.status === 200 && h.text.includes("¥1,000") && h.text.includes("19:00"), note: `status ${h.status}` };
});
await check("S01", "画面で名前がエスケープされる(XSS が成立しない)", async () => {
  await register(jwt("org", ORG), { 名前: "<script>window.__pwn=1</script><img src=x onerror=alert(1)>", 会場: "XSS", 開始日時: "2026-12-27T10:00:00+09:00", 参加費: 0, 定員: 1 });
  const h = await html("/");
  const raw = h.text.includes("<script>window.__pwn") || h.text.includes("<img src=x onerror");
  return { ok: h.status === 200 && !raw, note: raw ? "生の HTML が出力された" : "" };
});
await check("S02", "画面の応答にセキュリティヘッダ(CSP または nosniff)", async () => {
  const h = await html("/");
  return { ok: !!(h.headers.get("content-security-policy") || h.headers.get("x-content-type-options")), note: [...h.headers.keys()].join(",") };
});
await check("S03", "技術の痕跡(X-Powered-By・Server の製品名)を出さない", async () => {
  const h = await html("/"), a = await fetch(API + "/api/health");
  const bad = [h.headers, a.headers].some((x) => x.get("x-powered-by") || /express|werkzeug|uvicorn|python|node/i.test(x.get("server") ?? ""));
  return { ok: !bad, note: [h.headers.get("x-powered-by"), h.headers.get("server"), a.headers.get("server")].filter(Boolean).join(",") };
});
await check("S04", "他サイトからの POST(CSRF)を画面のサーバーが受け付けない", async () => {
  // 画面サーバーに、別オリジンからフォーム形式で予約を送る(Cookie は送れない状況を想定し、少なくとも 2xx にならないこと)
  const r = await fetch(`${WEB}/api/actions/${enc("予約する")}`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded", Origin: "http://evil.example" },
    body: "イベントID=e2&人数=1",
    redirect: "manual",
  });
  return { ok: r.status >= 400, note: `status ${r.status}` };
});

const score = results.filter((r) => r.ok).length;
console.log(JSON.stringify({ score, total: results.length, results }, null, 2));
