// KOTONOHA Studio の画面(ADR-0135)。表示はすべて textContent で作る(ソースの名前を HTML として扱わない)
const TOKEN = document.querySelector('meta[name="studio-token"]').content;
let S = null; // サーバーから受け取った状態
let sel = { kind: "home" }; // 選んでいるもの
const dirtySource = new Set();

// ---- 小さな道具 ----
function el(tag, attrs = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (k === "text") e.textContent = v;
    else e.setAttribute(k, v === true ? "" : String(v));
  }
  for (const k of kids.flat()) if (k !== null && k !== undefined && k !== false) e.append(k instanceof Node ? k : document.createTextNode(String(k)));
  return e;
}
/** 検査の文から英語の記号を外す(「(UNRESOLVED_INTENT)」「P-LIST」が出て怖かった。GUI で直す評価) */
function soft(t) {
  return String(t ?? "")
    .replace(/\s*\(UNRESOLVED_INTENT\)/g, "")
    .replace(/画面の文型は docs\/spec\/intent_patterns\.md の P-LIST .*?です/, "画面の文は「…を一覧表示」「…の…を表示」「「ボタンの文字」を押すと 動作(…)」などの形です(はじめにの表に例があります)");
}
const $ = (id) => document.getElementById(id);
function store(k, v) {
  try {
    if (v === undefined) return JSON.parse(localStorage.getItem("studio." + k) ?? "null");
    localStorage.setItem("studio." + k, JSON.stringify(v));
  } catch {
    return null;
  }
}
async function load() {
  S = await (await fetch("/api/state")).json();
  if (S.home) return renderWelcome();
  renderAll();
}
// ---- 新しいアプリを作る(プロジェクトが無いフォルダで開いたとき) ----
function renderWelcome() {
  $("welcome").hidden = false;
  for (const sel of [".ask", "main", ".tabs", ".status"]) document.querySelector(sel).hidden = true;
  const box = $("welcome-templates");
  box.replaceChildren(el("legend", { text: "ひな形" }));
  const opts = [{ name: "", desc: "白紙(見本のデータ「商品」1 つと一覧の画面だけ。「「商品」を「本」に変える」のように書くと、自分の言葉になります)" }, ...(S.templates ?? [])];
  opts.forEach((t, i) =>
    box.append(
      el("label", { class: "tpl" }, el("input", { type: "radio", name: "tpl", value: t.name, ...(i === 1 ? { checked: "" } : {}) }), el("b", { text: t.name || "白紙" }), el("span", { class: "help", text: t.desc })),
    ),
  );
  // フォルダ名が「A」「C」のような短い記号なら、アプリの名前には入れない(「Cサロン予約」になった。第 8 回評価)
  if (!$("welcome-name").value && !/^[A-Za-z0-9_.-]{1,3}$/.test(S.folder ?? "")) $("welcome-name").value = S.folder ?? "";
}
$("welcome-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const tpl = document.querySelector('input[name="tpl"]:checked')?.value ?? "";
  $("welcome-msg").textContent = "作っています…";
  const j = await (await fetch("/api/create", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": TOKEN }, body: JSON.stringify({ name: $("welcome-name").value, template: tpl }) })).json();
  if (j.error) {
    $("welcome-msg").textContent = j.error;
    $("welcome-msg").className = "msg err";
    return;
  }
  $("welcome").hidden = true;
  for (const sel of [".ask", "main", ".tabs", ".status"]) document.querySelector(sel).hidden = false;
  S = j;
  renderAll();
});
let autoScreenTried = false;
let notice = null; // 編集の欄に出す直前の結果({kind, text})
async function post(path, body, beforeRender) {
  const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": TOKEN }, body: JSON.stringify(body) });
  const j = await r.json();
  if (j.outline || j.home) S = j;
  // beforeRender が false を返したら編集の欄を描き直さない(読めなかった文と理由を欄に残す)
  const keep = beforeRender?.(j) === false;
  if (j.error || keep) {
    // 失敗したときは編集の欄を描き直さない(書いた内容を残す)。ほかの欄だけ新しくする
    renderSide();
  } else {
    renderAll();
    reloadSoon();
  }
  return j;
}
async function op(o, msgBox) {
  // 前の指示の結果(✓ や誤り)を残さない(別の操作の後も出続けた。第 8 回評価)
  $("ask-result")?.replaceChildren();
  // 知らせは描き直す前に入れる(描き直した後に入れていたので、役割を保存しても何も出なかった。第 10 回評価)
  const okText = (j) => "✓ 入れました(ソースに書きました)" + (j.at?.note ? `。${j.at.note}` : "");
  const j = await post("/api/op", o, (j) => {
    if (!j.error) notice = { kind: "ok", text: okText(j) };
  });
  const text = j.error ? j.error : okText(j);
  if (msgBox && msgBox.isConnected) {
    msgBox.textContent = text;
    msgBox.className = "msg " + (j.error ? "err" : "ok");
  }
  return j;
}
const decls = (kind) => (S?.outline?.decls ?? []).filter((d) => d.kind === kind);
const srcLines = (file) => (S?.sources?.[file] ?? "").split("\n");
const diagsAt = (file, line) => (S?.outline?.diagnostics ?? []).filter((d) => d.file === file && d.span?.line === line);
const typeChoices = () => [...(S?.outline?.types ?? []), ...Object.keys(S?.outline?.enums ?? {})];

// 上の「検査に合格」の表示。確かめるの結果はあとから届くので、届いたときにも描き直す
// (役割を作って確かめるが ✗ になっても「✓ 検査に合格」のままだった。第 14 回評価)
function renderStatus() {
  const errs = (S.outline?.diagnostics ?? []).filter((d) => d.severity === "error").length;
  const st = $("status");
  // 確かめるが ✗ のときは、検査に合格でもそう出す(「✓ 検査に合格」のままで迷った。第 7 回評価)
  const ng = errs === 0 && S.checks?.ok === false ? (S.checks.lines ?? []).filter((l) => l.trim().startsWith("✗")).length : 0;
  st.textContent = errs > 0 ? `✗ 問題 ${errs} 件` : ng > 0 ? `✗ 確かめる ${ng} 件が期待と違う(検査は合格)` : "✓ 検査に合格";
  st.className = "pill " + (errs > 0 ? "err" : ng > 0 ? "warn" : "ok");
}

// ---- 全体 ----
function renderAll() {
  if (!S) return;
  $("app-name").textContent = S.outline?.app ?? "";
  renderStatus();
  $("undo").disabled = !S.canUndo;
  $("ask-target").textContent = sel.kind === "screen" ? `(入れ先: 画面「${sel.name}」)` : sel.kind === "scenario" ? `(入れ先: 確かめる「${sel.name}」)` : "";
  renderTree();
  renderEditor();
  renderSide();
  renderLook();
  renderSources();
  renderRun();
}

// ---- 動かす・初期データ・公開 ----
let seedSel = store("seed") ?? "";
let seedRows = null; // 編集中の表(保存するまで)
/** 動いているか・アプリを開くのリンク(3 秒ごとの読み直しでも更新する。「動いています」なのに押せなかった。第 7 回評価) */
function renderRunStatus() {
  if (!S || S.home) return;
  $("run-state").textContent = S.devError
    ? `アプリを起動できません: ${S.devError}`
    : S.running
      ? "動いています。「アプリを開く」でブラウザの別のタブに出ます(ログインは「名前を入れてすぐ入る」)"
      : S.previewNote || "止まっています";
  $("run-state").className = S.devError ? "msg err" : "help";
  const a = $("run-open");
  if (S.running && S.previewPort && !S.devError) a.href = `http://127.0.0.1:${S.previewPort}/`;
  else a.removeAttribute("href");
  $("run-stop").disabled = !S.running;
  $("run-start").disabled = !!S.running;
}
function renderRun() {
  if (!S || S.home) return;
  renderRunStatus();
  const ds = S.seed ?? [];
  const sel = $("seed-data");
  if (!ds.some((d) => d.data === seedSel)) seedSel = ds[0]?.data ?? "";
  sel.replaceChildren(...ds.map((d) => el("option", { value: d.data, ...(d.data === seedSel ? { selected: "" } : {}) }, d.data)));
  const D = ds.find((d) => d.data === seedSel);
  if (!D) return $("seed-table").replaceChildren(el("p", { class: "help", text: "データがありません" }));
  // 手で直していない間は、いつも今の初期データ(名前の変更・項目の追加の後)を出す(古い項目名のまま保存して断られた。第 8 回評価)
  if (!seedRows || seedRows.data !== D.data || !seedRows.dirty) seedRows = { data: D.data, rows: D.rows.map((r) => ({ ...r })), dirty: false };
  const cols = D.fields.map((f) => f.name);
  seedRows.cols = cols;
  const table = el(
    "table",
    {},
    el("thead", {}, el("tr", {}, ...cols.map((c) => el("th", { text: c })), el("th", { text: "" }))),
    el(
      "tbody",
      {},
      ...seedRows.rows.map((r, i) =>
        el(
          "tr",
          {},
          ...cols.map((c) =>
            el("td", {}, el("input", { value: seedShow(r[c]), "aria-label": `${i + 1} 行目の${c}`, oninput: (e) => { seedRows.rows[i][c] = e.target.value; seedRows.dirty = true; } })),
          ),
          el("td", {}, el("button", { type: "button", class: "small", onclick: () => { seedRows.rows.splice(i, 1); seedRows.dirty = true; renderRun(); } }, "この行を消す")),
        ),
      ),
    ),
  );
  $("seed-table").replaceChildren(table);
  $("publish-check").textContent = S.published || "";
}
$("seed-data").addEventListener("change", (e) => {
  seedSel = e.target.value;
  store("seed", seedSel);
  seedRows = null;
  renderRun();
});
$("seed-add").addEventListener("click", () => {
  if (seedRows) {
    seedRows.rows.push({});
    seedRows.dirty = true;
  }
  renderRun();
});
/** 初期データの表に出す値。日時は「2026-12-05T19:00:00+09:00」ではなく「2026-12-05 19:00」で出す(ISO の形を写して入れていた。第 10 回評価) */
function seedShow(v) {
  if (v === undefined || v === null) return "";
  if (v === true) return "はい";
  if (v === false) return "いいえ";
  const m = /^(\d{4}-\d\d-\d\d)T(\d\d:\d\d)(?::00(?:\.0+)?)?\+09:00$/.exec(String(v));
  return m ? `${m[1]} ${m[2]}` : String(v);
}
async function saveSeed() {
  // 表が空のまま保存すると、前の初期データが全部消える。消してよいか確かめる(0 行で保存された。第 9 回評価)
  const had = (S?.seed ?? []).find((d) => d.data === seedRows?.data)?.rows?.length ?? 0;
  const nonEmpty = (seedRows?.rows ?? []).filter((r) => Object.values(r).some((v) => String(v ?? "").trim() !== "")).length;
  if (had > 0 && nonEmpty === 0 && !confirm(`「${seedRows.data}」の初期データ ${had} 行を全部消して保存しますか?`)) return;
  // 表に出ている項目だけ送る(消した・名前を変えた項目の古い値は送らない)
  const cols = seedRows.cols ?? [];
  const rows = seedRows.rows.map((r) => Object.fromEntries(Object.entries(r).filter(([k]) => !cols.length || cols.includes(k))));
  const j = await post("/api/seed", { data: seedRows.data, rows });
  const n = rows.filter((r) => Object.values(r).some((v) => String(v ?? "").trim() !== "")).length;
  $("seed-msg").textContent = j.error ? j.error : `✓ 保存しました(${n} 行)。動いているアプリに入れ直します`;
  $("seed-msg").className = "msg " + (j.error ? "err" : "ok");
  if (!j.error) seedRows = null;
  renderRun();
}
$("seed-save").addEventListener("click", saveSeed);
// seed.json のような JSON({"データ名": [{…}, …]} か [{…}, …])をそのまま貼っても入れる
// (表の形に書き直していた。第 10〜13 回評価で 6 人以上)
async function pasteJson(pasted) {
  let j;
  try {
    j = JSON.parse(pasted);
  } catch (e) {
    return { error: `JSON として読めませんでした(${e.message})。Excel の表なら、1 行目に項目名を入れて貼り付けます` };
  }
  const sets = Array.isArray(j) ? [[seedRows?.data, j]] : Object.entries(j ?? {});
  const known = (S?.seed ?? []).map((d) => d.data);
  const bad = sets.filter(([k, v]) => !known.includes(k) || !Array.isArray(v)).map(([k]) => k);
  if (bad.length) return { error: `データ「${bad.join("」「")}」はありません(データ: ${known.join("・")})。先に名前をそろえてから貼り付けます` };
  let n = 0;
  for (const [k, rows] of sets) {
    const r = await post("/api/seed", { data: k, rows });
    if (r.error) return { error: `「${k}」: ${r.error}` };
    n += rows.length;
  }
  return { ok: `✓ JSON から ${n} 行入れました(${sets.map(([k, v]) => `${k} ${v.length} 行`).join("・")})。動いているアプリに入れ直します(貼り付けは保存まで済んでいます。「初期データを保存する」を押さなくて大丈夫です)` };
}
$("seed-paste-apply").addEventListener("click", async () => {
  const pasted = $("seed-paste").value;
  if (/^\s*[[{]/.test(pasted)) {
    const r = await pasteJson(pasted.trim());
    $("seed-msg").textContent = r.error ?? r.ok;
    $("seed-msg").className = "msg " + (r.error ? "err" : "ok");
    if (!r.error) {
      $("seed-paste").value = "";
      seedRows = null;
      renderRun();
    }
    return;
  }
  const lines = pasted.split(/\r?\n/).filter((l) => l.trim());
  if (lines.length < 2) {
    $("seed-msg").textContent = "1 行目に項目名、2 行目から値を貼り付けてください";
    $("seed-msg").className = "msg err";
    return;
  }
  // Excel からの貼り付けはタブ区切り。タブが無いときだけカンマで区切る(「1,200」で列がずれた。第 7 回評価)
  const sep = lines[0].includes("\t") ? "\t" : ",";
  const head = lines[0].split(sep).map((x) => x.trim());
  const bad = lines.slice(1).findIndex((l) => l.split(sep).length !== head.length);
  if (bad >= 0) {
    $("seed-msg").textContent = `${bad + 2} 行目の列の数(${lines[bad + 1].split(sep).length})が、1 行目の項目の数(${head.length})と合いません。Excel から表をそのままコピーして貼り付けてください`;
    $("seed-msg").className = "msg err";
    return;
  }
  // 1 行目の項目名が、選んでいるデータの項目と合わないときは入れない(別のデータの表を貼って、行が空になった。第 9 回評価)
  const cols = seedRows.cols ?? [];
  const unknown = head.filter((h) => h && !cols.includes(h));
  if (cols.length && unknown.length) {
    const other = (S?.seed ?? []).find((d) => d.data !== seedRows.data && unknown.every((h) => d.fields.some((f) => f.name === h)));
    $("seed-msg").textContent =
      `「${seedRows.data}」には項目「${unknown.join("」「")}」がありません(項目: ${cols.join("・")})。` +
      (other ? `「${other.data}」の表なら、上でデータを「${other.data}」に切り替えてから貼り付けます` : "1 行目には項目名を書きます");
    $("seed-msg").className = "msg err";
    return;
  }
  const before = seedRows.rows;
  seedRows.rows = lines.slice(1).map((l) => Object.fromEntries(l.split(sep).map((v, i) => [head[i], v.trim()]).filter(([k]) => k)));
  seedRows.dirty = true;
  $("seed-paste").value = "";
  await saveSeed();
  // 貼り付けはそのまま保存まで済む(「初期データを保存する」も押すのか迷った。第 13 回評価)
  if (!$("seed-msg").className.includes("err")) $("seed-msg").textContent += "(貼り付けは保存まで済んでいます。「初期データを保存する」を押さなくて大丈夫です)";
  // 入れられなかったときは、貼り付けた表と元の行を戻す
  if ($("seed-msg").className.includes("err")) {
    if (!$("seed-paste").value) $("seed-paste").value = pasted;
    if (seedRows) seedRows.rows = before;
    renderRun();
  }
});
$("run-stop").addEventListener("click", () => post("/api/app", { action: "stop" }));
$("run-start").addEventListener("click", () => post("/api/app", { action: "start" }));
$("publish").addEventListener("click", async () => {
  $("publish-msg").textContent = "作っています…(1 分ほどかかります)";
  const j = await post("/api/publish", {});
  $("publish-msg").textContent = j.error ? j.error : `✓ ${j.done}`;
  $("publish-msg").className = "msg " + (j.error ? "err" : "ok");
});

function renderTree() {
  const t = $("tree");
  t.replaceChildren();
  const item = (label, s, isNew) =>
    el("button", { type: "button", class: isNew ? "new" : "", "aria-current": !isNew && sel.kind === s.kind && sel.name === s.name ? "true" : null, onclick: () => select(s) }, label);
  t.append(item("はじめに", { kind: "home" }));
  const group = (title, kind, newLabel, newKind) => {
    t.append(el("h3", { text: title }));
    for (const d of decls(kind)) t.append(item(d.name, { kind, name: d.name }));
    if (newLabel) t.append(item("+ " + newLabel, { kind: newKind }, true));
  };
  group("データ", "data", "新しいデータ", "new-data");
  group("画面", "screen", "新しい画面", "new-screen");
  group("動作(ボタンで行う処理)", "action", "新しい動作", "new-action");
  group("役割(だれが何をできるか)", "role", "新しい役割", "new-role");
  group("集計(決まった名前の表)", "analysis", "新しい集計", "new-analysis");
  t.append(el("h3", { text: "約束(見てよい人・重複・順番)" }));
  t.append(item(decls("rules").length ? "約束" : "+ 約束を書く", { kind: "rules", name: "約束" }, !decls("rules").length));
  group("確かめる(動きの確認)", "scenario", "新しい確かめる", "new-scenario");
}
function select(s) {
  sel = s;
  renderAll();
  $("editor").focus?.();
}

/** 画面の行の記号まじりの文(「「1名で予約」を押すと 予約する(イベント.ID, 1)」)を、読める言い方にする。読めなければ null
 *  (行の中の記号が怖いと 10 人中 9 人。GUI で直す評価) */
function friendlyLine(t) {
  const arg = (a) => {
    a = a.trim();
    let m;
    if ((m = /^(\S+?)\.ID$/.exec(a))) return `この${m[1]}`;
    if (a === "本人") return "ログインしている人";
    if ((m = /^"(.*)"$/.exec(a))) return `「${m[1]}」`;
    return a;
  };
  let m = /^(?:(.+?)とき)?「(.+?)」を押すと\s+(\S+?)\((.*)\)$/.exec(t.trim());
  if (m) {
    const act = (S?.outline?.decls ?? []).find((x) => x.kind === "action" && x.name === m[3]);
    const args = m[4]
      .split(",")
      .map((x) => x.trim())
      .filter(Boolean)
      .map((x, i) => {
        // 何の値かを動作の受け取る値の名前で添える(「このイベント・1」では 1 が何か分からない)
        const pn = act?.params?.[i]?.name;
        return /\.ID$/.test(x) || !pn ? arg(x) : `${pn} ${arg(x)}`;
      });
    const when = m[1] ? `${m[1].replace(/\S+?\.(\S+?)が/g, "$1が")}ときだけ出る、` : "";
    return `${when}「${m[2]}」のボタン → 押すと「${m[3]}」${args.length ? `(${args.join("・")})` : ""}`;
  }
  if ((m = /^(\S+?)のフォームを表示$/.exec(t.trim()))) return `「${m[1]}」の入力欄(フォーム)`;
  return null;
}
// ---- 編集の欄 ----
function lineList(d, opts = {}) {
  const ls = srcLines(d.file);
  const box = el("div", { class: "rows" });
  for (let n = d.line + 1; n <= d.end; n++) {
    const text = ls[n - 1] ?? "";
    if (!text.trim() || text.trim().startsWith("#")) continue;
    const bad = diagsAt(d.file, n).filter((x) => x.severity === "error");
    // その場で直す(足して消すしかなく、同じ表示が 2 回出た。第 8 回評価)
    const edit = () => {
      const input = el("input", { type: "text", value: text.trim(), "aria-label": "直した文" });
      const msg = el("div", { class: "msg", role: "status", text: friendlyLine(text) ? "ボタンの文字は「」の中を書き換えます。( ) の中はそのまま残します" : "" });
      row.replaceChildren(
        el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); const v = input.value; input.blur(); if (v.trim() && v.trim() !== text.trim()) { const j = await op({ op: "replace_line", file: d.file, line: n, old: text, text: v }, msg); if (j.error) { input.value = v; input.focus(); } } else renderAll(); } }, input, el("button", { type: "submit" }, "直す"), el("button", { type: "button", class: "ghost", onclick: () => renderAll() }, "やめる")),
        msg,
      );
      input.focus();
    };
    const row = el(
        "div",
        { class: "line" + (bad.length ? " bad" : "") },
        friendlyLine(text)
          ? el("div", { class: "plain" }, el("span", { text: friendlyLine(text) }), el("code", { class: "raw", title: "ソースでの書き方" }, text.trim()), ...bad.map((b) => el("span", { class: "why", text: soft(b.message_ja) + (b.hints?.length ? "(" + soft(b.hints[0]) + ")" : "") })))
          : el("code", {}, text.trim(), ...bad.map((b) => el("span", { class: "why", text: soft(b.message_ja) + (b.hints?.length ? "(" + soft(b.hints[0]) + ")" : "") }))),
        opts.removable === false ? null : el("button", { type: "button", class: "small ghost", "aria-label": "この行を直す", title: "この行を書き直す", onclick: edit }, "直す"),
        opts.removable === false ? null : el("button", { type: "button", class: "small ghost", "aria-label": "この行を上へ", title: "上へ", onclick: () => op({ op: "move_line", file: d.file, line: n, old: text, dir: "up" }) }, "↑"),
        opts.removable === false ? null : el("button", { type: "button", class: "small ghost", "aria-label": "この行を下へ", title: "下へ", onclick: () => op({ op: "move_line", file: d.file, line: n, old: text, dir: "down" }) }, "↓"),
        opts.removable === false ? null : el("button", { type: "button", class: "small", "aria-label": "この行を消す", onclick: () => op({ op: "remove_line", file: d.file, line: n, old: text }) }, "消す"),
      );
    box.append(row);
  }
  return box;
}
function addLineForm(kind, name, placeholder, list) {
  const id = "dl-" + Math.random().toString(36).slice(2);
  const input = el("input", { type: "text", placeholder, list: list ? id : null, "aria-label": "足す文" });
  const msg = el("div", { class: "msg", role: "status" });
  const form = el(
    "form",
    {
      class: "row",
      onsubmit: async (e) => {
        e.preventDefault();
        if (!input.value.trim()) return;
        // 画面の足す欄は、上の「日本語で指示する」と同じに読む(例の「「2名で予約」ボタンを足す」がそのまま通らず、
        // 読めない行が入って ✗ になった。第 11 回評価)。読めない文は入れずに理由と近い書き方を出す
        if (kind === "screen") {
          $("ask-result")?.replaceChildren();
          // 送る前に欄を空にする(書きかけの欄があると描き直さないので、足した行が画面に出なかった)。読めなかったら戻す
          const text = input.value;
          input.value = "";
          const j = await post("/api/instruct", { text, screen: name }, (j) => {
            if (j.result?.ok) notice = { kind: "ok", text: "✓ " + j.result.done };
            else return false;
          });
          const r = j.result;
          if (r?.ok) return;
          input.value = text;
          if (msg.isConnected) {
            const why = j.error || r?.errors?.[0]?.message || r?.message || "この文は読めませんでした";
            msg.className = "msg err";
            msg.replaceChildren(
              el("div", { text: "この文は読めませんでした: " + why }),
              ...(r?.suggestions?.length ? [el("div", { class: "help", text: "近い書き方(押すと入ります):" }), el("div", { class: "chips" }, ...r.suggestions.map((sg) => el("button", { type: "button", class: "chip", onclick: () => { input.value = sg; input.focus(); } }, sg)))] : []),
            );
          }
          return;
        }
        const text = input.value;
        input.value = "";
        const j = await op(kind === "rules" ? { op: "add_rule", text } : { op: "add_line", kind, name, text }, msg);
        if (j.error) input.value = text;
      },
    },
    input,
    el("button", { type: "submit" }, "足す"),
  );
  return el("div", {}, form, list ? el("datalist", { id }, ...list.map((p) => el("option", { value: p }))) : null, msg);
}
function typeSelect(value) {
  const s = el("select", { "aria-label": "型" }, ...typeChoices().map((t) => el("option", { value: t, selected: t === value }, t)));
  return s;
}
function fieldRows(box, rows) {
  const add = (n = "", t = "文字列") => {
    const name = el("input", { type: "text", placeholder: "項目の名前(例: 電話番号)", value: n, "aria-label": "項目の名前" });
    const type = typeSelect(t);
    const opt = el("input", { type: "checkbox", "aria-label": "空でもよい" });
    const r = el("div", { class: "row" }, name, type, el("label", {}, opt, " 空でもよい"), el("button", { type: "button", class: "small", onclick: () => r.remove() }, "消す"));
    r.read = () => ({ name: name.value, type: opt.checked ? `選択<${type.value}>` : type.value });
    box.append(r);
  };
  rows.forEach(([n, t]) => add(n, t));
  return add;
}

