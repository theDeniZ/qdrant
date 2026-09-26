# sopack 1.0.0 — plan

> **Superseded in part (2026-09-26):** `sopack propose` (P5, §3.5) and the offline
> book-code registry (§3.2, M3) were removed before the 1.0.0 tag, by user decision —
> the client never decides book identity; the importer does, from store state. The
> sections below still describe them as planned; the status table at the end is current.

**Status (2026-09-26): M0–M7 implemented; 1.0.0 prepared (version bumped, changelog
written) — the tag and the first real `/2` import are the user's, see
"Implementation status" at the end.** M0 results: [../sopack-rs/spike/README.md](../sopack-rs/spike/README.md). Plan written 2026-09-24. Supersedes the "proposal" status of
[SOPACK-RUST-MIGRATION.md](SOPACK-RUST-MIGRATION.md) (its analysis stays valid and is
referenced below). The store-independence design is taken as-is from
[SOPACK-AUTONOMY.md](SOPACK-AUTONOMY.md) and is not repeated here.

Related: [IMPORT-PIPELINE.md](IMPORT-PIPELINE.md) (R1–R13, failures #1–#15),
[IMPORT-PIPELINE-PLAN.md](IMPORT-PIPELINE-PLAN.md), [../sopack/README.md](../sopack/README.md),
[../Formula/README.md](../Formula/README.md).

---

## 0. What 1.0.0 is

| # | Property (user, 2026-09-24) | Decided | How it is proven |
|---|---|---|---|
| P1 | Not Python | **Rust**, one binary | no Python on the machine that packs |
| P2 | Independent of Qdrant | SOPACK-AUTONOMY.md, implemented | full run with `--network none` |
| P3 | Core-efficient, auto-scales to the CPU | one model copy, ORT threads + rayon, cgroup-aware | benchmark table (§3.3) on 4 and 10+ cores |
| P4 | Reports progress with % for everything | shared progress engine, TTY bar or NDJSON | every command shows `%` + ETA; tested |
| P5 | Easy to drive from agents (Skills) for the metadata work | **`--json` CLI + `sopack propose` + a SKILL.md** | an agent takes a raw EPUB to a verified pack with no human-typed flags |
| P6 | Future-ready | **swappable embedding model** (contract as data) + **GPU / Apple acceleration** | a second contract loads without code changes; CoreML/CUDA self-verify or fall back |

Platforms: **macOS arm64** and **Linux x86_64 + arm64** (the devcontainer, CI, the
server). Intel macOS and Windows are out of scope for 1.0.

### Decisions I made — veto any of them

1. **The importer stays Python.** Only the producer (the CLI) becomes Rust. The
   server keeps reading packs with Python `PackReader`, but reads the **contract
   from the same data file** the Rust binary embeds (§3.6). Nothing else needs a
   Python CLI.
2. **PDF input is not in 1.0.** `pd-books/converted/_pdf2epub.py` stays the
   pre-step. The extractor interface (§3.2) leaves room for a `pdf` kind later.
3. **`book.json` stays `sopack.book/1`.** Every `book.json` already reviewed,
   such as `packs/wdys.book.json`, is valid 1.0 input with no migration.
4. **Pack format goes to `sopack/2`** (store-neutral, fixture probe). The importer
   reads `/1` and `/2`. 1.0 writes only `/2`.
5. **Versioning:** the Rust CLI ships as `0.9.x` pre-releases side by side with
   Python `0.1.x`. **1.0.0 is the cut-over release** (M8), not the first Rust build.
6. **Distribution:** GitHub Releases with prebuilt binaries for the three targets.
   The Homebrew formula installs the macOS binary (no `cargo` build on the user's
   Mac, no venv, no `post_install`). Linux: a one-line install script plus the same
   tarball. This also retires every problem in SOPACK-RUST-MIGRATION.md §1.
7. **Python CLI after 1.0:** deleted from the release path. The stdlib-only
   `format.py` reader and the contract loader remain for the server.

---

## 1. Milestones

Each milestone has an exit criterion. Nothing starts before M0 passes.

| M | Work | Exit criterion |
|---|---|---|
| **M0** | Spike: vector equivalence (go/no-go) | §2 all green |
| **M1** | Contract as data + calibration fixture + `sopack/2`, done **in Python first** | server imports a Python-built `sopack/2` pack; no Qdrant call left in `sopack/` |
| **M2** | Rust core: contract, ids, `book.json`, pack format writer/reader | golden conformance suite passes in both languages |
| **M3** | Rust extractors + chunker + `inspect` + `propose` | Python extract tests ported and green; byte-identical `book.json` on the fixture corpus |
| **M4** | Embedding engine + `pack` + progress + auto-scaling + resume | 3 real books packed in Rust pass the server probe ≥ 0.9999 |
| **M5** | Agent surface: JSON schemas, exit codes, SKILL.md | agent dry run (§3.5) passes end to end |
| **M6** | Acceleration: CoreML (macOS), CUDA (Linux) | each EP either passes calibration or falls back, and records which |
| **M7** | Packaging + release workflow | `brew install` and the Linux install script work on clean runners |
| **M8** | Cut-over → **1.0.0** | one real batch (the next pioneer set, e.g. ATNW) packed and imported with the Rust binary |

M1 is deliberately Python. It makes the **server** ready for store-neutral packs
before any Rust exists. The Rust port then targets a finished format instead of a
moving one.

---

## 2. M0 — the go/no-go spike

Extends SOPACK-RUST-MIGRATION.md §4 step 0 with what the new properties add. Work
in `qdrant/sopack-rs/spike/`. It needs `rustup` in the devcontainer, which is not
installed today. The devcontainer has the cached model and can read Qdrant, so the
spike runs in-session.

| Check | Bar |
|---|---|
| Rust (`ort` + `tokenizers`, mean pooling, L2 norm, `passage: ` prefix) vs stored vectors of 8 `sop` + 8 `bibles` points | cosine ≥ **0.9999** each |
| Token ids vs Python `tokenizers` for de, en, ja, ko, ru texts, and one > 512 tokens | identical |
| **Length-sorted, token-budget batching** (P3) vs one-text-at-a-time | cosine ≥ 0.9999. Padding is masked in mean pooling, so this should hold, but it is the main speed lever and must be measured, not assumed |
| Thread scaling: 1, 2, 4, all cores, same inputs | identical vectors, near-linear throughput |
| CoreML EP (on the Mac) and CUDA EP (if a GPU is at hand): measure only, no gate | recorded. CoreML may run fp16 on the Neural Engine, which is exactly the drift the calibration gate exists for |
| Link mode: `ort` download-binaries vs dynamic ORT | choose one that builds in CI without network at build time |

**Result (2026-09-24): GO.** 281 stored vectors reproduced at min cosine 0.99999999998,
289/289 token sequences identical, batching and thread count vector-neutral (threads
bitwise). Two findings amend §3.3: per-call CPU speed equals Python's (same ORT), and
large batches are *slower* on CPU (batch 1 = 2.73 blocks/s vs 0.99 at batch 32), so
token-budget batching is a GPU lever and the CPU default is a 512-token budget. Open:
CoreML (needs the Mac) and the static link mode (needs a runner with gcc). Details in
[../sopack-rs/spike/README.md](../sopack-rs/spike/README.md).

**Fail:** if the CPU path cannot reach 0.9999, stop. The fallback is
SOPACK-RUST-MIGRATION.md §6 (stay Python, fix the formula), and P3/P4/P5 get
implemented in Python instead. The plan below assumes a pass.

---

## 3. Design per property

### 3.1 P1 — Rust workspace layout

```
qdrant/sopack-rs/
├── Cargo.toml                workspace, one Cargo.lock (pins ort / tokenizers = the vector space)
├── contracts/                shared with the Python server (§3.6)
│   └── e5-large-v1/
│       ├── contract.toml     model, pooling, prefixes, dim, max tokens, profiles, id rules, thresholds
│       └── calibration.json  the fixture (SOPACK-AUTONOMY.md §3.1)
├── crates/
│   ├── sopack-contract/      loads + validates contract.toml; id rules (uuid5 NAMESPACE_DNS); payload schemas
│   ├── sopack-format/        .sopack writer/reader (ZIP: manifest.json, points.jsonl, vectors.f32 LE, titles.json)
│   ├── sopack-book/          book.json model + validate + to_payload
│   ├── sopack-extract/       trait Extractor; epub, markdown, text, sop_json; chunker + damage gates
│   ├── sopack-embed/         model load, tokenise, batch, run, pool, normalise; execution providers; calibration
│   ├── sopack-progress/      the progress engine (§3.4)
│   └── sopack-cli/           clap subcommands, --json, exit codes
└── conformance/              golden files both implementations must reproduce
```

The library crates are what "future-ready" rests on. Another front end (an MCP
server, a GUI) would be one more crate over the same libraries, not a rewrite.

### 3.2 P2 — store independence

Implement SOPACK-AUTONOMY.md as written: the committed calibration fixture replaces
live canaries, the contract loses collection, vector-name and index-type fields, and
the manifest's `target` becomes `{profile, contract}`. What the Rust side adds:

- **No HTTP client dependency** at all. This is enforced: CI fails if `reqwest`,
  `hyper` or `ureq` shows up in `cargo tree` for `sopack-cli`.
- **Model download** is the one network action, and it is not a store call. It
  happens only in `sopack model fetch` (explicit, with progress and sha256
  verification against the contract). `pack` never downloads; if the model is
  missing, it fails with the command to run. Offline machines can use
  `sopack model import <dir>`.
- **Book-code collisions are checked offline** against a committed registry,
  `contracts/<c>/book_codes.json`. It is generated server-side from the title
  table and refreshed by the importer's admin command, the same one that makes the
  fixture. `propose` and `extract` warn on a collision, and the importer still has
  the final say (its identity probe).

### 3.3 P3 — core efficiency and auto-scaling

Today: one Python process, one block per second on 4 cores (the 2486-block Andrews
book takes ~40 min). `--workers` forks full 2.2 GB model copies and hangs on low RAM.

1.0 design, one process and **one model copy**:

```
read/extract (rayon, per book) ─► tokenize (rayon) ─► batcher ─► ORT session ─► pool+norm ─► writer
                                                     sort by token length,        (intra-op threads
                                                     pack to a token budget        = physical cores)
                                                                                   restores original order
```

- **CPU detection:** `std::thread::available_parallelism()` (respects cgroup quotas
  and affinity, so it is correct inside Docker and the devcontainer) → physical-core
  estimate → ORT intra-op threads. Tokenisation and extraction use rayon on the same
  budget. Pipeline stages overlap, so tokenising batch n+1 runs while batch n embeds.
- **Memory guard:** before loading, check available RAM ≥ model + peak batch
  activations. Refuse with a clear message instead of hanging (failure mode of
  today's `--workers`).
- **Token-budget batching:** sorting by length removes most padding work. This is
  the largest single gain on mixed-length corpora, and M0 must prove it
  vector-neutral.
- **Overrides:** `--threads N`, `--batch-tokens N`. `--workers` is removed; there is
  nothing to fork.
- **Resume:** `pack` writes a checkpoint of finished batches next to the temp file.
  Re-running the same command after a kill resumes. Atomic publish (temp path →
  rename) is unchanged.
- **Benchmark gate (M4):** a committed `bench/` run on the Andrews book and a
  200-block book, reporting blocks/s at 1, 2, 4 and all threads, and the speedup vs
  Python 0.1.x. The speedup is measured and written down, not promised in advance.

### 3.4 P4 — progress everywhere

One engine used by every command. Every long action is a **stage** with a known
total, so a percentage always exists:

| Command | Stages (weight by expected cost) | Unit |
|---|---|---|
| `model fetch` | download, verify | bytes |
| `extract` / `propose` | read container, parse spine items, chunk, gate | items → blocks |
| `pack` | preflight + calibration, tokenise, embed, write, finalise | **tokens** (more accurate than blocks, since cost ∝ tokens) |
| `verify` | checksums, stream points, cross-check | bytes |
| `doctor` | each check | checks |

Output modes, chosen automatically and overridable with `--progress`:
- **TTY:** a bar with overall %, current stage, rate, ETA and elapsed time
  (`indicatif`).
- **Non-TTY / logs:** one line every N seconds or every 5 %, with no carriage-return
  noise in CI logs.
- **`--progress json`:** NDJSON on **stderr**, one event per update:
  `{"event":"progress","stage":"embed","done":81234,"total":210000,"unit":"tokens","pct":38.7,"eta_s":412}`
  plus `stage_start` / `stage_end` / `warning` / `done` events. stdout stays
  reserved for the command's result, so agents can read both without parsing a
  mixture.

The event schema is versioned and committed (§3.5). Tests assert that `pct` is
monotonic and ends at 100 for every command.

### 3.5 P5 — agents and the metadata work

The expensive human part today is **metadata**: book code, exact title, author
form, the *work's* year (not the EPUB's, see the year trap), corpus, provenance.
1.0 makes that an agent loop with sopack as the evidence source and the validator.

**`sopack propose <source> --json`** reads the file and returns a metadata *draft*
with candidates and evidence. It never guesses silently (values below are illustrative):

```json
{"source": {...,"sha256": "…"},
 "fields": {
   "title":  {"value": null, "candidates": [
       {"value": "The Atonement; An Examination of a Remedial System…", "from": "title_page", "evidence": "p.1 lines 1-3"},
       {"value": "AERS - The Atonement", "from": "opf:dc:title"}]},
   "year":   {"value": null, "candidates": [
       {"value": 1884, "from": "title_page"}, {"value": 2021, "from": "opf:dc:date", "warning": "digital-edition date?"}]},
   "book_code": {"value": null, "candidates": [{"value": "ATNW", "collision": false}]},
   …},
 "unresolved": ["title", "year", "rights"]}
```

The agent resolves each field, researching where needed. It writes a
**`<source>.meta.toml`**, and `sopack extract --meta` refuses on any required field
that is still missing. The same sidecar supports folder batches:
`sopack extract dir/ --meta-from-sidecars`.

Machine-friendly contract for all commands:
- `--json` on every command. The result goes to stdout, validated against a
  committed JSON Schema (`sopack schema <name>` prints any of them).
- **Stable exit codes** (0 ok, 2 usage, 3 input invalid, 4 needs metadata,
  5 calibration failed, 6 resources, …), plus an error object
  `{code, message, hint, field?}` so an agent can act without reading prose.
- `sopack commands --json` describes subcommands, flags and schemas, so a skill can
  check what the installed version supports.
- No interactive prompts, ever.

**Skill:** `qdrant/plugin/skills/corpus-prep/SKILL.md` (bible-sop plugin, next to
corpus-lookup). Its loop: `propose` → resolve fields with evidence (year trap,
author name forms, archive provenance) → write `meta.toml` → `extract` → `inspect`
review (damage, drops) → `pack` (progress relayed) → `verify`. It **stops before
import** and hands the pack to the user, because an import is a write to the live
store (see memory: writes need explicit confirmation).

**M5 exit test:** an agent is given only
`pd-books/converted/waggoner-jh__the-atonement-…__1884__archive.epub` and produces a
verified pack whose metadata matches the manifest entry in
`_results_pioneers2026.json`, with no human-typed flags.

### 3.6 P6 — future-ready

**Swappable embedding model — the contract is data, not code.**
`contracts/<id>/contract.toml` declares everything that defines a vector space:
model id + ONNX file sha256, tokenizer sha256, pooling, normalisation, passage and
query prefixes, dimension, max tokens, the chunker's word limits (which depend on the
model's window), profiles, id rules, probe thresholds, and the calibration fixture.
- The binary embeds the released contracts (`include_str!`). `--contract <id|path>`
  selects one, and the default is the one the live collections use (`e5-large-v1`).
- The server reads the **same files** (Python `tomllib`), which ends the
  duplication SOPACK-RUST-MIGRATION.md §4.2 worried about.
- A new model means a new contract directory, a new fixture made from the new
  collection, and a full re-embed into a new collection. It needs **no code change**
  in sopack as long as the model fits the "ONNX encoder + pooling" shape. M2 tests
  this with a small second model (e.g. `multilingual-e5-small`) as a test-only
  contract.

**GPU / Apple acceleration — opt in, and self-verifying.**
- Cargo features `coreml` (macOS) and `cuda` (Linux). `--device auto|cpu|coreml|cuda`
  defaults to `cpu` in 1.0. `auto` becomes the default only after it has been proven
  on real packs.
- **Every non-CPU run embeds the calibration fixture first.** Below 0.9999 against the
  fixture: with `--device auto` it falls back to CPU with a warning; with an explicit
  device it refuses. The chosen device and the measured cosines go into the pack
  manifest (R10 provenance). The same gate protects against a future ORT bump.
- CoreML is configured to prefer fp32 compute units. Whether ANE fp16 is acceptable
  is exactly what the gate decides, per machine.

**Also left open on purpose (not built in 1.0):** the `Extractor` trait accepts new
kinds (PDF/OCR); the format reader rejects unknown major versions and ignores
unknown manifest keys; and the library crates allow an MCP front end later without
touching the core.

---

## 4. Conformance — keeping two implementations honest (M2 onward)

`qdrant/sopack-rs/conformance/` holds golden inputs and outputs. CI runs both sides:

- point ids for every id rule (fixed uid → uuid)
- `book.json` from each fixture source (epub, md, txt, sop_json): byte-identical
  across Python 0.1.x and Rust, or a documented intended difference
- a small `.sopack` written by Rust that Python `verify` + `PackReader` accept, and
  the reverse
- calibration: Rust embeddings of the fixture ≥ 0.9999

After M8 the Python CLI side of these tests is frozen as a reference snapshot. The
server-side reader keeps running them.

---

## 5. Packaging and release (M7)

- `release-sopack.yml` becomes a matrix: `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. Each job builds, runs
  the unit + conformance tests, runs `sopack doctor --quick`, and runs a small real
  pack on CPU against the calibration fixture (model cached in CI).
- Artifacts: tarball per target + sha256 + the contracts. `Formula/sopack.rb`
  becomes a binary formula (`url` per arch, `bin.install`, `brew test` = `--version`
  + `doctor --quick`). No `post_install`, no `var/`, which retires the deprecation
  deadline (2027-12-11).
- Model cache: `~/Library/Caches/sopack` (macOS), `$XDG_CACHE_HOME/sopack` (Linux),
  `$SOPACK_CACHE` to override. `doctor` reports the path it actually uses.
- Tag scheme unchanged: `sopack-v<version>`, version from `Cargo.toml`.

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| Rust vectors drift from the collection | M0 gate at 0.9999; calibration check on every run; `Cargo.lock` pins `ort`/`tokenizers`, and bumping them means re-measuring |
| Length-sorted batching changes vectors | measured in M0; if it does, keep order-preserving batching and accept less speedup |
| CoreML fp16 drift | self-verifying device selection; CPU remains the default in 1.0 |
| Extractor parity (years of OCR rules in `chunk.py`, `epub.py`) | port the 36 extract tests first, then the code; byte-identical `book.json` gate |
| Two contract readers diverge | one data file, loaded by both; conformance suite |
| `propose` looks authoritative when it is not | it only returns candidates with evidence and never fills `value` unless every candidate agrees and the source is the title page; `extract` refuses unresolved fields |
| Scope creep (PDF, MCP, Windows) | explicitly out of 1.0; the traits and crates leave room |

## 7. What I need from you later (not blocking the start)

1. **M0 on your Mac:** the CoreML measurement needs Apple hardware. The rest of M0
   runs in the devcontainer (I'd install `rustup` there, since it isn't present
   today).
2. **Book-code registry source:** confirm the server's title table is the
   authority `book_codes.json` should be generated from.
3. **A CUDA machine**, if CUDA should be verified before 1.0 rather than shipped
   untested behind its feature flag.

---

## Implementation status (2026-09-25)

| M | State | Evidence |
|---|---|---|
| M0 | **GO** | spike/README.md — 281 stored vectors at min cosine 0.99999999998 |
| M1 | **done** | Python `sopack` 0.2.0 writes `sopack/2`; contract is data ([contract.toml](../sopack-rs/contracts/e5-large-v1/contract.toml) read by both sides); calibration fixture (16 entries) from the live store; `canaries` removed; importer behind `StoreAdapter` (Qdrant + in-memory), 3-step probe ([SOPACK-2-FORMAT.md](SOPACK-2-FORMAT.md) §4); 101 sopack + 60 app tests |
| M2 | **done** | `sopack-contract`, `sopack-format`: ids 13/13 golden, Rust reads real `/1` (`packs/wdys.sopack`), Python reads Rust `/2`, resume byte-identical; second (e5-small) contract loads with no code change |
| M3 | **done** | `sopack-book`, `sopack-extract` (+ `propose`, `meta.toml`, offline registry `book_codes.json`): 8/8 extract goldens byte-identical to Python (3 real EPUBs) |
| M4 | **done** | `sopack pack` = one model copy, token-budget batching, memory guard, calibration before any book, checkpoint/resume. Exit gate: WDYS 23/23, TTL 225/225, MAS 290/290 point ids = live store, cosine 1.0 ([../sopack-rs/bench/README.md](../sopack-rs/bench/README.md)). Thread/batch sweep and the Python-baseline speedup were **not** completed (run interrupted); `bench/run_bench.sh` reproduces them |
| M5 | **done** | `--json` everywhere, stable exit codes 0–7, 15 committed schemas, `sopack commands --json`; skill [corpus-prep](../plugin/skills/corpus-prep/SKILL.md). **Blind test** (fresh agent, only the skill + the Atonement EPUB): it detected the re-import (`registry:title_match` → AERS), reused the live code, packed 993 points; **993/993 ids equal the live store, min cosine 0.9999999999** |
| M6 | **code done, hardware untested** | `--device auto|cpu|coreml|cuda`, features `coreml`/`cuda` compile; non-CPU devices self-verify on the fixture or fall back. CoreML needs the Mac, CUDA a GPU (§7) |
| M7 | **files done, not yet run on CI** | [release-sopack.yml](../.github/workflows/release-sopack.yml) (3 targets, bundles ORT 1.30.0, tests the staged tarball incl. a real model run), [ci-sopack.yml](../.github/workflows/ci-sopack.yml), binary [Formula](../Formula/sopack.rb) (ORT kept in libexec), [install.sh](../sopack-rs/install.sh) |
| M8 | **1.0.0 prepared, tag pending** | 0.9.0 released (the Formula carries its sha256) and run on the Mac. `BP3.sopack` (0.9.0, re-import) passed the server dry-run through `open`/`contract`/`probe` (worst cosine 1.00000), refused only at `preflight` (points exist — correct). **User decisions 2026-09-26: release 1.0.0 without a real import; `propose` and the offline registry (`book_codes.json`, `--registry`) removed — book identity is the importer's call from store state (preflight same-title check, `allow_same_title`)**; `BSM.sopack` (Miller, *Bible Student's Manual*, 178 points, new code) stays ready for the first one. see CHANGELOG 1.0.0 |

Totals: Rust workspace 318 tests green (7 model-gated tests run by hand, all green), clippy `-D warnings` and fmt clean, no HTTP client in the binary's dependency tree.

**Findings along the way.** (1) The conversion manifest `_results_pioneers2026.json`
does not hold the live codes: 13 of 22 were renamed at import (ATNW→AERS, TATS→BP3, …).
`propose` now matches titles against the registry, so a re-import is detected rather
than duplicated under a new code. (2) The title table (`app/data/sop_books.json`) was
stale. It was regenerated from the live store (codes were only added), and the registry now
carries author and year. The server needs a redeploy to serve the new table. (3) M0's
CPU finding holds: batch 1 / a 512-token budget is fastest on CPU. Python 0.2.0 now
defaults to batch 1.

**M8: what is left for the user** (updated 2026-09-26 — steps 1–2 done for 0.9.0; for 1.0.0: commit + tag `sopack-v1.0.0`; the BSM import is deferred).
1. Cut the first Rust release: set `[workspace.package] version` in `sopack-rs/Cargo.toml`
   (0.9.0 now; `1.0.0` for the cut-over), commit, then tag `sopack-v<version>` and push the tag.
   The first run of release-sopack.yml is the real test of M7. Its brew job is the
   first `brew install` of the binary formula.
2. On the Mac: `brew install theDeniZ/qdrant/sopack && sopack model fetch && sopack doctor`,
   then `sopack calibrate --device coreml` (records whether CoreML passes the gate, §7.1).
3. Import a real pack through the admin UI (dry-run first). `aers.sopack` from the blind test
   is identical to the live points, so it would be a harmless first `sopack/2` import. Any new
   book goes through the corpus-prep skill.
4. Redeploy the server so that `app/` (adapter, `/2` probe) and the refreshed `sop_books.json` go live.
