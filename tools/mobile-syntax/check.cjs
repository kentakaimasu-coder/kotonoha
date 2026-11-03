// tree-sitter で構文エラー(ERROR / MISSING ノード)を数える
const Parser = require("web-tree-sitter");
const fs = require("fs");
const path = require("path");
(async () => {
  await Parser.init();
  const langs = {
    swift: await Parser.Language.load(require.resolve("tree-sitter-wasms/out/tree-sitter-swift.wasm")),
    kt: await Parser.Language.load(require.resolve("tree-sitter-wasms/out/tree-sitter-kotlin.wasm")),
  };
  let bad = 0;
  for (const f of process.argv.slice(2)) {
    const ext = path.extname(f).slice(1);
    const p = new Parser();
    p.setLanguage(langs[ext]);
    const tree = p.parse(fs.readFileSync(f, "utf8"));
    const errs = [];
    const walk = (n) => {
      if (n.type === "ERROR" || n.isMissing) errs.push(`${n.startPosition.row + 1}:${n.startPosition.column + 1} ${n.isMissing ? "MISSING " + n.type : "ERROR"} ${JSON.stringify(n.text.slice(0, 60))}`);
      for (const c of n.children) walk(c);
    };
    walk(tree.rootNode);
    console.log(`${errs.length === 0 ? "OK " : "NG "} ${path.basename(f)} ${errs.length ? "\n  " + errs.slice(0, 8).join("\n  ") : ""}`);
    if (errs.length) bad++;
  }
  process.exit(bad ? 1 : 0);
})();