/** 集計(分析)を作るフォーム。記号を書かずに、データ・まとめる項目・合計する項目・列の名前を選ぶ(第 7 回評価: 30 人中 29 人が例を写した) */
function analysisForm() {
  const msg = el("div", { class: "msg", role: "status" });
  const name = el("input", { type: "text", placeholder: "例: 会場別の予約状況", "aria-label": "集計の名前" });
  const datas = decls("data");
  const data = el("select", { "aria-label": "集計するデータ" }, ...datas.map((d) => el("option", { value: d.name }, d.name)));
  const unit = el("select", { "aria-label": "まとめる項目" });
  const sums = el("div", { class: "rows" });
  const cols = el("div", { class: "rows" });
  const colNames = () => {
    const D = datas.find((d) => d.name === data.value);
    return [unit.value, `${D?.name ?? ""}数`, ...order];
  };
  // チェックを付けた順(行は動かさない。付けると行が上に動き、押し間違えた。第 16 回評価)
  let order = [];
  const renderCols = () => {
    // 書き換えた列の名前は、同じ列が残っている間は保つ(選び直した列は元の名前にする)
    const kept = new Map([...cols.querySelectorAll("input")].filter((x) => x.dataset.edited).map((x) => [x.dataset.base, x.value]));
    cols.replaceChildren(
      ...colNames().map((c, i) => {
        const input = el("input", { type: "text", value: kept.get(c) ?? c, "aria-label": `${i + 1} 列目の名前`, "data-base": c, oninput: () => (input.dataset.edited = "1") });
        if (kept.has(c)) input.dataset.edited = "1";
        return el("label", { class: "row" }, el("span", { text: `${i + 1} 列目` }), input);
      }),
    );
  };
  const fill = () => {
    const D = datas.find((d) => d.name === data.value);
    const fs = (D?.fields ?? []).filter((f) => f.name !== "ID");
    unit.replaceChildren(...fs.filter((f) => !/^(整数|金額)/.test(f.type)).map((f) => el("option", { value: f.name }, f.name)));
    // 合計の列の順番も選べる(列の順が課題と違った。第 8 回評価)
    order = [];
    // 列の番号を行の横に出す(どの順で列になるかが見える)
    const marks = new Map();
    const refresh = () => {
      for (const [n, m] of marks) m.textContent = order.includes(n) ? `${order.indexOf(n) + 3} 列目` : "";
      renderCols();
    };
    // ↑↓ は列の順(付けた順)の中で 1 つ前・後ろへ
    const move = (name, dir) => {
      const k = order.indexOf(name);
      const t = k + dir;
      if (k < 0 || t < 0 || t >= order.length) return;
      [order[k], order[t]] = [order[t], order[k]];
      refresh();
    };
    sums.replaceChildren(
      ...fs
        .filter((f) => /^(選択<)?(整数|金額)/.test(f.type))
        .map((f) => {
          const onchange = (e) => {
            order = order.filter((n) => n !== f.name);
            if (e.target.checked) order.push(f.name);
            refresh();
          };
          const mark = el("span", { class: "help" });
          marks.set(f.name, mark);
          return el(
            "div",
            { class: "line" },
            el("label", {}, el("input", { type: "checkbox", value: f.name, onchange }), ` ${f.name} `),
            mark,
            el("button", { type: "button", class: "small ghost", "aria-label": `${f.name}の列を左へ`, title: "列を左へ", onclick: () => move(f.name, -1) }, "←"),
            el("button", { type: "button", class: "small ghost", "aria-label": `${f.name}の列を右へ`, title: "列を右へ", onclick: () => move(f.name, 1) }, "→"),
          );
        }),
    );
    renderCols();
  };
  data.addEventListener("change", fill);
  unit.addEventListener("change", renderCols);
  fill();
  const go = async () => {
    const D = datas.find((d) => d.name === data.value);
    const picked = order;
    const names = [...cols.querySelectorAll("input")].map((x) => x.value.trim());
    const sentence = picked.length ? `${D.name}の${picked.join("と")}を${unit.value}ごとに合計` : `${D.name}を${unit.value}ごとに集計`;
    const text = `分析「${name.value.trim()}」を作る(${sentence}、列: ${names.join("・")})`;
    // 失敗したときに選んだ内容が消えないよう、ここでは編集の欄を描き直さない
    const r0 = await fetch("/api/instruct", { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": TOKEN }, body: JSON.stringify({ text }) });
    const j = await r0.json();
    const r = j.result ?? {};
    msg.textContent = j.error ?? (r.ok ? `✓ ${r.done}` : r.errors?.[0]?.message ?? "作れませんでした");
    msg.className = "msg " + (j.error || !r.ok ? "err" : "ok");
    if (j.outline) S = j;
    if (!j.error && r.ok) {
      notice = { kind: "ok", text: `✓ ${r.done}` };
      select({ kind: "analysis", name: name.value.trim() });
      reloadSoon();
    } else renderSide();
  };
  return [
    el("p", { class: "sub", text: "決まった名前の表(API)として、データを項目ごとにまとめた件数と合計を出します。画面に「集計の名前を表示」と書くと画面にも出ます。" }),
    el("label", { class: "row" }, el("span", { text: "集計の名前" }), name),
    el("label", { class: "row" }, el("span", { text: "どのデータを" }), data),
    el("label", { class: "row" }, el("span", { text: "何ごとにまとめる" }), unit),
    el("h3", { text: "合計する項目(選ばなければ件数だけ。チェックを付けた順に列になり、横に何列目かが出ます。← → で列の順を入れ替えられます)" }),
    sums,
    el("h3", { text: "列の名前(左から順に。決まった名前があれば書き換える)" }),
    cols,
    el("div", { class: "row" }, el("button", { type: "button", onclick: go }, "この集計を作る")),
    msg,
  ];
}
const EDITORS = {
  home() {
    const themes = ["藍", "紺", "青", "緑", "深緑", "朱", "紅", "紫", "橙", "茶", "墨"];
    const pick = el("select", { "aria-label": "テーマ色" }, el("option", { value: "" }, S.theme ? `今の色: ${S.theme}` : "今の色: 藍(既定)"), ...themes.filter((c) => c !== (S.theme || "藍")).map((c) => el("option", { value: c }, c)));
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: "画面で作る" }),
      el("p", { class: "sub", text: "左の一覧から選んで、データ・画面・動作・役割・約束を足したり直したりできます。上の「日本語で指示する」に書いても作れます。" }),
      el("ol", {}, el("li", { text: "データ: 覚えておくもの(お客様・予約など)と、その項目" }), el("li", { text: "画面: 一覧・詳細・フォーム・ボタン" }), el("li", { text: "動作: ボタンを押したときの処理(予約する・取り消す など)" }), el("li", { text: "役割と約束: だれが何を見て・何をできるか" }), el("li", { text: "確かめる: 思ったとおりに動くかの確認(保存するたびに自動で試します)" })),
      el("p", { class: "help", text: "変更はすべてソース(src/*.koto)に文として入ります。「見た目の確認」で、iPhone・Android の各機種の大きさでの表示を並べて見られます。" }),
      el("h3", { text: "色" }),
      el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); if (pick.value) await op({ op: "set_theme", color: pick.value }, msg); } }, pick, el("button", { type: "submit" }, "この色にする")),
      msg,
    ];
  },
  analysis() {
    const d = decls("analysis").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    const ls = srcLines(d.file).slice(d.line - 1, d.end);
    return [
      el("h2", { text: `集計「${d.name}」` }),
      el("p", { class: "sub", text: `決まった名前の表です。アプリでは /api/analyses/${d.name} で取り出せます。画面に出すときは、画面を選んで「${d.name}を表示」と書きます。` }),
      el("pre", { class: "code" }, ls.join("\n")),
      el("hr"),
      el("button", { type: "button", class: "danger", onclick: () => confirm(`集計「${d.name}」を消しますか?(出している画面の行も消します)`) && op({ op: "remove_decl", kind: "analysis", name: d.name }).then(() => select({ kind: "home" })) }, "この集計を消す"),
    ];
  },
  "new-analysis"() {
    return [el("h2", { text: "新しい集計" }), ...analysisForm()];
  },
  data() {
    const d = decls("data").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    const ls = srcLines(d.file);
    const rows = el("div", { class: "rows" });
    for (const f of d.fields) {
      const bad = diagsAt(d.file, f.line).filter((x) => x.severity === "error");
      rows.append(
        el(
          "div",
          { class: "line" + (bad.length ? " bad" : "") },
          el("code", {}, `${f.name}: ${f.type}`, ...bad.map((b) => el("span", { class: "why", text: b.message_ja }))),
          // 項目の順番 = 一覧の列・詳細の並び(列の順番を変えられなかった。第 7 回評価)
          f.name === "ID" ? null : el("button", { type: "button", class: "small ghost", "aria-label": `${f.name}を上へ`, title: "上へ(一覧の列が左へ)", onclick: () => op({ op: "move_line", file: d.file, line: f.line, old: ls[f.line - 1], dir: "up" }) }, "↑"),
          f.name === "ID" ? null : el("button", { type: "button", class: "small ghost", "aria-label": `${f.name}を下へ`, title: "下へ(一覧の列が右へ)", onclick: () => op({ op: "move_line", file: d.file, line: f.line, old: ls[f.line - 1], dir: "down" }) }, "↓"),
          f.name === "ID" ? el("span", { class: "help", text: "自動で決まります" }) : el("button", { type: "button", class: "small", onclick: () => op({ op: "remove_line", file: d.file, line: f.line, old: ls[f.line - 1] }) }, "消す"),
        ),
      );
    }
    for (const c of d.computed ?? []) {
      const bad = diagsAt(d.file, c.line).filter((x) => x.severity === "error");
      rows.append(
        el("div", { class: "line" + (bad.length ? " bad" : "") }, el("code", {}, `${c.name} = ${c.expr}`, el("span", { class: "help", text: "(計算項目)" }), ...bad.map((b) => el("span", { class: "why", text: b.message_ja }))), el("button", { type: "button", class: "small", onclick: () => op({ op: "remove_line", file: d.file, line: c.line, old: ls[c.line - 1] }) }, "消す")),
      );
    }
    const box = el("div", { class: "rows" });
    const add = fieldRows(box, [["", "文字列"]]);
    const msg = el("div", { class: "msg", role: "status" });
    const cname = el("input", { type: "text", placeholder: "名前(例: 残席)", "aria-label": "計算項目の名前" });
    const cexpr = el("input", { type: "text", placeholder: "式(例: 定員 - 予約数)", "aria-label": "計算の式" });
    const cmsg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: `データ「${d.name}」` }),
      el("p", { class: "sub", text: "覚えておく項目の一覧です。ID は自動で付きます。計算項目は読むときに計算します(保存しません)。項目の順番が、一覧の列と詳細の並びになります(↑↓ で変えられます)。" }),
      rows,
      el("h3", { text: "計算項目を足す(残席 = 定員 - 予約数 など)" }),
      el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); const j = await op({ op: "add_line", kind: "data", name: d.name, text: `${cname.value.trim()} = ${cexpr.value.trim()}` }, cmsg); if (!j.error) { cname.value = ""; cexpr.value = ""; } } }, cname, el("span", { text: "=" }), cexpr, el("button", { type: "submit" }, "足す")),
      cmsg,
      el("h3", { text: "項目を足す" }),
      box,
      el("div", { class: "row" }, el("button", { type: "button", class: "ghost", onclick: () => add() }, "もう 1 つ"), el("button", { type: "button", onclick: async () => { for (const r of [...box.children]) { const v = r.read(); if (v.name.trim()) await op({ op: "add_field", data: d.name, ...v }, msg); } } }, "足す")),
      msg,
      el("hr"),
      el("button", { type: "button", class: "danger", onclick: () => confirm(`データ「${d.name}」を消しますか?(使っている画面・動作はエラーになります)`) && op({ op: "remove_decl", kind: "data", name: d.name }).then(() => select({ kind: "home" })) }, "このデータを消す"),
    ];
  },
  "new-data"() {
    const name = el("input", { type: "text", placeholder: "データの名前(例: お客様)", "aria-label": "データの名前" });
    const box = el("div", { class: "rows" });
    const add = fieldRows(box, [["名前", "文字列"], ["", "文字列"]]);
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: "新しいデータ" }),
      el("p", { class: "sub", text: "覚えておくもの(お客様・予約・商品など)と、その項目を決めます。" }),
      el("div", { class: "row" }, name),
      el("h3", { text: "項目" }),
      box,
      el("div", { class: "row" }, el("button", { type: "button", class: "ghost", onclick: () => add() }, "項目をもう 1 つ"), el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_data", name: name.value, fields: [...box.children].map((r) => r.read()) }, msg); if (!j.error) select({ kind: "data", name: name.value.trim() }); } }, "このデータを作る")),
      msg,
    ];
  },
  screen() {
    const d = decls("screen").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    return [
      el("h2", { text: `画面「${d.name}」` }),
      el("p", { class: "sub", text: "この画面に出すもの。上の「日本語で指示する」に書いた文も、この画面に入ります。" }),
      lineList(d),
      el("h3", { text: "足す(候補から選ぶか、日本語で書く)" }),
      // 例は今のデータの名前で出す(名前を変えた後も「枠」のままだった。第 10 回評価)
      addLineForm("screen", d.name, `例: ${decls("data")[0]?.name ?? "予約"}を新しい順に一覧表示 / 「2名で予約」ボタンを足す`, S.outline.patterns.screen),
      el("hr"),
      el("button", { type: "button", class: "danger", onclick: () => confirm(`画面「${d.name}」を消しますか?`) && op({ op: "remove_decl", kind: "screen", name: d.name }).then(() => select({ kind: "home" })) }, "この画面を消す"),
    ];
  },
  "new-screen"() {
    const name = el("input", { type: "text", placeholder: "画面の名前(例: 予約一覧)", "aria-label": "画面の名前" });
    const id = "dl-new-screen";
    const first = el("input", { type: "text", placeholder: "最初に出すもの(例: 予約を一覧表示)", list: id, "aria-label": "最初に出すもの" });
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: "新しい画面" }),
      el("div", { class: "rows" }, name, first, el("datalist", { id }, ...S.outline.patterns.screen.map((p) => el("option", { value: p })))),
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_screen", name: name.value, lines: [first.value] }, msg); if (!j.error) select({ kind: "screen", name: name.value.trim() }); } }, "この画面を作る"),
      msg,
    ];
  },
  action() {
    const d = decls("action").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    const data = decls("data").map((x) => x.name);
    const input = el("input", { type: "text", placeholder: "処理を 1 行(例: 件.状況 = \"完了\")", "aria-label": "処理の行" });
    const msg = el("div", { class: "msg", role: "status" });
    const tpl = (label, text) => el("button", { type: "button", class: "small", onclick: () => { input.value = text; input.focus(); } }, label);
    const D = data[0] ?? "データ";
    return [
      el("h2", { text: `動作「${d.name}」` }),
      el("p", { class: "sub", text: "処理は上から順に行い、失敗したら全部元に戻ります。" }),
      // 引数の順番 = フォームの入力欄の順番(フォームの項目の順を変えられなかった。第 8 回評価)
      el("h3", { text: "受け取る値(この順にフォームの入力欄が並びます)" }),
      el(
        "div",
        { class: "rows" },
        ...(d.params.length
          ? d.params.map((p, i) =>
              el(
                "div",
                { class: "line" },
                el("code", {}, `${p.name}: ${p.type}`),
                el("button", { type: "button", class: "small ghost", "aria-label": `${p.name}を上へ`, disabled: i === 0, onclick: () => op({ op: "move_param", action: d.name, index: i, dir: "up" }) }, "↑"),
                el("button", { type: "button", class: "small ghost", "aria-label": `${p.name}を下へ`, disabled: i === d.params.length - 1, onclick: () => op({ op: "move_param", action: d.name, index: i, dir: "down" }) }, "↓"),
              ),
            )
          : [el("p", { class: "help", text: "なし" })]),
      ),
      (() => {
        const pn = el("input", { type: "text", placeholder: "受け取る値の名前(例: 目的)", "aria-label": "受け取る値の名前" });
        const pm = el("div", { class: "msg", role: "status" });
        return el(
          "div",
          {},
          el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); if (!pn.value.trim()) return; const j = await op({ op: "add_param", action: d.name, name: pn.value }, pm); if (!j.error) pn.value = ""; } }, pn, el("button", { type: "submit" }, "受け取る値を足す")),
          el("p", { class: "help", text: "型は名前から決めます(金額・日付など)。ボタンと確かめるの呼び出しにも仮の値が入ります。" }),
          pm,
        );
      })(),
      el("h3", { text: "処理" }),
      lineList(d),
      el("h3", { text: "処理を足す" }),
      el("div", { class: "row" }, tpl("1 件取り出す", `件 = ${D}を取得(${D}ID)`), tpl("条件を確かめる", `件.数 > 0 を確認 または 失敗("不正")`), tpl("書き換える", `件.状況 = "完了"`), tpl("登録する", `${D}を登録(${(decls("data")[0]?.fields ?? []).filter((f) => f.name !== "ID").map((f) => `${f.name}: …`).join(", ")}) を返す`), tpl("ID を返す", `${D}ID を返す`)),
      el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); const j = await op({ op: "add_line", kind: "action", name: d.name, text: input.value }, msg); if (!j.error) input.value = ""; } }, input, el("button", { type: "submit" }, "足す")),
      msg,
      el("hr"),
      el("button", { type: "button", class: "danger", onclick: () => confirm(`動作「${d.name}」を消しますか?`) && op({ op: "remove_decl", kind: "action", name: d.name }).then(() => select({ kind: "home" })) }, "この動作を消す"),
    ];
  },
  "new-action"() {
    const name = el("input", { type: "text", placeholder: "動作の名前(例: 予約を取り消す)", "aria-label": "動作の名前" });
    const pbox = el("div", { class: "rows" });
    const addP = (n = "", t = "識別子") => {
      const pn = el("input", { type: "text", placeholder: "引数の名前(例: 予約ID)", value: n, "aria-label": "引数の名前" });
      const pt = typeSelect(t);
      const r = el("div", { class: "row" }, pn, pt, el("button", { type: "button", class: "small", onclick: () => r.remove() }, "消す"));
      r.read = () => ({ name: pn.value, type: pt.value });
      pbox.append(r);
    };
    addP();
    const body = el("textarea", { rows: 5, placeholder: "処理を 1 行ずつ。例:\n件 = 予約を取得(予約ID)\n件.状態 = \"取消\"\n予約ID を返す", "aria-label": "処理" });
    const fails = Object.keys(S.outline.enums ?? {});
    const fail = el("select", { "aria-label": "失敗の種類" }, el("option", { value: "" }, "(自動で作る)"), ...fails.map((f) => el("option", { value: f }, f)));
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: "新しい動作" }),
      el("p", { class: "sub", text: "ボタンやフォームで行う処理です。データを変える処理は、途中で失敗すると全部元に戻ります。" }),
      el("div", { class: "row" }, name),
      el("h3", { text: "引数(受け取る値)" }),
      pbox,
      el("button", { type: "button", class: "ghost", onclick: () => addP() }, "引数をもう 1 つ"),
      el("h3", { text: "処理" }),
      body,
      el("div", { class: "row" }, el("label", {}, "失敗の種類(列挙) ", fail)),
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_action", name: name.value, params: [...pbox.children].map((r) => r.read()), body: body.value.split("\n"), fail: fail.value }, msg); if (!j.error) select({ kind: "action", name: name.value.trim() }); } }, "この動作を作る"),
      msg,
    ];
  },
  role() {
    return roleEditor(decls("role").find((x) => x.name === sel.name));
  },
  "new-role"() {
    return roleEditor(null);
  },
  rules() {
    const d = decls("rules")[0];
    return [
      el("h2", { text: "約束" }),
      el("p", { class: "sub", text: "だれが見てよいか、二重登録の禁止、状態の順番などを日本語で書きます。" }),
      d ? lineList(d) : el("p", { class: "help", text: "まだありません。" }),
      el("h3", { text: "足す(候補から選ぶか、日本語で書く)" }),
      addLineForm("rules", "約束", "例: 予約は予約者が本人のものだけ閲覧可", S.outline.patterns.rules),
    ];
  },
  scenario() {
    const d = decls("scenario").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    return [
      el("h2", { text: `確かめる「${d.name}」` }),
      el("p", { class: "sub", text: "保存するたびに、新しいデータベースで上から順に試します。結果は右に出ます。" }),
      lineList(d),
      el("h3", { text: "手順を足す" }),
      addLineForm("scenario", d.name, "例: 花子(利用者)が 予約する(s1, 2) → 成功 / 花子から 予約が 1件見える"),
      el("p", { class: "help", text: "結果は 成功 / 失敗(\"値\") / ログインが必要 / 権限がない / 重複 / 順番が違う / 見つからない。→ 成功(名前) で作った物に名前を付けて後で使えます。" }),
    ];
  },
  "new-scenario"() {
    const name = el("input", { type: "text", placeholder: "名前(例: 予約の流れ)", "aria-label": "名前" });
    const lines = el("textarea", { rows: 5, placeholder: "1 行に 1 つ。例:\n花子(利用者)が 予約する(s1, 2) → 成功\n花子から 予約が 1件見える", "aria-label": "手順" });
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: "新しい確かめる" }),
      el("div", { class: "rows" }, name, lines),
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_scenario", name: name.value, lines: lines.value.split("\n") }, msg); if (!j.error) select({ kind: "scenario", name: name.value.trim() }); } }, "この確かめるを作る"),
      msg,
    ];
  },
};
function roleEditor(d) {
  const has = new Set(d?.caps ?? []);
  const data = decls("data").map((x) => x.name);
  const verbs = ["読取", "作成", "更新", "削除"];
  const boxes = [];
  const cb = (cap, label) => {
    const c = el("input", { type: "checkbox", "aria-label": label ?? cap, checked: has.has(cap) });
    boxes.push([cap, c]);
    return c;
  };
  const table = el(
    "table",
    { class: "grid-caps" },
    el("thead", {}, el("tr", {}, el("th", { text: "データ" }), ...verbs.map((v) => el("th", { text: v })), el("th", { text: "全員の分を見る" }))),
    el("tbody", {}, ...data.map((x) => el("tr", {}, el("td", { text: x }), ...verbs.map((v) => el("td", {}, cb(`${x}.${v}`, `${x} の${v}`))), el("td", {}, cb(`全件閲覧.${x}`, `${x} を全員の分見る`))))),
  );
  const acts = decls("action").map((a) => el("label", { class: "row" }, cb(`動作.${a.name}`, `${a.name} をこの役割だけにする`), ` ${a.name}`));
  const name = d ? null : el("input", { type: "text", placeholder: "役割の名前(例: 店長)", "aria-label": "役割の名前" });
  const msg = el("div", { class: "msg", role: "status" });
  const caps = () => boxes.filter(([, c]) => c.checked).map(([k]) => k);
  return [
    el("h2", { text: d ? `役割「${d.name}」` : "新しい役割" }),
    el("p", { class: "sub", text: "この役割の人ができることを選びます。何も選ばなければ何もできません(既定で禁止)。" }),
    name,
    table,
    el("h3", { text: "この役割だけが実行できる動作" }),
    el("p", { class: "help", text: "選ぶと、同じデータを更新できる別の役割でも実行できなくなります(例: 店長だけの締切)。" }),
    el("div", { class: "rows" }, ...acts),
    el("button", { type: "button", onclick: async () => { const j = await op(d ? { op: "set_role_caps", name: d.name, caps: caps() } : { op: "add_role", name: name.value, caps: caps() }, msg); if (!j.error && !d) select({ kind: "role", name: name.value.trim() }); } }, d ? "役割を保存する" : "この役割を作る"),
    msg,
    d ? el("hr") : null,
    d ? el("button", { type: "button", class: "danger", onclick: () => confirm(`役割「${d.name}」を消しますか?`) && op({ op: "remove_decl", kind: "role", name: d.name }).then(() => select({ kind: "home" })) }, "この役割を消す") : null,
  ];
}
function renderEditor() {
  const ed = $("editor");
  const active = document.activeElement;
  // 入力中は描き直さない(書いている途中の文を消さない)
  // (選ぶ欄・チェックで選んでいる途中も同じ。集計のフォームで選んだ内容が消えた)
  if (ed.contains(active) && (((active.tagName === "INPUT" || active.tagName === "TEXTAREA") && active.value) || active.tagName === "SELECT")) return;
  ed.replaceChildren(...(EDITORS[sel.kind] ?? EDITORS.home)().filter((x) => x !== null && x !== undefined));
  if (notice) {
    ed.prepend(el("div", { class: "msg " + notice.kind, role: "status", text: notice.text }));
    notice = null;
  }
}
function renderSide() {
  const ch = $("checks");
  ch.replaceChildren();
  const c = S.checks ?? { lines: [] };
  // 確かめるが 1 つも無いときは、ずっと「試しています…」にしない(第 9 回評価)
  if (!decls("scenario").length) ch.append(el("li", { text: "確かめるはまだありません。左の「確かめる」で動きを日本語で書くと、保存するたびに自動で試します" }));
  else if (!c.lines.length) ch.append(el("li", { text: S.previewPort ? "試しています…(保存すると自動で試します)" : "まだ試していません" }));
  // 検査に合格していない間は試し直せないので、前の結果だと分かるようにする(第 5 回評価: 古い結果が出続けた)
  else if (S.outline && !S.outline.ok) ch.append(el("li", { class: "ng", text: "検査に合格していないので、これは前に試した結果です。下の「問題」を直すと試し直します" }));
  else if (c.stale) ch.append(el("li", { class: "head", text: S.devError ? "アプリを起動できないので、試し直せていません(下は前の結果)。「動かす・初期データ・公開」の理由を見てください" : "直したので試し直しています…(下は直す前の結果)" }));
  // はじめに何件が想定どおりかを大きく出す(記号の行だけでは何が確かめられたか読めなかった。デザインの確認)
  const okN = c.lines.filter((l) => l.trim().startsWith("✓")).length;
  const ngN = c.lines.filter((l) => l.trim().startsWith("✗")).length;
  if (okN + ngN) ch.append(el("li", { class: ngN ? "summary ng" : "summary ok", text: ngN ? `${okN + ngN} 件のうち ${ngN} 件が想定と違います` : `${okN} 件すべて想定どおりに動きました` }));
  // 結果の書き方を日本語に寄せる(「→ 失敗("満席")」→「→ 「満席」で断る」)
  const plain = (t) => t.replace(/→ 成功(\(.+?\))?/, "→ できる").replace(/→ 失敗\("(.+?)"\)/, "→ 「$1」で断る").replace(/→ 権限がない/, "→ 権限が無いので断る").replace(/→ ログインが必要/, "→ ログインが要るので断る");
  for (const l of c.lines) {
    const t = l.trim();
    ch.append(el("li", { class: t.startsWith("✓") ? "ok" : t.startsWith("✗") ? "ng" : "head", text: plain(t), title: t }));
  }
  const pr = $("problems");
  pr.replaceChildren();
  const ds = S.outline?.diagnostics ?? [];
  if (!ds.length) pr.append(el("li", { class: "none", text: "問題はありません" }));
  for (const d of ds.slice(0, 30)) {
    pr.append(el("li", { class: d.severity === "error" ? "" : "warn" }, `${d.file ? d.file + ":" : ""}${d.span?.line ?? ""} ${soft(d.message_ja)}`, ...(d.hints ?? []).slice(0, 2).map((h) => el("span", { class: "hint", text: soft(h) }))));
  }
}

