# Corpus import pipeline — requirements

**Status: agreed requirement, not yet built. 2026-09-21.**

> "I NEVER AGAIN NEED SOME SKETCHY SCRIPTS. What I need is an actual import
> pipeline via a server with fail-safes and backup-restore procedures. Every
> time I tried to import something into Qdrant in past 6 months you gave me a
> NEW SCRIPT always — I hate it. It's the last time we do it that way."

Every corpus import for the last six months has been answered with a fresh
one-off script. Nothing accumulates, each run re-discovers the same failures,
and none of them carries a safety net. This document is the standing
requirement that replaces that pattern, and the starting point for building
the replacement.

**Rule for future work:** a corpus, language or book set that needs to reach
Qdrant is a job for *the pipeline*. Do not open with "here is a script". The
one-offs under `pd-books/` are legacy to be folded in, not a template to copy.
If a stopgap is genuinely unavoidable, say so and get agreement first.

---

## 1. Why: what the 2026-09 pioneer import actually cost

23 books. Every item below was hit for real, in one run, and each one is a
pipeline concern rather than a script concern.

| # | Failure | Class |
|---|---|---|
| 1 | venv copied between machines; interpreter symlink dead, `No module named pip` | environment |
| 2 | `jszip` absent — `build_catalog.mjs` expects it hoisted from a monorepo root that does not travel with a copy of `qdrant/` | dependency |
| 3 | `qdrant_client` absent, and imported **inside** `main()`, so it failed only after the corpus scan had already printed a healthy-looking summary | dependency |
| 4 | `nproc` is GNU-only; silently wrong worker count on macOS | portability |
| 5 | 3-argument `match()` is a gawk extension; macOS ships BSD awk | portability |
| 6 | `pgrep` not installed in the dev container | portability |
| 7 | tesseract PDF output silently produces nothing without `pdf.ttf` **and** `tessdata/configs/` | silent failure |
| 8 | tesseract embeds every 300dpi page image: 111 MB for one 188-page book | hygiene |
| 9 | Two acquisitions had edition-qualified year parts (`1853-2nded1872`) which `parseFilename` would read as the work slug, shifting the author | data integrity |
| 10 | `preflight()` cannot distinguish re-indexing from a genuine code collision; an unscoped second import aborts listing all 49 existing works | ergonomics |
| 11 | `export_book_titles.py` **rebuilds** `app/data/sop_books.json` from scratch. Miss a source and codes vanish — the generator mirror is the only source of de/ja/ko `en_code`, so a rebuild from a `qdrant/`-only checkout destroys every mapping | **data loss** |
| 12 | Without `--merge`, newly indexed books get no title-table entry and `sop_list_books` returns `"titles": []` — exactly how the 2026-08-23 import became invisible | **silent data loss** |
| 13 | Re-running the entry generator reset 23 hand-written descriptions to TODO | data loss |
| 14 | onnxruntime ≥1.23 rejects `multilingual-e5-large`: its 2.2 GB `model.onnx_data` is symlinked into a different HF `blobs/` directory than `model.onnx`, so external data "escapes the model directory". Surfaces only after a 19-minute download | dependency |
| 15 | fastembed 0.8.0 silently switched this model from CLS to **mean pooling**. Embedding new points with different pooling than the existing 899k writes them into another geometric space — no error, just permanently degraded retrieval | **silent corruption** |

Items 11, 12 and 13 destroy work silently. That is the core argument: these
need to be structurally impossible, not remembered.

---

## 2. Requirements

### R1 — Server-side, one durable path
One ingestion service, versioned with the repo, not a script per import. It
accepts a *job* (a corpus JSONL plus metadata) and owns everything from
validation to the title table. A new book set is new **input**, never new code.

### R2 — Backup before any mutation
No write proceeds without a restorable snapshot:
- Qdrant snapshot (`PUT /collections/<c>/snapshots`), id recorded in the run log
- a copy of `app/data/sop_books.json`
- retention and a pruning policy

### R3 — Documented, exercised restore
A `restore` command that has been run successfully at least once and is
covered by a test. An untested restore is not a restore.

