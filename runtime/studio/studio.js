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
  renderAll();
}
let notice = null; // 編集の欄に出す直前の結果({kind, text})
async function post(path, body) {
  const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json", "X-Studio-Token": TOKEN }, body: JSON.stringify(body) });
  const j = await r.json();
  if (j.outline) S = j;
  if (j.error) {
    // 失敗したときは編集の欄を描き直さない(書いた内容を残す)。ほかの欄だけ新しくする
    renderSide();
  } else {
    renderAll();
    reloadSoon();
  }
  return j;
}
async function op(o, msgBox) {
  const j = await post("/api/op", o);
  const text = j.error ? j.error : "✓ 入れました(ソースに書きました)";
  if (msgBox && msgBox.isConnected) {
    msgBox.textContent = text;
    msgBox.className = "msg " + (j.error ? "err" : "ok");
  }
  if (!j.error) notice = { kind: "ok", text };
  return j;
}
const decls = (kind) => (S?.outline?.decls ?? []).filter((d) => d.kind === kind);
const srcLines = (file) => (S?.sources?.[file] ?? "").split("\n");
const diagsAt = (file, line) => (S?.outline?.diagnostics ?? []).filter((d) => d.file === file && d.span?.line === line);
const typeChoices = () => [...(S?.outline?.types ?? []), ...Object.keys(S?.outline?.enums ?? {})];

// ---- 全体 ----
function renderAll() {
  if (!S) return;
  $("app-name").textContent = S.outline?.app ?? "";
  const errs = (S.outline?.diagnostics ?? []).filter((d) => d.severity === "error").length;
  const st = $("status");
  st.textContent = errs === 0 ? "✓ 検査に合格" : `✗ 問題 ${errs} 件`;
  st.className = "pill " + (errs === 0 ? "ok" : "err");
  $("undo").disabled = !S.canUndo;
  $("ask-target").textContent = sel.kind === "screen" ? `(入れ先: 画面「${sel.name}」)` : sel.kind === "scenario" ? `(入れ先: 確かめる「${sel.name}」)` : "";
  renderTree();
  renderEditor();
  renderSide();
  renderLook();
  renderSources();
}

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
  t.append(el("h3", { text: "約束(見てよい人・重複・順番)" }));
  t.append(item(decls("rules").length ? "約束" : "+ 約束を書く", { kind: "rules", name: "約束" }, !decls("rules").length));
  group("確かめる(動きの確認)", "scenario", "新しい確かめる", "new-scenario");
}
function select(s) {
  sel = s;
  renderAll();
  $("editor").focus?.();
}