// ---- 日本語で指示 ----
$("ask-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const text = $("ask-text").value;
  if (!text.trim()) return;
  $("ask-send").disabled = true;
  try {
    const j = await post("/api/instruct", { text, screen: sel.kind === "screen" ? sel.name : undefined, scenario: sel.kind === "scenario" ? sel.name : undefined });
    showResult(j);
  } finally {
    $("ask-send").disabled = false;
  }
});
$("ask-text").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
    e.preventDefault();
    $("ask-form").requestSubmit();
  }
});
function showResult(j) {
  const box = $("ask-result");
  box.replaceChildren();
  const r = j.result;
  if (j.error) {
    autoScreenTried = false;
    return box.append(el("span", { class: "err", text: j.error }));
  }
  if (!r) return;
  if (r.ok) {
    autoScreenTried = false;
    box.append(el("span", { class: "ok", text: "✓ " + r.done }));
    $("ask-text").value = "";
    return;
  }
  if (r.need === "screen") {
    // 入れられる画面が 1 つだけなら、聞かずにそこへ入れる(画面を選んでから送り直すのが手間だった。第 10 回評価で 11 人)
    if (r.screens?.length === 1 && !autoScreenTried) {
      autoScreenTried = true;
      select({ kind: "screen", name: r.screens[0] });
      $("ask-form").requestSubmit();
      return;
    }
    autoScreenTried = false;
    box.append(el("span", { class: "err", text: r.message }), el("div", { class: "chips" }, ...r.screens.map((s) => el("button", { type: "button", class: "chip", onclick: () => { select({ kind: "screen", name: s }); $("ask-form").requestSubmit(); } }, `画面「${s}」に入れる`))));
    return;
  }
  for (const e2 of r.errors ?? []) box.append(el("div", { class: "err", text: "この文は読めませんでした: " + soft(e2.message) }), ...(e2.hints ?? []).slice(0, 2).map((h) => el("div", { class: "help", text: soft(h) })));
  if (r.suggestions?.length) {
    box.append(el("div", { class: "help", text: "近い書き方(押すと入ります):" }), el("div", { class: "chips" }, ...r.suggestions.map((s) => el("button", { type: "button", class: "chip", onclick: () => { $("ask-text").value = s; $("ask-text").focus(); } }, s))));
  }
}
$("undo").addEventListener("click", async () => {
  const j = await post("/api/undo", {});
  $("ask-result").replaceChildren(el("span", { class: j.ok ? "ok" : "err", text: j.ok ? "1 つ前に戻しました" : "戻せる変更はありません" }));
});

