#!/usr/bin/env bash
# ペルソナ評価の採点: 各ペルソナの A(KOTONOHA)と B(普段の技術)を起動し、tools/eval/probe.mjs で同じ基準で採点する。
# 使い方: tools/eval/grade.sh <評価フォルダ(P1..P5 を含む)> <出力 JSON>
set -uo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
EVAL=$1
OUT=$2
KOTO="$ROOT/target/debug/koto"
SECRET="grade-secret-0123456789abcdef0123456789abcdef"
SEED="$ROOT/docs/evaluation/persona/seed.json"
port=9100
echo "[" > "$OUT"
first=1
# 手で書いた行数(空行・コメント行を除く)。生成物・依存・データは数えない
count_lines() {
  find "$1" -type f \( -name '*.koto' -o -name '*.sql' -o -name '*.py' -o -name '*.ts' -o -name '*.js' -o -name '*.mjs' -o -name '*.html' -o -name '*.jinja*' -o -name '*.css' -o -name '*.sh' -o -name 'kotonoha.json' \) \
    -not -path '*/node_modules/*' -not -path '*/.venv/*' -not -path '*/venv/*' -not -path '*/out/*' -not -path '*/dist/*' -not -path '*/__pycache__/*' -not -path '*/gen/*' -not -path '*/build/*' -not -name 'seed*.json' -print0 \
    | xargs -0 -r cat | grep -v '^\s*$' | grep -Ev '^\s*(#|//|--)' | wc -l
}
for P in P1 P2 P3 P4 P5; do
  D="$EVAL/$P"
  [ -d "$D" ] || continue
  for V in A B; do
    port=$((port + 3))
    api="" web="" pids=() note=""
    work=$(mktemp -d)
    if [ "$V" = A ]; then
      if "$KOTO" -C "$D/A" check >/dev/null 2>&1 && "$KOTO" -C "$D/A" build --target backend --out "$work/be" >/dev/null 2>&1 && "$KOTO" -C "$D/A" build --target web --out "$work/web" >/dev/null 2>&1; then
        (cd "$work/web" && npm install --no-audit --no-fund --silent >/dev/null 2>&1 && npm run build --silent >/dev/null 2>&1)
        (cd "$work/be" && KOTONOHA_JWT_SECRET=$SECRET KOTONOHA_DB="$work/db.sqlite" node --no-warnings tools/seed.ts "$SEED" >/dev/null 2>&1)
        (cd "$work/be" && PORT=$port KOTONOHA_JWT_SECRET=$SECRET KOTONOHA_DB="$work/db.sqlite" KOTONOHA_RATE_LIMIT_PER_MINUTE=0 exec node --no-warnings src/main.ts >"$work/be.log" 2>&1) & pids+=($!)
        (cd "$work/web" && PORT=$((port + 1)) KOTONOHA_BACKEND_URL=http://127.0.0.1:$port exec node --no-warnings src/server-main.ts >"$work/web.log" 2>&1) & pids+=($!)
        api=http://127.0.0.1:$port web=http://127.0.0.1:$((port + 1))
      else
        note="koto check/build に失敗"
      fi
    else
      if [ -x "$D/B/start.sh" ] || [ -f "$D/B/start.sh" ]; then
        cp "$SEED" "$D/B/seed.json"
        (cd "$D/B" && PORT=$port KOTONOHA_JWT_SECRET=$SECRET DB_PATH="$work/db.sqlite" exec bash start.sh >"$work/b.log" 2>&1) & pids+=($!)
        api=http://127.0.0.1:$port web=$api
      else
        note="start.sh が無い"
      fi
    fi
    result='{"score":0,"total":19,"results":[]}'
    if [ -n "$api" ]; then
      ok=0
      for _ in $(seq 1 150); do curl -s -o /dev/null "$api/api/health" && curl -s -o /dev/null "$web/" && { ok=1; break; }; sleep 0.2; done
      if [ $ok = 1 ]; then
        result=$(node "$ROOT/tools/eval/probe.mjs" --api "$api" --web "$web" --secret "$SECRET" 2>/dev/null || echo '{"score":0,"total":19,"results":[]}')
      else
        note="起動しない"
      fi
    fi
    for p in "${pids[@]}"; do pkill -P "$p" 2>/dev/null; kill "$p" 2>/dev/null; done
    sleep 0.5
    lines=$(count_lines "$D/$V")
    [ $first = 1 ] || echo "," >> "$OUT"
    first=0
    printf '{"persona":"%s","variant":"%s","lines":%s,"note":"%s","probe":%s}' "$P" "$V" "$lines" "$note" "$result" >> "$OUT"
    rm -rf "$work"
  done
done
echo "]" >> "$OUT"
echo "採点結果: $OUT"