### R4 — Dry-run that exercises the real write path
The current `--dry-run` stops before the model loads, so embedding and upsert
are never rehearsed. It must validate against a scratch collection instead, so
a clean dry-run actually means something.

### R5 — Idempotent and resumable
Re-running a completed job is a no-op. An interrupted job resumes. Re-indexing
existing works must be distinguishable from a code collision (fixes #10) —
compare on `(book_code, slug)` identity, not code presence alone.

### R6 — Additive by default, never destructive
Derived tables are updated additively. A step that would remove or shrink any
language table refuses and reports. Full rebuilds are an explicit opt-in flag,
never the default and never implied by the presence of an optional input
(fixes #11, #12). `scripts/merge_corpus_titles.py` is the shape to generalise.

### R7 — Dependencies pinned and verified up front
A lockfile, and a preflight that imports exactly what the run will import —
including deferred imports inside `main()` — so a missing wheel fails in
seconds, not twenty minutes in (fixes #2, #3).

### R8 — No silent partial success
Every stage asserts its own output: OCR produced a text layer, conversion kept
≥95 % of source words, every book has a title-table entry after indexing.
A stage that produces nothing usable fails loudly (fixes #7, #12).

### R9 — Self-contained
Runs from a `qdrant/` checkout alone. Optional inputs such as
`generator/data/sop` may enrich a run but must never be required, and their
absence must never cause data loss.

### R10 — Run log and provenance
Each run appends an auditable record: job id, snapshot id, counts before and
after, per-book outcome, operator, timestamp. `pioneers_report.md` is the
right instinct; make it permanent and machine-readable.

### R11 — Assert the embedding space before writing a single point
The vector geometry is not verifiable by inspection, and a mismatch is
invisible until retrieval quality quietly degrades. Before any upsert, re-embed
a point that is already indexed and compare with its stored vector; abort below
a cosine threshold (0.95). Record the model, the fastembed version and the
observed cosine in the run log. This is the single most important fail-safe:
it is the only one guarding against damage that cannot be seen (fixes #15).

Embedding dependencies are pinned by *behaviour*, not just version: pooling,
prefix (`passage:` / `query:`) and normalisation must match what built the
collection, and the indexer and `app/` query path must be pinned together.
`requirements.txt` currently pins `fastembed==0.8.0` for the service; if the
corpus predates the pooling change, the live query path is already mismatched.

### R12 — Portable or containerised
Prefer a container so #4, #5, #6 and #14 cannot recur. If it must run on a host,
POSIX tools only: no `nproc`, no GNU-only `awk`, no bash-4 syntax.

---

## 3. Shape to aim for

```
qdrant/import/
  service.py        job API: submit, status, cancel
  stages/           validate → snapshot → embedcheck → embed → upsert → titles → verify
  snapshots.py      R2 + R3
  jobs/<id>/        manifest, run log, snapshot id, per-book outcome
```

A job is data:

```jsonc
{
  "job": "pioneers-2026-09",
  "corpus": "pd-books/qdrant/pioneers_corpus.jsonl",
  "collection": "sop",
  "scope": ["the-cross-and-its-shadow", "..."],   // R5
  "titles": {"mode": "additive"},                  // R6
  "on_failure": "restore"                          // R2/R3
}
```

Adding a book set means writing that file. Nothing else.

---

## 4. Migration

1. Keep `run_pioneer_import.sh` **only** until the pipeline lands; it is the
   reference for what the stages must do, not a thing to extend.
2. Port the stage order, `merge_corpus_titles.py` (already R6-shaped) and the
   shrink guard first — they are the data-loss protections.
3. Re-run the 2026-09 pioneer import through the pipeline against a scratch
   collection and diff the result against the live one. That is the
   acceptance test.
4. Delete the one-offs once it passes.

## 5. Related

- `references/ACQUISITION-BRIEF.md` — where the 2026-09 acquisition started
- `pd-books/downloads/pioneers/ACQUISITION-REVIEW.md` — what it produced
- `scripts/merge_corpus_titles.py` — the additive title merge (R6)
- `scripts/export_book_titles.py` — the full rebuild; note its rebuild-from-scratch semantics (#11)
