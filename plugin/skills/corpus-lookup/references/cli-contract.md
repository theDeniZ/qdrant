# `sopack` CLI contract

Generated from the actual binary (`sopack 1.0.0`, `sopack commands --json`,
`sopack schema <name>`) — not paraphrased. Re-derive this if `sopack
--version` reports a different version than what you last checked here;
`sopack commands --json` is always the live source of truth.

Build if the binary is missing: `~/.cargo/bin/cargo build --release -p
sopack-cli` from `qdrant/sopack-rs/`, binary at
`qdrant/sopack-rs/target/release/sopack`.

## Global flags (every command)

| Flag | Default | Meaning |
|---|---|---|
| `--json` | off | One JSON result document on stdout (or `{"ok":false,"error":{...}}`); progress still goes to stderr |
| `--progress` | `auto` | `auto\|tty\|plain\|json\|none` — `auto` picks a live bar on a terminal, throttled plain lines otherwise; `json` always emits NDJSON on stderr regardless of what stderr is attached to |
| `--contract` | `e5-large-v1` | Embedding contract id (embedded) or a directory with its own `contract.toml` |
| `--quiet` | off | Suppress the human-readable summary line (no effect with `--json`) |

## Exit codes

| Code | Name | Meaning |
|---|---|---|
| 0 | ok | success |
| 1 | internal | unexpected/internal error |
| 2 | usage | bad CLI usage (also clap's own parse-error code) |
| 3 | input_invalid | given source/file/pack is malformed or unreadable |
| 4 | needs_metadata | required metadata missing — `error.field` names it |
| 5 | calibration_failed | calibration self-check scored below the contract's threshold |
| 6 | resources | a required resource is missing (model, ONNX Runtime dylib, memory, disk) |
| 7 | interrupted | run was interrupted; a partial/resumable state was left behind |

On failure, `--json` prints `{"ok": false, "error": {"code", "exit", "message",
"hint", "field"}}` in place of the command's normal result — `code` is the
name above, `exit` the number, `hint` and `field` may be `null`. This is
schema `error`.

## Commands

### `sopack extract <source> --out <book.json> [--meta FILE] [--meta-from-sidecars] [flags…]`

Deterministic, no model. Reads metadata from (in priority order, conflict =
exit 2 `usage`): explicit CLI flags win over the sidecar; the sidecar fills
what flags don't set; a value given in **both** must agree exactly.
`<source>.meta.toml` is auto-picked-up if it exists next to `source`, even
without `--meta`.

**Fields this crate's extractors never derive from the source** (so
`extract --meta` needs them from flags/sidecar or fails exit 4):

| Kind | Always required | Also required when `corpus` is set |
|---|---|---|
| `epub` | `book_code` | `author`, `year` |
| `markdown` / `text` | `book_code`, `lang`, `title` | `author`, `year` |
| `sop_json` | *(none — all fall back to file structure)* | — |

This is a **preflight convenience**, not the final check — `sopack_book`'s
own `validate()` (run before every write) is authoritative and additionally
rejects: an `id_rule` not valid for the book's `profile`, duplicate
`(para_key, seq)` pairs, empty block text, non-dense `seq` per `para_key`.
`validate()`'s own required-field rule: `book_code`/`lang`/`title` always;
`author`/`year` only when `book.corpus` is a genuinely present, non-null key
(i.e. **not** an EGW work).

Metadata flags: `--book-code --lang --title --author --year --corpus --slug
--book-pair --acquired-from --rights --page-kind --id-rule`.
`sop_json` refuses all of these as redundant (exit 2) — its metadata comes
from the file's own `meta` block.

Result schema `extract` (single-file):
`{"out","book_code","lang","id_rule","stats":{"blocks_in","blocks_out",
"dropped","damage","words","dropped_detail"},"year_from_source_note"}`. Batch (`--meta-from-sidecars`, `source` a
directory, `--out` a directory):
`{"written":[<path>,…],"errors":[<string>,…]}` — writes `<stem>.book.json`
per source; the run only fails outright if **every** file failed.

### `sopack inspect <book.json>`

No model. Result schema `inspect`: `{"file","profile","id_rule","blocks",
"stats","paragraphs","collided_para_keys","split_blocks","book_code","lang",
"title"}`. Use this as the review gate before packing: `stats.damage` (0–1,
fraction of input blocks dropped) should be small — the `WDYS` golden runs
0.037 on heavily OCR-damaged 1861 text; a much higher ratio on a cleaner
source is worth a second look at the source file, not something this skill
tunes (no chunker-limit flags are exposed on `extract`). `collided_para_keys`
> 0 or `split_blocks` unexpectedly high are also worth eyeballing before
committing model time to `pack`.

### `sopack pack <book.json…> -o <out.sopack> [--device D] [--threads N] [--batch-tokens N] [--model-dir DIR] [--fresh]`

The slow step — loads the model. Order: validate every book.json (all must
share one `profile`, else refused) → verify model present/matches contract
(else exit 6, with the `sopack model fetch` command to run) → memory guard
(else exit 6) → load engine for `--device` (`cpu` default; `auto` self-
verifies each candidate against the fixture and falls back to `cpu` with a
warning; an explicit non-`cpu` device below the fixture threshold **refuses**,
exit 5) → **calibration self-check** before touching any book (below
`pack_min_cosine` → exit 5, **nothing is written**) → embed + stream points,
checkpointing periodically to `<out>.sopack.partial/` → atomic publish.