// ---- 見た目の確認 ----
const DEVICES = [
  { id: "se", name: "iPhone SE", w: 375, h: 667, kind: "iphone home", os: "iOS" },
  { id: "ip15", name: "iPhone 15", w: 393, h: 852, kind: "iphone", os: "iOS" },
  { id: "ip15pm", name: "iPhone 15 Pro Max", w: 430, h: 932, kind: "iphone", os: "iOS" },
  { id: "gs", name: "Galaxy S24", w: 360, h: 780, kind: "android", os: "Android" },
  { id: "px", name: "Pixel 8", w: 412, h: 915, kind: "android", os: "Android" },
  { id: "ga", name: "Galaxy A55", w: 384, h: 854, kind: "android", os: "Android" },
  { id: "ipad", name: "iPad mini", w: 744, h: 1133, kind: "tablet", os: "iPadOS" },
  { id: "pc", name: "パソコン", w: 1280, h: 800, kind: "pc", os: "ブラウザ" },
];
let picks = new Set(store("devices") ?? ["se", "ip15", "gs", "px"]);
let lookPath = store("path") ?? "/";
let lastLookKey = "";
function previewBase() {
  return `http://${location.hostname === "localhost" ? "localhost" : "127.0.0.1"}:${S.previewPort}`;
}
function renderLook() {
  const fs = $("device-picks");
  if (!fs.dataset.ready) {
    for (const d of DEVICES) {
      const c = el("input", { type: "checkbox", checked: picks.has(d.id), onchange: () => { c.checked ? picks.add(d.id) : picks.delete(d.id); store("devices", [...picks]); lastLookKey = ""; renderLook(); } });
      fs.append(el("label", {}, c, d.name));
    }
    fs.dataset.ready = "1";
  }
  const sel2 = $("look-path");
  const paths = [["/", "最初の画面"], ...decls("screen").map((d) => [`/s/${encodeURIComponent(d.name)}`, `画面「${d.name}」`]), ["/login", "ログイン"]];
  const want = paths.map((p) => p[0]).join("|");
  if (sel2.dataset.key !== want) {
    sel2.replaceChildren(...paths.map(([v, l]) => el("option", { value: v, selected: v === lookPath }, l)));
    sel2.dataset.key = want;
  }
  const key = `${S.previewPort}|${[...picks].join(",")}|${lookPath}`;
  if (key === lastLookKey) return;
  lastLookKey = key;
  const grid = $("devices");
  grid.replaceChildren();
  if (!S.previewPort) return grid.append(el("p", { class: "help", text: S.previewNote || "表示の準備中です…" }));
  // 同じ種類の機種は同じ縮尺で並べる(大きさの違いがそのまま見える)
  const SCALE = { iphone: 0.62, "iphone home": 0.62, android: 0.62, tablet: 0.5, pc: 0.5 };
  // 画面の上の、時刻と電池の帯(実機ではページはこの下から始まる)
  const BAR = { iphone: 54, "iphone home": 20, android: 32, tablet: 24, pc: 0 };
  for (const d of DEVICES.filter((x) => picks.has(x.id))) {
    const s = SCALE[d.kind] ?? 0.6;
    const barH = BAR[d.kind] ?? 0;
    const frame = el("iframe", { title: `${d.name} での表示`, src: previewBase() + lookPath });
    frame.style.width = d.w + "px";
    frame.style.height = d.h - barH + "px";
    frame.style.transform = `scale(${s})`;
    const holder = el("div", { class: "page" }, frame);
    holder.style.height = Math.round((d.h - barH) * s) + "px";
    const bar = barH ? el("div", { class: "statusbar " + d.kind.split(" ")[0] }, el("span", { text: "9:41" }), d.kind === "iphone" ? el("span", { class: "island" }) : d.kind === "android" ? el("span", { class: "hole" }) : null, el("span", { class: "batt", text: d.kind === "android" ? "▾ ▮" : "● ▮" })) : null;
    if (bar) bar.style.height = Math.round(barH * s) + "px";
    const screen = el("div", { class: "screen" }, bar, holder);
    screen.style.width = Math.round(d.w * s) + "px";
    screen.style.height = Math.round(d.h * s) + "px";
    grid.append(el("figure", { class: "device" }, el("div", { class: "frame " + d.kind }, screen), el("figcaption", {}, d.name, el("small", { text: `${d.os}・${d.w}×${d.h}` }))));
  }
}
$("look-path").addEventListener("change", (e) => {
  lookPath = e.target.value;
  store("path", lookPath);
  renderLook();
});
$("look-login").addEventListener("click", () => {
  lookPath = "/login";
  lastLookKey = "";
  renderLook();
});
function reloadFrames() {
  for (const f of document.querySelectorAll("#devices iframe")) {
    try {
      f.src = f.src;
    } catch {}
  }
}
$("look-reload").addEventListener("click", reloadFrames);
let reloadTimers = [];
function reloadSoon() {
  // 保存すると koto dev が作り直す(数秒)。終わったころに表示を読み直す
  reloadTimers.forEach(clearTimeout);
  reloadTimers = [4000, 9000].map((ms) => setTimeout(reloadFrames, ms));
}