// ---- 編集の欄 ----
function lineList(d, opts = {}) {
  const ls = srcLines(d.file);
  const box = el("div", { class: "rows" });
  for (let n = d.line + 1; n <= d.end; n++) {
    const text = ls[n - 1] ?? "";
    if (!text.trim() || text.trim().startsWith("#")) continue;
    const bad = diagsAt(d.file, n).filter((x) => x.severity === "error");
    box.append(
      el(
        "div",
        { class: "line" + (bad.length ? " bad" : "") },
        el("code", {}, text.trim(), ...bad.map((b) => el("span", { class: "why", text: b.message_ja + (b.hints?.length ? "(" + b.hints[0] + ")" : "") }))),
        opts.removable === false ? null : el("button", { type: "button", class: "small", "aria-label": "この行を消す", onclick: () => op({ op: "remove_line", file: d.file, line: n, old: text }) }, "消す"),
      ),
    );
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
        const j = await op(kind === "rules" ? { op: "add_rule", text: input.value } : { op: "add_line", kind, name, text: input.value }, msg);
        if (!j.error) input.value = "";
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

const EDITORS = {
  home() {
    const themes = ["藍", "紺", "青", "緑", "深緑", "朱", "紅", "紫", "橙", "茶", "墨"];
    const pick = el("select", { "aria-label": "テーマ色" }, el("option", { value: "" }, "(既定: 藍)"), ...themes.map((c) => el("option", { value: c, selected: c === S.theme }, c)));
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
  data() {
    const d = decls("data").find((x) => x.name === sel.name);
    if (!d) return [el("p", { text: "見つかりません" })];
    const ls = srcLines(d.file);
    const rows = el("div", { class: "rows" });
    for (const f of d.fields) {
      const bad = diagsAt(d.file, f.line).filter((x) => x.severity === "error");
      rows.append(
        el("div", { class: "line" + (bad.length ? " bad" : "") }, el("code", {}, `${f.name}: ${f.type}`, ...bad.map((b) => el("span", { class: "why", text: b.message_ja }))), f.name === "ID" ? el("span", { class: "help", text: "自動で決まります" }) : el("button", { type: "button", class: "small", onclick: () => op({ op: "remove_line", file: d.file, line: f.line, old: ls[f.line - 1] }) }, "消す")),
      );
    }
    const box = el("div", { class: "rows" });
    const add = fieldRows(box, [["", "文字列"]]);
    const msg = el("div", { class: "msg", role: "status" });
    return [
      el("h2", { text: `データ「${d.name}」` }),
      el("p", { class: "sub", text: "覚えておく項目の一覧です。ID は自動で付きます。" }),
      rows,
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
      el("div", { class: "row" }, el("button", { type: "button", class: "ghost", onclick: () => add() }, "項目をもう 1 つ"), el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_data", name: name.value, fields: [...box.children].map((r) => r.read()) }, msg); if (!j.error) select({ kind: "data", name: name.value.trim() }); } }, "作る")),
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
      addLineForm("screen", d.name, "例: 予約を新しい順に一覧表示 / 「予約する」を押すと 予約する(枠.ID, 1)", S.outline.patterns.screen),
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
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_screen", name: name.value, lines: [first.value] }, msg); if (!j.error) select({ kind: "screen", name: name.value.trim() }); } }, "作る"),
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
      el("p", { class: "sub", text: `引数: ${d.params.map((p) => `${p.name}(${p.type})`).join("、") || "なし"}。処理は上から順に行い、失敗したら全部元に戻ります。` }),
      lineList(d),
      el("h3", { text: "処理を足す" }),
      el("div", { class: "row" }, tpl("1 件取り出す", `件 = ${D}を取得(${D}ID)`), tpl("条件を確かめる", `件.数 > 0 を確認 または 失敗("不正")`), tpl("書き換える", `件.状況 = "完了"`), tpl("登録する", `${D}を登録(${(decls("data")[0]?.fields ?? []).filter((f) => f.name !== "ID").map((f) => `${f.name}: …`).join(", ")}) を返す`), tpl("ID を返す", `${D}ID を返す`)),
      el("form", { class: "row", onsubmit: async (e) => { e.preventDefault(); const j = await op({ op: "add_line", kind: "action", name: d.name, text: input.value }, msg); if (!j.error) input.value = ""; } }, input, el("button", { type: "submit" }, "足す")),
      msg,
      el("p", { class: "help", text: "行の順番を変えるときは「ソース」で直します。" }),
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
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_action", name: name.value, params: [...pbox.children].map((r) => r.read()), body: body.value.split("\n"), fail: fail.value }, msg); if (!j.error) select({ kind: "action", name: name.value.trim() }); } }, "作る"),
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
      el("button", { type: "button", onclick: async () => { const j = await op({ op: "add_scenario", name: name.value, lines: lines.value.split("\n") }, msg); if (!j.error) select({ kind: "scenario", name: name.value.trim() }); } }, "作る"),
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
    el("button", { type: "button", onclick: async () => { const j = await op(d ? { op: "set_role_caps", name: d.name, caps: caps() } : { op: "add_role", name: name.value, caps: caps() }, msg); if (!j.error && !d) select({ kind: "role", name: name.value.trim() }); } }, d ? "保存する" : "作る"),
    msg,
    d ? el("hr") : null,
    d ? el("button", { type: "button", class: "danger", onclick: () => confirm(`役割「${d.name}」を消しますか?`) && op({ op: "remove_decl", kind: "role", name: d.name }).then(() => select({ kind: "home" })) }, "この役割を消す") : null,
  ];
}
function renderEditor() {
  const ed = $("editor");
  const active = document.activeElement;
  // 入力中は描き直さない(書いている途中の文を消さない)
  if (ed.contains(active) && (active.tagName === "INPUT" || active.tagName === "TEXTAREA") && active.value) return;
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
  if (!c.lines.length) ch.append(el("li", { text: S.previewPort ? "試しています…(保存すると自動で試します)" : "まだ試していません" }));
  for (const l of c.lines) {
    const t = l.trim();
    ch.append(el("li", { class: t.startsWith("✓") ? "ok" : t.startsWith("✗") ? "ng" : "head", text: t }));
  }
  const pr = $("problems");
  pr.replaceChildren();
  const ds = S.outline?.diagnostics ?? [];
  if (!ds.length) pr.append(el("li", { class: "none", text: "問題はありません" }));
  for (const d of ds.slice(0, 30)) {
    pr.append(el("li", { class: d.severity === "error" ? "" : "warn" }, `${d.file ? d.file + ":" : ""}${d.span?.line ?? ""} ${d.message_ja}`, ...(d.hints ?? []).slice(0, 2).map((h) => el("span", { class: "hint", text: h }))));
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
  if (j.error) return box.append(el("span", { class: "err", text: j.error }));
  if (!r) return;
  if (r.ok) {
    box.append(el("span", { class: "ok", text: "✓ " + r.done }));
    $("ask-text").value = "";
    return;
  }
  if (r.need === "screen") {
    box.append(el("span", { class: "err", text: r.message }), el("div", { class: "chips" }, ...r.screens.map((s) => el("button", { type: "button", class: "chip", onclick: () => { select({ kind: "screen", name: s }); $("ask-form").requestSubmit(); } }, `画面「${s}」に入れる`))));
    return;
  }
  for (const e2 of r.errors ?? []) box.append(el("div", { class: "err", text: "この文は読めませんでした: " + e2.message }), ...(e2.hints ?? []).slice(0, 2).map((h) => el("div", { class: "help", text: h })));
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
  if (!S.previewPort) return grid.append(el("p", { class: "help", text: "表示の準備中です…" }));
  // 同じ種類の機種は同じ縮尺で並べる(大きさの違いがそのまま見える)
  const SCALE = { iphone: 0.62, "iphone home": 0.62, android: 0.62, tablet: 0.5, pc: 0.5 };
  // 画面の上の、時刻と電池の帯(実機ではページはこの下から始まる)
  const BAR = { iphone: 54, "iphone home": 20, android: 32, tablet: 24, pc: 0 };
  for (const d of DEVICES.filter((x) => picks.has(x.id))) {
    const s = SCALE[d.kind] ?? 0.6;
    const barH = BAR[d.kind] ?? 0;
    const frame = el("iframe", { title: `${d.name} での表示`, src: previewBase() + lookPath, loading: "lazy" });
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
      wrap = el("div", { class: "file", "data-file": file }, el("h3", { text: file }), ta, el("div", { class: "row" }, el("button", { type: "button", onclick: async () => { await op({ op: "save_source", file, text: ta.value }, msg); dirtySource.delete(file); } }, "保存する"), msg));
      box.append(wrap);
    }
    const ta = wrap.querySelector("textarea");
    if (!dirtySource.has(file) && ta.value !== text) ta.value = text;
  }
}

// ---- タブ ----
for (const [tab, panel] of [["tab-build", "panel-build"], ["tab-look", "panel-look"], ["tab-source", "panel-source"]]) {
  $(tab).addEventListener("click", () => {
    for (const [t, p] of [["tab-build", "panel-build"], ["tab-look", "panel-look"], ["tab-source", "panel-source"]]) {
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
    const changed = JSON.stringify(j.checks) !== JSON.stringify(S?.checks) || j.previewPort !== S?.previewPort;
    S = { ...j };
    if (changed) {
      renderSide();
      renderLook();
    }
  } catch {}
}, 3000);
load();
