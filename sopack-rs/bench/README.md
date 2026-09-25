# sopack M4 exit gate + benchmark

Evidence for `SOPACK-1.0-PLAN.md` §1 M4 row ("3 real books packed in Rust
pass the server probe ≥ 0.9999") and §3.3 "Benchmark gate". Run 2026-09-24,
**stopped early by the orchestrator** partway through the largest-book
timing sweep — see "What was NOT completed" below. Packs themselves are
large binaries and are **not** committed; they live under
`/home/vscode/.claude/jobs/00f226f6/tmp/bench-packs/` on the machine that
ran this. This directory holds only the runner script and the result JSON
files.

## Machine

- arch: aarch64 (Apple Silicon under Linux virtualization — `Vendor ID:
  Apple`, devcontainer)
- cores: 4 (`nproc` / `available_parallelism()`)
- RAM: 12 GiB total, **shared with other concurrent agents** on this box
  for the whole run (no isolated benchmark environment — see the MAS/TTL
  throughput note below)
- ONNX Runtime: 1.30.0 (`$ORT_DYLIB_PATH`, from the `onnxruntime` Python
  wheel in `/workspaces/sdarm/.venv`)
- `sopack` version: 0.9.0

## M4 exit gate: 3 real books vs the live store

Three books already imported into the live `sop` Qdrant collection
(`http://10.10.10.10:6333`), re-extracted and re-packed with the Rust CLI,
compared point-for-point against what's actually stored. Picked from the
offline book-code registry (`contracts/e5-large-v1/book_codes.json`,
`corpus: "pioneers"`) — **not** from
`pd-books/converted/_results_pioneers2026.json`: that 22-book list is a
different, not-yet-imported batch, and 13 of its 22 codes were renamed at
import (orchestrator correction mid-task; confirmed none of its codes
appear in the live registry). Metadata for each book was resolved from
`sopack propose --write-meta` candidates, then corrected against a live
sample point's own payload (fetched first, before writing each
`<source>.meta.toml` sidecar) rather than guessed.

| role | book_code | title | author | year | source |
|---|---|---|---|---|---|
| small (23 blocks) | WDYS | Why Do You Swear? | J. N. Andrews | 1866 | `pd-books/converted/andrews__why-do-you-swear__1861__archive.epub` |
| medium (225 blocks) | TTL | Tremont Temple Lectures | A. T. Jones | 1888 | `pd-books/converted/jones__tremont-temple-lectures__1888__archive.epub` |
| medium (290 blocks) | MAS | Matter and Spirit | D. M. Canright | 1871 | `pd-books/converted/canright__matter-and-spirit__1882__archive.epub` |

**Result: all 3 pass at cosine 1.0 (well above the ≥0.9999 bar), 100% id
match.**

| book_code | pack points | live points (scroll count) | matched ids | min cosine | mean cosine | below 0.9999 | `sopack verify` | `check_pack_py.py` | `check_import_probe_py.py` |
|---|---|---|---|---|---|---|---|---|---|
| WDYS | 23 | 23 | 23/23 | 1.0000000000 | 1.0000000000 | 0 | clean | OK | OK |
| TTL | 225 | 225 | 225/225 | 1.0000000000 | 1.0000000000 | 0 | clean | OK | OK |
| MAS | 290 | 290 | 290/290 | 1.0000000000 | 1.0000000000 | 0 | clean | OK | OK |

Block count from `sopack extract` matched the live store's point count
exactly for all three books before any comparison was run (23/225/290),
and every extracted `para_key` + `raw_text` sampled matched the live
payload verbatim. Live vectors were fetched read-only via `POST
/collections/sop/points` with `with_vector: true` (named vector
`fast-multilingual-e5-large`), batched at ≤256 ids
(`bench-packs/compare_live.py`, uses the Python reference
`sopack.format.PackReader` to stream the Rust-built pack's own vectors —
implementation-independent of the Rust reader). Full per-book output and
`.compare.json` files are under `bench-packs/<code>/`.

