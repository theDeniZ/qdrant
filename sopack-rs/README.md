# sopack

A single Rust binary that turns a book (EPUB, Markdown, plain text, or a
`sop_json` file) into a **`.sopack`** — a portable, verifiable bundle of
embedded paragraphs ready to import into a vector store. No Python, no
Qdrant connection required to build or verify a pack; the only network
action is fetching the embedding model, and only when you ask for it.

```
source (.epub/.md/.txt/sop_json)
   │  sopack extract           deterministic, no model, no network
   ▼
book.json                      ← reviewable, diffable, the thing you check in
   │  sopack pack               the slow step: embeds every block
   ▼
name.sopack                    ← verify, then hand to an importer
```

Design: [`../docs/SOPACK-1.0-PLAN.md`](../docs/SOPACK-1.0-PLAN.md). Format
spec: [`../docs/SOPACK-2-FORMAT.md`](../docs/SOPACK-2-FORMAT.md). Python
reference implementation (being retired): [`../sopack/`](../sopack/).

## Install

**macOS (Apple Silicon, 14+)** — this repository is the Homebrew tap:

```bash
brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant
brew install theDeniZ/qdrant/sopack
```

**Linux (x86_64 or aarch64)**:

```bash
curl -s https://raw.githubusercontent.com/theDeniZ/qdrant/main/sopack-rs/install.sh | sh
# installs to $HOME/.local by default; add $HOME/.local/bin to PATH
```

Both paths install the `sopack` binary next to the ONNX Runtime shared
library it needs (`lib/libonnxruntime.*`) and the embedded contract data
(`contracts/`). Verify:

```bash
sopack --version
sopack doctor            # ~30s first time: loads the model, embeds one string
sopack model fetch       # downloads the ~2.2 GB model (once, cached)
```

### Building from source

```bash
cd qdrant/sopack-rs
cargo build --release -p sopack-cli
./target/release/sopack --version
```

