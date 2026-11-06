#!/usr/bin/env python3
"""生成物を schemas/*.schema.json で検証する(CI 用)。

使い方: python3 tools/validate_schemas.py <koto 実行ファイル>
一時ディレクトリに見本プロジェクトを作り、koto の実際の出力(VAG・候補・承認・ロック)を検証する。
"""
import json, pathlib, shutil, subprocess, sys, tempfile
import jsonschema

# Windows の既定(cp1252 等)でも日本語・記号を出力・読み取りできるようにする
sys.stdout.reconfigure(encoding="utf-8")

ROOT = pathlib.Path(__file__).resolve().parent.parent
def schema(n): return json.loads((ROOT / "schemas" / n).read_text(encoding="utf-8"))
def check(name, data, s):
    jsonschema.Draft202012Validator(schema(s)).validate(data)
    print(f"OK  {name} ⊨ {s}")

koto = sys.argv[1]
check("examples/shop/expected/vag.json", json.loads((ROOT / "examples/shop/expected/vag.json").read_text(encoding="utf-8")), "vag.schema.json")

with tempfile.TemporaryDirectory() as d:
    d = pathlib.Path(d)
    shutil.copytree(ROOT / "examples/shop", d / "shop")
    p = d / "shop"
    src = (p / "src/app.koto").read_text(encoding="utf-8") + "\n@柔軟(確認)\n画面 人気:\n    人気商品を表示\n"
    (p / "src/app.koto").write_text(src, encoding="utf-8")
    (p / "fx.json").write_text(json.dumps({"人気商品を表示": {"interpretations": [{"canonical": "売れ筋商品を表示", "rationale": "r"}], "assumptions": [], "unresolved": [], "required_effects": ["data.read"], "evidence_refs": []}}, ensure_ascii=False), encoding="utf-8")
    run = lambda *a: subprocess.run([koto, "-C", str(p), *a], capture_output=True, text=True, encoding="utf-8")
    r = run("--json", "resolve", "--adapter", "fixture", "--fixture", "fx.json"); assert r.returncode == 0, r.stderr
    cands = json.loads((p / "kotonoha.candidates.json").read_text(encoding="utf-8"))
    for c in cands["candidates"]: check("candidate " + c["intent_id"], c, "intent-candidate.schema.json")
    r = run("approve", cands["candidates"][0]["intent_id"], "1", "--approver", "CI", "--at", "2026-11-03T00:00:00Z"); assert r.returncode == 0, r.stderr
    check("kotonoha.approvals.json", json.loads((p / "kotonoha.approvals.json").read_text(encoding="utf-8")), "approvals.schema.json")
    r = run("approve", "--write-lock"); assert r.returncode == 0, r.stderr
    check("kotonoha.lock.json", json.loads((p / "kotonoha.lock.json").read_text(encoding="utf-8")), "lock.schema.json")
    r = run("graph"); assert r.returncode == 0, r.stderr
    check("graph (承認済み解釈入り)", json.loads(r.stdout), "vag.schema.json")
print("全スキーマ検証に合格")