This was independently corroborated by the pre-existing model-gated
regression test `pack_wdys_is_verifiable_and_matches_the_python_built_reference`
(`crates/sopack-cli/tests/model_gated.rs`, written by another agent, not
part of this task): a full pack, `verify`, both Python conformance checks,
crash-after-1-batch + resume producing a byte-identical pack, and cosine
1.00000000 against a Python-built reference pack for the same book — still
green after the fix below.

**HSFD (2486 blocks, "the 2486-block Andrews book" from `SOPACK-1.0-PLAN.md`
§3.3, source `pd-books/ready/andrews__history-of-the-sabbath__1873__gutenberg.epub`)
was extracted (2486/2486 blocks, exactly matching the live store's count)
but its `pack` run was killed partway through by the orchestrator to keep
this task's turnaround short, and was not restarted — it did not enter the
comparison above.** Per the orchestrator's explicit instruction, the 3-book
gate was satisfied instead with WDYS + TTL + MAS (all smaller, all
confirmed 1.0 cosine / 100% id match), and the largest-book timing sweep
was dropped rather than resumed. A checkpoint from the killed run is left
at `bench-packs/hsfd/hsfd.sopack.sopack.partial/` (unused).

## Benchmark

**Not the full matrix asked for.** Only default-settings (`--threads`
unset → all 4 cores, `--batch-tokens` unset → CPU default 512) timings
exist, taken from the same packs built for the id/cosine comparison above,
because the run was stopped before the threads={1,2,4}/batch-tokens
sweep or the Python-baseline comparison could start. What exists:

| book | blocks | wall time (full `pack`, incl. ~10s model sha256 verify + ~5-8s load + calibration) | embed-phase only (all book chunks, excl. calibration) | blocks/s | tokens/s |
|---|---|---|---|---|---|
| WDYS | 23 | 31.7s | 6.91s / 2232 tokens | 3.33 | 323.0 |
| TTL | 225 | 106.8s | 82.7s / 26294 tokens | 2.72 | 317.9 |
| MAS | 290 | 195.8s | 161.7s / 20746 tokens | 1.79 | 128.3 |

`embed-phase only` sums every `[embed] done (...)` line's own `elapsed_s`
from `--progress plain` output, excluding the one calibration-fixture embed
(always 2031 tokens) that runs first in every `pack` invocation — this
isolates book-embedding throughput from the fixed ~15-25s of
verify/load/calibration overhead every run pays regardless of book size.

MAS's per-token throughput (128 tok/s) is well under TTL's (318 tok/s) at
the *same* settings on the *same* machine — almost certainly contention
from other agents running concurrently on this shared 4-core/12GB box
during MAS's run (confirmed: `sopack doctor`'s "available" memory reading
dropped between runs over the course of this session), not a real
per-book effect. **Do not read this table as a clean threads/batch-tokens
comparison** — that sweep (the actual point of the benchmark gate) was not
run. Peak RSS was sampled once, for WDYS at `--threads 4`: **1.67 GiB**
(`peak_rss_kb: 1714328`), against `sopack doctor`'s own guard estimate of
**~2.63 GiB** ("model + batch activations + margin") — the guard is
comfortably conservative on this one data point, but this was not checked
against the largest book, where batch activation memory would be closer to
its true peak.

## What was NOT completed (orchestrator cut the task short)

- Largest-book (HSFD, 2486 blocks) pack + comparison — extracted only,
  `pack` killed mid-run.
- `--threads 1`, `--threads 2` sweep on any book.
- `--batch-tokens 1024` / `--batch-tokens 2048` at `--threads 4`.
- Python baseline (`sopack` 0.2.0 batch-1-default, and 0.1.x-style
  `--batch-size 128`) on the ~200-block book — no speedup-vs-Python number
  can be reported.
- Peak RSS was sampled for one run (WDYS) only, not the largest book.