Needs the ONNX Runtime 1.30.0 shared library discoverable at runtime — see
[Devices](#devices--execution-providers) below for the search order, or set
`ORT_DYLIB_PATH` directly. `Cargo.lock` is committed and pins `ort` +
`tokenizers`, the two crates that (together with the ONNX Runtime build they
load) define the vector space; bumping either means re-running
`sopack calibrate` and checking it still passes.

## Quickstart

```bash
# 1. Turn a source into a reviewable book.json.
sopack extract waggoner.epub --book-code ATNW --corpus pioneers --slug the-atonement \
  -o waggoner.book.json

# 2. Look it over.
sopack inspect waggoner.book.json

# 3. Embed it into a pack (the slow step — needs the model, see `sopack model fetch`).
sopack pack waggoner.book.json -o waggoner.sopack

# 4. Verify it offline (no model, no network).
sopack verify waggoner.sopack
```

Every command also takes `--json` for a single machine-readable result
document on stdout (see [Agent / scripting use](#agent--scripting-use)).

## Commands

### `sopack extract <source> [--kind K] -o <book.json>`

Deterministic, no-LLM extraction into a `book.json` you can review and diff.
`--kind` defaults to whatever the file extension implies
(`.epub`/`.md`,`.markdown`/`.json` → `sop_json`/anything else → `text`).

Metadata flags (which ones a given `--kind` accepts varies — `sop_json`
derives most of its own metadata from the file's `meta` block and refuses
`--title`/`--year`/etc as redundant, exit 2):

```
--book-code --lang --title --author --year --corpus --slug --book-pair
--acquired-from --rights --page-kind --id-rule
```

Metadata can also come from a **`<source>.meta.toml` sidecar** (the same
field names as the flags, written by hand or by an agent). Pass `--meta
<file.toml>` to use a specific one; otherwise `<source>.meta.toml` is picked
up automatically if it exists. A value given on the command line always
wins over the sidecar's; a value given in *both* an explicit flag and a
sidecar must agree (a silent pick between two different values is refused,
exit 2).

The book code you give is the code the points get. `sopack` knows nothing
about what is already imported: whether a code is taken, or the work is
already live under another code, is decided by the importer from the
store's state (its `preflight` stage, shown by a dry-run).

`extract` always runs `sopack_book::validate` on the result before writing:
if that validation's *only* problems are required fields with no value at
all, the run refuses with **exit 4** (needs metadata) and lists exactly
which fields; any other validation problem (a bad `--id-rule`, structurally
broken blocks) is **exit 3** (input invalid). Nothing is ever written on
either failure.

**Folder batch mode:**

```bash
sopack extract sources/ --meta-from-sidecars -o books/
```

Extracts every file in `sources/` that has its own `<file>.meta.toml`
sidecar next to it, writing `<stem>.book.json` per source into the output
directory. Per-file failures are collected and reported; the run only fails
outright if *every* file failed.

### `sopack inspect <book.json>`

Counts, damage stats, and codes for a `book.json` — no model needed.

### `sopack pack <book.json...> -o <out.sopack>`

The slow step: loads the embedding model, embeds every block, and writes a
`sopack/2` `.sopack`. Order of operations:

1. Load and validate every `book.json` (all must share one profile; a
   mixed pack is refused).
2. Verify the model is present and matches the contract (sha256 of the
   small files, size of the ~2.2 GB weights) — **exit 6** with the
   `sopack model fetch` command to run, if not.
3. Check available memory against an estimate of what loading the model
   plus one batch needs — **exit 6** if short, rather than hanging.
4. Load the engine for `--device` (self-verifying — see
   [Devices](#devices--execution-providers)).
5. **Calibration self-check**, before any book is embedded: embeds the
   contract's fixture and compares against its stored vectors. Below the
   contract's `pack_min_cosine` → **exit 5**, refusing to write anything.
6. Embed every block (length-sorted, token-budget batched) and stream
   points into the pack, checkpointing periodically (see
   [Resume](#resume) below).
7. Finish: atomic publish (`.sopack.partial/` → the real path).

```bash
sopack pack a.book.json b.book.json -o combined.sopack \
  --device cpu --threads 4 --batch-tokens 512
```

Flags: `--profile`, `--pack-id`, `--id-rule` (all inferred when every book
agrees), `--threads` / `--batch-tokens` (default: all cores / 512 padded
tokens on CPU, 8192 on a GPU/CoreML device — see the M0 spike's
measurements in `spike/README.md` for why CPU's default is *small*),
`--device`, `--model-dir` (override the resolved cache path), `--fresh`
(discard an existing checkpoint instead of resuming it).

### `sopack calibrate`

Embeds the contract's calibration fixture and reports every entry's cosine
against its stored vector, plus min/mean and pass/fail. What CI runs to
prove a machine reproduces the contract's vector space; also what `pack`
runs internally before touching any book. **Exit 5** on fail.

### `sopack verify <pack.sopack>`

Fully offline (no model, no network): checksums, streams every point with
id re-verification, cross-checks per-book counts against the manifest, and
— for `sopack/2` — recomputes the calibration probe against the contract's
fixture. Reads both `sopack/1` (the old, Python-only format) and `sopack/2`
packs. **Exit 0** clean, **exit 3** otherwise, with every problem listed.

### `sopack doctor [--quick]`

Environment check: version, resolved contract, cache directory, model
presence, ONNX Runtime dylib + version, CPU/thread plan, memory vs. the
guard's estimate, temp-directory writability. Every check is independent —
one failing check never stops the rest from running. `--quick` skips the
sha256 verification of the model and treats a missing model or ONNX Runtime
library as a warning, not a failure (so it can run cleanly right after a
fresh install, before `sopack model fetch`); without it, the model is fully
sha256-verified and the ONNX Runtime library must be present. **Exit 0** if
every check passes (warnings don't count against this), **exit 6**
otherwise.

### `sopack model fetch|import|verify|path`

Manages the local model cache — the **only** place `sopack` ever makes a
network call (`pack` itself never downloads anything).

- `sopack model fetch [--model-dir D]` — downloads every file the contract
  names, resuming a partial download and verifying sha256 + size on
  completion.
- `sopack model import <dir> [--model-dir D]` — verifies an existing model
  snapshot (e.g. a fastembed/Hugging Face cache directory) against the
  contract, then hardlinks or copies it into the cache — for offline
  machines.
- `sopack model verify [--model-dir D]` — checks every file's size and
  sha256 without touching the network.
- `sopack model path [--model-dir D]` — prints the resolved model
  directory (downloaded or not).

### `sopack schema [<name>|--list]`

Prints a committed JSON Schema — one per command's `--json` result, plus
`error`, `progress-event`, and `book` (the `book.json` schema).
`--list` enumerates every name.

### `sopack commands --json`

Describes every subcommand, its flags, its result's schema name, and the
exit-code table — generated from the CLI's own command tree, so it cannot
drift from what the binary actually does. What a Skill/agent checks before
assuming a flag exists.

### `sopack contract show|list`

`show` prints the resolved contract's full detail (embedding space, model
files, chunker limits, calibration thresholds, profiles, id rules). `list`
prints the contract ids this binary embeds (`e5-large-v1`, currently the
only one).

## Exit codes

| Code | Name | Meaning |
|---|---|---|
| 0 | ok | success |
| 1 | internal | an unexpected/internal error |
| 2 | usage | bad command-line usage (also clap's own parse-error code) |
| 3 | input_invalid | the given source/file/pack is malformed or unreadable |
| 4 | needs_metadata | required metadata is missing and must be supplied |
| 5 | calibration_failed | the calibration self-check scored below the contract's threshold |
| 6 | resources | a required resource is missing (model, ONNX Runtime, memory, disk) |
| 7 | interrupted | the run was interrupted; a partial/resumable state was left behind |

`sopack commands --json` reports this same table (`exit_codes`), so a
caller never has to hardcode it.

## Agent / scripting use

Every command accepts `--json`: stdout then carries **exactly one JSON
document** — the result, or `{"ok": false, "error": {"code", "exit",
"message", "hint", "field"}}` on failure — validated against the schema
`sopack schema <command-name>` prints. Progress, if any, always goes to
**stderr**, never stdout, in any mode — an agent can read both streams
without untangling a mixture. There are no interactive prompts, ever.

```bash
sopack pack a.book.json -o a.sopack --json 2>progress.log
jq '.points, .calibration_min_cosine' <<<"$(sopack verify a.sopack --json)"
```

## Progress

`--progress auto|tty|plain|json|none` (default `auto`):

- **`tty`** (auto-selected when stderr is a terminal): a live, redrawn bar
  with overall %, stage, rate and ETA.
- **`plain`** (auto-selected otherwise): one throttled line per update, no
  carriage returns — safe for CI logs.
- **`json`**: NDJSON on stderr, one event per state change
  (`stage_start`/`progress`/`stage_end`/`warning`/`done`), schema:
  [`schemas/progress-event.v1.json`](schemas/progress-event.v1.json).
  `pct` is monotonic and the final `done` event always reports exactly 100.
- **`none`**: no progress output at all.

## Model cache

Resolved in this order: `$SOPACK_CACHE`, else `~/Library/Caches/sopack`
(macOS) or `$XDG_CACHE_HOME/sopack`/`~/.cache/sopack` (Linux). Inside:
`models/<repo with "/" -> "--">/<revision>/<files>` — mirroring the
Hugging Face cache's own naming so the two are recognisable side by side.
`--model-dir` on any model-touching command overrides the resolved path
entirely. `sopack doctor` reports the path actually in use.

## Devices / execution providers

`--device cpu` (default) | `coreml` (macOS, needs the `coreml` cargo
feature) | `cuda` (Linux, needs the `cuda` feature) | `auto`. Every non-CPU
run — and `auto`'s own candidate search — self-verifies against the
contract's calibration fixture *before* touching real data: below the
contract's threshold, `auto` falls back to the next candidate (CPU last,
never itself gated) with a warning; an explicit non-CPU device refuses
outright (exit 5). The device actually used, and its measured cosines when
self-verified, are recorded in the pack manifest as provenance.

The ONNX Runtime shared library is resolved in this order: `$ORT_DYLIB_PATH`,
then `<binary's directory>/../lib/`, then the binary's own directory — the
lookup canonicalises the executable path first, so a Homebrew symlink into
`bin/` still finds the real library next to the Cellar binary. The exact
version loaded (not just the `ort` crate version pinned in `Cargo.lock`) is
recorded as manifest provenance and reported by `sopack doctor`.

## Resume

`pack` checkpoints periodically to `<out>.sopack.partial/` (points and
vectors already durably written, plus a small state file recording which
books, contract, and options this run started with). Re-running the exact
same `pack` command after an interruption resumes automatically, picking up
right after the last checkpoint; changing the input books, the contract, or
the embedding options refuses to resume (with the message to add
`--fresh`, which discards the checkpoint and starts over). A completed pack
is byte-identical whether or not it was interrupted and resumed along the
way, given the same `--pack-id`/`--created-by` (or none, on the same
machine within the process time granularity that provenance uses).

## Contracts

An embedding *contract* is data, not code — `contracts/<id>/contract.toml`
plus `calibration.json`, declaring the model, pooling, dimension, chunker
limits, id rules, and calibration thresholds that together define one
vector space. This binary embeds `e5-large-v1` (the space the live `sop`
and `bibles` collections use) and defaults to it; `--contract <path>`
points at any directory with its own `contract.toml` instead — e.g. a
second, not-yet-released model, verified the same way (`sopack calibrate
--contract <path>`) before anything is packed against it.

## Conformance

`conformance/` holds golden inputs/outputs both this Rust implementation
and the Python reference must reproduce identically — extraction goldens,
pack-format cross-checks (a Rust-written pack read by the Python reader and
vice versa), and calibration comparisons. See
[`conformance/README.md`](conformance/README.md).
