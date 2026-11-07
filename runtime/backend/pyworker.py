# KOTONOHA backend runtime: Python 外部関数の常駐ワーカー(ADR-0124)
# 標準入力から {"id", "fn", "args"} を 1 行ずつ読み、{"id", "ok"} または {"id", "error"} を 1 行で返す。
# 引数・戻り値は API と同じワイヤ形式(JSON)。戻り値は backend が宣言した型で検証する。
import importlib.util
import json
import sys


def main() -> None:
    path = sys.argv[1]
    spec = importlib.util.spec_from_file_location("kotonoha_ext", path)
    if spec is None or spec.loader is None:
        sys.stderr.write("cannot load extension module\n")
        sys.exit(2)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    out = sys.stdout
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            rid = req["id"]
        except Exception:
            continue
        try:
            fn = getattr(module, req["fn"], None)
            if fn is None or not callable(fn):
                raise LookupError("function not found")
            res = {"id": rid, "ok": fn(*req["args"])}
            text = json.dumps(res, ensure_ascii=False, allow_nan=False)
        except Exception as e:  # noqa: BLE001  詳細は標準エラーにだけ出す(応答に内部情報を含めない)
            sys.stderr.write(f"extension error: {type(e).__name__}: {e}\n")
            text = json.dumps({"id": rid, "error": "failed"})
        out.write(text + "\n")
        out.flush()


if __name__ == "__main__":
    main()