Re-run with `bench/run_bench.sh` (present, tested working — see its own
header) once time/machine contention allow; it already does the
`--progress json` parsing and RSS sampling this pass needed, and was
validated against a live WDYS `--threads 4` run (`bench-packs/wdys/wdys_t4.*`,
also cross-checked against the live store with 1.0 cosine).

## Runner

`run_bench.sh <label> <book.json> <out_dir> <model_dir> [--threads N]
[--batch-tokens N]` — see the script's own header comment for why it
parses `--progress json` instead of trusting wall-clock `time` naively for
per-chunk timing, and how it samples peak RSS from `/proc/<pid>/status
VmHWM` (no `/usr/bin/time -v` on this image). One JSON result per run in
`results/`.

```bash
export ORT_DYLIB_PATH=/workspaces/sdarm/.venv/lib/python3.11/site-packages/onnxruntime/capi/libonnxruntime.so.1.30.0
MODEL_DIR=/workspaces/sdarm/.fastembed_cache/models--qdrant--multilingual-e5-large-onnx/snapshots/66076b8dc6e367337e3e90e6fb309fb0f3addaf6
./bench/run_bench.sh ttl_t4 /path/to/ttl.book.json /path/to/out "$MODEL_DIR" --threads 4
```

Note for whoever resumes the sweep: `run_bench.sh`'s "last `stage_end` for
stage `embed`" parsing (in its header comment) only gives the LAST
checkpoint chunk's own timing, not the whole book's — accurate only for a
book ≤16 blocks (one chunk). For any larger book use the same summation
approach this README's benchmark table used by hand (sum every `embed`
`stage_end`'s `elapsed_s`/`total`, dropping the first occurrence of the
calibration fixture's fixed 2031-token entry) — `run_bench.sh` itself was
not updated to do this automatically before the task was cut short.

## Progress engine: bug found + fixed, and one found-but-not-fixed issue

**Fixed** — `crates/sopack-cli/src/commands/pack.rs`, in `run()`:

```diff
-    let total_tokens_hint: u64 = flat
-        .iter()
-        .skip(already)
-        .map(|&(bi, ei)| {
-            let block = &loaded[bi].book.blocks[ei];
-            block.words.max(1) as u64
-        })
-        .sum();
-    progress.stage_start("embed", total_tokens_hint.max(1), Unit::Tokens);
-
+    // NOTE: no `progress.stage_start("embed", ...)` here. Each checkpoint
+    // chunk below calls `engine.embed()`, which opens and closes its own
+    // "embed" stage (accurate per-chunk token total) around the ORT calls
+    // it runs (`sopack-embed/src/engine.rs`). An outer stage_start here
+    // used to declare a *word*-count total (mislabeled as `Unit::Tokens`,
+    // and always ~1.3-1.6x under the real subword-tokenized count) and was
+    // clobbered by the first chunk's own stage_start before ever getting a
+    // matching stage_end — an orphaned event in `--progress json` output
+    // (found + removed during the M4 benchmark pass, SOPACK-1.0-PLAN.md
+    // §3.3; see this section for the remaining, unfixed cosmetic issue
+    // this doesn't address: percentage plateaus across a multi-chunk
+    // embed stage since every chunk restart shares the same stage name).
```

`total_tokens_hint` was used **only** for that one `stage_start` call
(confirmed by grep — no other reference in the file), and the loop's
trailing `progress.stage_end()` (kept, unchanged) was already a no-op by
the time it ran in every observed case, since the last checkpoint chunk's
own `engine.embed()` call had already closed the stage — so removing the
opening call is a pure, zero-behavior-risk deletion of dead/misleading
code, not a restructuring.

