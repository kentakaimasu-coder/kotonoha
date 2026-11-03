#!/usr/bin/env bash
# 見本アプリの iOS / Android を生成し、全 .swift / .kt を tree-sitter で構文検査する
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/tools/mobile-syntax"
[ -d "$HERE/node_modules" ] || (cd "$HERE" && npm install --no-audit --no-fund --silent)
OUT="$ROOT/target/mobile-syntax"
rm -rf "$OUT"
"$ROOT/target/debug/koto" -C "$ROOT/examples/shop" build --target ios --out "$OUT/ios" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/shop" build --target android --out "$OUT/android" >/dev/null 2>&1
node "$HERE/check.cjs" $(find "$OUT" -name '*.swift' -o -name '*.kt' | sort)
