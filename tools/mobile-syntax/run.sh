#!/usr/bin/env bash
# 見本アプリ(shop / lab)の iOS / Android を生成し、全 .swift / .kt を tree-sitter で構文検査する
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/tools/mobile-syntax"
[ -d "$HERE/node_modules" ] || (cd "$HERE" && npm install --no-audit --no-fund --silent)
OUT="$ROOT/target/mobile-syntax"
rm -rf "$OUT"
"$ROOT/target/debug/koto" -C "$ROOT/examples/shop" build --target ios --out "$OUT/ios" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/shop" build --target android --out "$OUT/android" >/dev/null 2>&1
# 拡張部品・分析・実験を含む例
"$ROOT/target/debug/koto" -C "$ROOT/examples/lab" build --target ios --out "$OUT/lab-ios" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/lab" build --target android --out "$OUT/lab-android" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/event" build --target ios --out "$OUT/event-ios" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/event" build --target android --out "$OUT/event-android" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/izakaya" build --target ios --out "$OUT/izakaya-ios" >/dev/null 2>&1
"$ROOT/target/debug/koto" -C "$ROOT/examples/izakaya" build --target android --out "$OUT/izakaya-android" >/dev/null 2>&1
node "$HERE/check.cjs" $(find "$OUT" -name '*.swift' -o -name '*.kt' | sort)