**Resume**: interrupted (exit 7) → **re-run the identical command** — same
books, contract, options — and it resumes from the checkpoint. A different
fingerprint (different inputs/contract/options) is not resumed silently.
`--fresh` discards the checkpoint and starts over; only use it if you mean to
discard partial progress.

**Progress** with `--progress json`: NDJSON on stderr, unit `tokens` for the
`embed` stage (more accurate than blocks, cost ∝ tokens) —
`{"v":1,"event":"progress","stage":"embed","done":…,"total":…,"unit":"tokens",
"pct":…,"rate_per_s":…,"eta_s":…}`, plus `stage_start`/`stage_end`/`warning`/
`done` events. `pct` is monotonic; the final `done` event's `pct` is exactly
`100`.

Result schema `pack`: `{"out","pack_id","profile","id_rule","points","books",
"device","calibration_min_cosine","calibration_mean_cosine","resumed"}`. No
sha256 field — compute the pack file's own sha256 yourself
(`sha256sum`/`shasum -a 256`) for the hand-off report; it is exactly the value
the importer's `POST /import/uploads` body wants.

### `sopack calibrate [--device D] [--threads N] [--batch-tokens N] [--model-dir DIR]`

What `pack` runs internally before touching any book; callable standalone.
Result schema `calibrate`: `{"device","n","min_cosine","mean_cosine",
"threshold","pass","per_entry":[[<label>,<cosine>], …]}`. Exit 5 on fail —
`pass` always matches the exit code.

### `sopack verify <pack.sopack>`

Fully offline. Checksums, streams every point re-verifying ids, cross-checks
per-book counts, recomputes the calibration probe against the contract's
fixture (for `sopack/2` packs). Reads both `sopack/1` and `sopack/2`. Result
schema `verify`: `{"pack","clean","errors":[<string>,…]}`. Exit 0 clean,
exit 3 otherwise (**not** 5 — a verify failure is reported as an invalid
pack, distinct from a live calibration failure during `pack`).

### `sopack doctor [--quick] [--model-dir DIR]`

Environment check, every check independent (one failing check never stops
the rest). `--quick` skips model sha256 verification and treats a missing
model/ONNX Runtime dylib as a **warning**, not failure — safe right after
install, before `model fetch`. Without `--quick`, the model is fully sha256-
verified and the dylib must be present. Result schema `doctor`: `{"checks":
[{"name","ok","level":"info"|"warning"|"error","detail"}, …],"ok"}`. Exit 0
if every check passes (warnings don't count against this), exit 6 otherwise.

### `sopack model fetch|import|verify|path [--model-dir DIR]`

The **only** command that makes a network call (`pack` never downloads).
`fetch` downloads (~2.2 GB, resumable, sha256+size verified). `import <dir>`
verifies an existing snapshot (e.g. a fastembed/HF cache dir) then
hardlinks/copies it in — for offline machines. `verify` checks without
touching the network. `path` prints the resolved model directory
(`{"path","exists"}`, schema `model-path`). `fetch`/`import`/`verify` share
schema `model`: `{"action","model_dir","files","bytes"}`.

### `sopack schema [<name>|--list]` / `sopack commands --json` / `sopack contract show|list`

`schema --list` enumerates: `error`, `progress-event`, `extract`,
`inspect`, `pack`, `calibrate`, `verify`, `doctor`, `model`, `model-path`,
`commands`, `contract-show`, `contract-list`, `schema-list`, `book` (the
`book.json` shape written by `extract` / read by `pack`). `commands --json`
is the live description of every subcommand/flag/exit code — check it before
assuming a flag exists on the installed version. `contract show` prints the
resolved contract's full detail (`embedding`, `model`, `chunker`,
`calibration` thresholds, `profiles`: `["bible","sop"]`, `id_rules`:
`["bible/v1","sop/plain","sop/seq"]`); `contract list` prints embedded
contract ids (`e5-large-v1` only, currently).

## `sopack/2` pack and the import hand-off

A finished, verified `.sopack` is **not** imported by this skill — see
`SKILL.md`'s "Stop before import". The admin app's upload API
(`qdrant/docs/IMPORT-API.md`) is what the user runs next: `POST
/import/uploads` wants `{"name","size","sha256"}` of the pack file, then
`PUT .../parts/{n}` slices it up, then `POST .../complete`. The admin UI at
`http://127.0.0.1:8081` (or tunnelled) does all of this from a drag-and-drop
of the `.sopack` file — the "1. Upload a .sopack" panel — followed by
creating an import job (`dry-run` first, then `apply`). Report the pack path
and its sha256 so the user (or the admin UI, which computes it itself on
upload) can cross-check.

**Book identity is the importer's decision, never the CLI's.** `sopack` packs
the `book_code` it is given, unchanged, and knows nothing about what is
already imported. The importer's `preflight` stage decides from the store's
own state: an existing `book_code`+`lang` with a different `slug` is a hard
refusal (a different work claiming a taken code); the same slug is a
re-index, which needs `allow_overwrite`; a **new** code whose title (and
author) is already live under another code is refused unless the job sets
`allow_same_title` (a separate volume or edition). A dry-run shows all
three.
