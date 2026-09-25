#!/usr/bin/env bash
# sopack pack benchmark runner — SOPACK-1.0-PLAN.md §3.3 "Benchmark gate" (M4).
#
# Runs `sopack pack` on one already-extracted book.json at a given thread
# count / batch-tokens budget, under the shared model flock (RAM is shared
# with other agents on this machine — see common.md), and writes one JSON
# result file per run into bench/results/.
#
# It parses the run's `--progress json` stream (NDJSON on stderr) rather
# than trusting `time`, because the pack wall clock also includes a fixed
# ~10-20s of model-file sha256 verification + model load + the mandatory
# calibration-fixture embed that happens before any book text is touched
# (see `sopack pack` step order in ../README.md). Isolating the *book's own*
# "embed" stage_end event (the last one — calibration's embed always runs
# first) gives blocks/s and tokens/s that are comparable across runs
# regardless of that fixed overhead. Peak RSS is sampled from
# /proc/<pid>/status VmHWM every 0.2s in the background, since this
# container has no /usr/bin/time -v.
#
# Usage:
#   run_bench.sh <label> <book.json> <out_dir> <model_dir> [--threads N] [--batch-tokens N]
#
# Writes bench/results/<label>.json and prints a one-line summary.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOPACK_RS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN="$SOPACK_RS_DIR/target/release/sopack"
RESULTS_DIR="$SCRIPT_DIR/results"
LOCK_FILE="${SOPACK_BENCH_LOCK:-/home/vscode/.claude/jobs/00f226f6/tmp/model.lock}"

label="$1"; shift
book_json="$1"; shift
out_dir="$1"; shift
model_dir="$1"; shift

extra_args=()
threads=""
batch_tokens=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --threads) threads="$2"; extra_args+=(--threads "$2"); shift 2 ;;
    --batch-tokens) batch_tokens="$2"; extra_args+=(--batch-tokens "$2"); shift 2 ;;
    *) extra_args+=("$1"); shift ;;
  esac
done

mkdir -p "$RESULTS_DIR" "$out_dir"
out_sopack="$out_dir/${label}.sopack"
rm -rf "${out_sopack}.partial" 2>/dev/null || true
progress_log="$out_dir/${label}.progress.ndjson"
rss_log="$out_dir/${label}.rss.txt"

: "${ORT_DYLIB_PATH:?ORT_DYLIB_PATH must be set (see common.md)}"

run_cmd=("$BIN" pack "$book_json" -o "$out_sopack" --model-dir "$model_dir" --fresh --progress json "${extra_args[@]}")

echo "[run_bench] $label: ${run_cmd[*]}" >&2

start_ns=$(date +%s%N)
flock "$LOCK_FILE" bash -c '
  "$@" 2>"'"$progress_log"'" &
  pid=$!
  peak=0
  while kill -0 "$pid" 2>/dev/null; do
    if [[ -r "/proc/$pid/status" ]]; then
      kb=$(awk "/VmHWM/{print \$2}" "/proc/$pid/status" 2>/dev/null || true)
      if [[ -n "$kb" ]] && (( kb > peak )); then peak=$kb; fi
    fi
    sleep 0.2
  done
  echo "$peak" > "'"$rss_log"'"
  wait "$pid"
' _ "${run_cmd[@]}"
end_ns=$(date +%s%N)
wall_s=$(awk -v a="$start_ns" -v b="$end_ns" 'BEGIN{printf "%.3f", (b-a)/1e9}')
peak_rss_kb=$(cat "$rss_log" 2>/dev/null || echo 0)

# Isolate the book's own embed stage: the LAST {"event":"stage_end","stage":"embed",...}
# line (calibration's fixture embed is always first within one pack run).
embed_line=$(grep '"stage":"embed"' "$progress_log" | grep '"event":"stage_end"' | tail -1 || true)

python3 - "$label" "$book_json" "$wall_s" "$peak_rss_kb" "$embed_line" "${threads:-default}" "${batch_tokens:-default}" "$RESULTS_DIR" <<'PYEOF'
import json, sys, os

label, book_json, wall_s, peak_rss_kb, embed_line, threads, batch_tokens, results_dir = sys.argv[1:9]

with open(book_json) as f:
    book = json.load(f)
blocks = len(book.get("blocks", []))

embed_tokens = None
embed_elapsed_s = None
if embed_line:
    ev = json.loads(embed_line)
    embed_tokens = ev.get("total")
    embed_elapsed_s = ev.get("elapsed_s")

result = {
    "label": label,
    "book_json": book_json,
    "blocks": blocks,
    "threads": threads,
    "batch_tokens": batch_tokens,
    "wall_s": float(wall_s),
    "peak_rss_kb": int(peak_rss_kb) if peak_rss_kb else None,
    "embed_stage_tokens": embed_tokens,
    "embed_stage_elapsed_s": embed_elapsed_s,
    "blocks_per_s": (blocks / embed_elapsed_s) if embed_elapsed_s else None,
    "tokens_per_s": (embed_tokens / embed_elapsed_s) if (embed_tokens and embed_elapsed_s) else None,
}

out_path = os.path.join(results_dir, f"{label}.json")
with open(out_path, "w") as f:
    json.dump(result, f, indent=2)
    f.write("\n")

print(f"[run_bench] {label}: blocks={blocks} threads={threads} batch_tokens={batch_tokens} "
      f"wall_s={wall_s} embed_elapsed_s={embed_elapsed_s} "
      f"blocks_per_s={result['blocks_per_s']} tokens_per_s={result['tokens_per_s']} "
      f"peak_rss_kb={peak_rss_kb}")
PYEOF