// ---- ソース ----
function renderSources() {
  const box = $("sources");
  for (const [file, text] of Object.entries(S.sources ?? {})) {
    let wrap = box.querySelector(`[data-file="${CSS.escape(file)}"]`);
    if (!wrap) {
      const ta = el("textarea", { spellcheck: "false", "aria-label": file, oninput: () => dirtySource.add(file) });
      const msg = el("div", { class: "msg", role: "status" });
      wrap = el("div", { class: "file", "data-file": file }, el("h3", { text: file }), ta, el("div", { class: "row" }, el("button", { type: "button", onclick: async () => { await op({ op: "save_source", file, text: ta.value }, msg); dirtySource.delete(file); } }, "ソースを保存する"), msg));
      box.append(wrap);
    }
    const ta = wrap.querySelector("textarea");
    if (!dirtySource.has(file) && ta.value !== text) ta.value = text;
  }
}

// ---- タブ ----
const TABS = [["tab-build", "panel-build"], ["tab-look", "panel-look"], ["tab-run", "panel-run"], ["tab-source", "panel-source"]];
for (const [tab, panel] of TABS) {
  $(tab).addEventListener("click", () => {
    for (const [t, p] of TABS) {
      $(t).setAttribute("aria-selected", String(t === tab));
      $(p).hidden = p !== panel;
    }
    if (panel === "panel-look") {
      lastLookKey = "";
      renderLook();
    }
  });
}

// 確かめるの結果は koto dev の出力から取るので、ときどき読み直す(入力中の欄は描き直さない)
setInterval(async () => {
  if (document.hidden) return;
  try {
    const j = await (await fetch("/api/state")).json();
    if (j.home) return;
    const changed = JSON.stringify(j.checks) !== JSON.stringify(S?.checks) || j.previewPort !== S?.previewPort || j.running !== S?.running;
    S = { ...j };
    if (changed) {
      renderStatus();
      renderSide();
      renderLook();
      renderRunStatus();
    }
  } catch {}
}, 3000);
load();