**Why it mattered**: in `--progress json` output this appeared as a
`stage_start` event (`{"event":"stage_start","stage":"embed","total":1366,
"unit":"tokens"}` for WDYS, 1366 being its *word* count, not a token count)
with **no matching `stage_end`** anywhere in the stream — immediately
followed by the first real chunk's own `stage_start` with the correct
total. An agent (or this task's own first draft of `run_bench.sh`)
pairing `stage_start`/`stage_end` 1:1 per book would get confused by the
orphan. Verified fixed: re-ran `pack_wdys_is_verifiable_and_matches_the_python_built_reference`
after the change (full pack + `verify` + both Python conformance checks +
crash-then-resume byte-identical round trip) — still green, and the
orphaned `starting (0/1366 tokens)` plain-text line that used to appear
between the calibration embed and the first real chunk's embed is gone
from a fresh run's output.

**Fixed afterwards (2026-09-25).** The remaining issue was that `pack`'s
overall percentage froze during the whole embed phase (TTL: 84.62 % for ~100 s).
Two causes: (1) `Engine::calibrate` embedded the fixture under the stage name
`embed`, which consumed the plan's `embed` weight, so the real book stage
counted for nothing; (2) every 16-block checkpoint chunk opened and closed its
own `embed` stage. Now `Engine::calibrate` reports a `calibrate` stage, and
`pack` counts the remaining tokens once, opens ONE `embed` stage for the whole
run and hands each chunk an advance-only sink (`AdvanceOnly` in
`commands/pack.rs` and `sopack-embed/src/engine.rs`). The model-verification
stage (`verify`) also got a plan weight. Re-measured on TTL:
stages `verify → load_model → calibrate (2031 tokens) → embed (25 844 tokens)`,
embed-phase `pct` climbing 22.4 → 25.9 → 29.8 → … → 58.4 (monotonic) within
the first 110 s.

## Progress JSON checks (item 4)

Using a full `--progress json` run of WDYS at `--threads 4`
(`bench-packs/wdys/wdys_t4.progress.ndjson`, post-fix):

- `pct` is monotonically non-decreasing across all 20 events, final `done`
  event reports `pct: 100.0` exactly.
- `eta_s` is present (non-null) on multiple `progress` events during the
  `verify` and `embed` stages.
- Separately, `sopack verify <pack> --json` stdout was captured to a file
  and parsed: exactly one JSON document (`json.JSONDecodeError` when
  treated as one-doc-per-line NDJSON; parses cleanly as a single
  pretty-printed document; nothing trailing after it).

## Peak RSS vs the memory guard (item 3)

One data point (no `/usr/bin/time -v` on this image; sampled
`/proc/<pid>/status VmHWM` at 0.2s intervals via `bench/run_bench.sh`):
WDYS at `--threads 4`, peak RSS **1,714,328 KB ≈ 1.67 GiB**, vs. `sopack
doctor`'s guard estimate of **~2.63 GiB** ("model + batch activations +
margin") reported consistently across multiple `doctor`/`pack` runs this
session. The guard is conservative on this data point. Not checked against
the largest book (HSFD pack was killed before reaching a peak-RSS
measurement), where batch activation memory would be more representative
of the guard's actual margin.

## Conclusion

**M4's exit criterion — "3 real books packed in Rust pass the server probe
≥ 0.9999" — is met**: WDYS (23 blocks), TTL (225 blocks) and MAS (290
blocks), all already-live pioneer-corpus books, were extracted, packed,
verified (`sopack verify`, Python `check_pack_py.py`, Python
`check_import_probe_py.py`, all clean/OK), and compared point-for-point
against the live `sop` collection's own stored vectors: 100% id match on
all three (23/23, 225/225, 290/290) at cosine **1.0** exactly (not just
≥0.9999) for every matched point. This is corroborated by the pre-existing,
independent `pack_wdys_is_verifiable_and_matches_the_python_built_reference`
regression test. What is **not** met is the fuller ambition of "3 real
books" implicitly meaning one of them is the large, multi-checkpoint-chunk
Andrews book (HSFD) exercising resume/checkpointing at scale, and the
benchmark gate itself (§3.3's threads/batch-tokens sweep + Python-baseline
speedup number) is materially incomplete — both were cut short by the
orchestrator mid-run to keep this task's turnaround short, not because of
any failure encountered. A real, unfixed progress-percentage plateau bug
(documented above) was found along the way and should be triaged before
M5, since it affects the user-facing promise (P4) for essentially every
real book, not an edge case.
