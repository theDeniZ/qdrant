# Corpus import pipeline — implementation plan

**Status: agreed design, not yet built. 2026-09-22.**

The standing requirement is [IMPORT-PIPELINE.md](IMPORT-PIPELINE.md) (R1–R12 and the
fifteen failures of the 2026-09 pioneer import). This document is the design that
satisfies it, with every open question settled.

---

## 0. Decisions

| # | Question | Decision |
|---|---|---|
| D1 | Where are vectors computed? | **On the Mac.** The server never loads an embedding model for import; it validates and upserts. |
| D2 | How does the file reach the server? | **Browser upload into the existing admin UI**, on the admin port, behind the existing `ADMIN_PASSWORD` Basic auth. No CLI→server transport, no new keys, no new public surface. Public exposure is a later option, not now. |
| D3 | Pack format | **ZIP container**: `manifest.json` + `points.jsonl` + `vectors.f32` (+ `titles.json`). Vectors are raw little-endian **float32**, chosen because it is the cheapest possible thing for the server to handle — no dequantisation, no numpy, zero-copy read. Size is explicitly not a constraint. |
| D4 | Prep input contract | **Two stages**: `sopack extract` (source → reviewable `book.json`) then `sopack pack` (book.json → `.sopack`). |
| D5 | Local LLM in prep | **None.** `extract` is deterministic and stdlib-only. Metadata comes from the source's OPF or is hand-entered in `book.json`. |
| D6 | Safety net | **Qdrant snapshot + per-job id ledger.** Snapshot before any mutation; `undo.jsonl` + created-id list for exact rollback. |
| D7 | Scope, v1 | The `sop` collection: **English works** and **other languages** (de/ja/ko style, including the de↔en `aligned` map). |
| D8 | Bible | The format carries a **`profile`** field from day one. `sop` is implemented in v1; `bible` is fully specified here and implemented in phase 4, reusing the entire transport, job and safety layer. |
| D9 | CLI home | `qdrant/sopack/`, sharing one `contract.py` with the server. Shipped by GitHub Release + `Formula/sopack.rb` in this repo, which doubles as the Homebrew tap (no separate tap repo). |

---

## 1. Shape

```
  ── Mac (local, offline from Qdrant) ─────────────   ── server (small, LAN) ──────────

   source                                              admin UI :8081
   .epub / .md / .txt / sdarm sop JSON                 ┌──────────────────────────┐
        │                                              │ Upload pack   [chunked]  │
        │ sopack extract                               │ Jobs  · logs · rollback  │
        ▼                                              └───────────┬──────────────┘
   book.json          ← reviewed, diffed, committed                │
        │                                                          ▼
        │ sopack pack   (fastembed, ~40 min/book)          import service
        ▼                                                  open → contract → probe
   <name>.sopack  ──── browser upload ──────────────────►  → preflight → snapshot
   manifest + points.jsonl + vectors.f32                   → undo → upsert → indexes
                                                           → titles → verify → report
                                                                      │
                                                                      ▼
                                                                  Qdrant :6333
```

The server's only expensive dependency is `requests`, which it already has. It does not
import fastembed, onnxruntime, numpy or qdrant-client.

---

## 2. Step 0 — the embedding contract — ✅ DONE, measured 2026-09-22

Failure #15 is the only one that destroys data invisibly, and the pipeline's central
fail-safe (R11) is worthless if it asserts the wrong contract. So this was measured
before any code was written, against the live instance at `10.10.10.10:6333`.

**Method.** Scroll points out of the live collections with their stored vectors,
re-embed their own `raw_text`/`text` with `passage: ` using fastembed 0.8.0, cosine the
two. Sampled in three groups, because a difference *between* groups would mean the
collection is split across two vector spaces:

| Group | n | min cosine | avg | max |
|---|---|---|---|---|
| `sop` pioneers (imported 2026-09) | 12 | **1.00000** | 1.00000 | 1.00000 |
| `sop` EGW / en | 12 | **1.00000** | 1.00000 | 1.00000 |
| `sop` EGW / de | 12 | **1.00000** | 1.00000 | 1.00000 |
| `bibles` (7 translations sampled) | 12 | **1.00000** | 1.00000 | 1.00000 |

**Result.** fastembed **0.8.0**, mean pooling, `passage: ` prefix, normalised, is the
contract for *both* collections. There is no split vector space, and the feared live
mismatch **does not exist** — `requirements.txt` pinning `fastembed==0.8.0` on the query
side is correct as it stands. onnxruntime 1.30.0 served the model without hitting the
external-data trap (#14).

The id rules were verified the same way — each reproduced 8/8 live point ids:

| Rule | Formula | Applies to |
|---|---|---|
| `sop/plain` | `uuid5(dns, "<lang>:<book_code>:<para_key>")` | EGW, de and en |
| `sop/seq` | `uuid5(dns, "<lang>:<book_code>:<para_key>#<seq>")` | pioneers — the suffix is **always** present, including on unsplit blocks |
| `bible/v1` | `uuid5(dns, "bible:<bible>:<osis>")` | every verse |

Collection facts as measured: `sop` 917,341 points, payload indexes
`lang · book_code · page · bible_refs`; `bibles` 339,046 points, indexes `bible · osis`.
Both carry one named vector `fast-multilingual-e5-large`, size 1024, Cosine.

All of the above is now frozen in [`sopack/contract.py`](../sopack/contract.py), which
both the CLI and the server import, and is asserted on every pack.

> Note for later: EGW points carry a `bible_refs` payload key that pioneer points do
> not. It is in `contract.PROFILES["sop"].optional`.

---

## 3. Format A — `book.json` (the reviewable intermediate)

Deterministic output of `sopack extract`, and the thing a human reads, diffs and keeps.
Plain JSON, no vectors, safe to commit.

```jsonc
{
  "schema": "sopack.book/1",
  "profile": "sop",
  "source": {
    "file": "ready/andrews__history-of-the-sabbath__1873__gutenberg.epub",
    "sha256": "…",
    "kind": "epub",
    "acquired_from": "gutenberg",
    "rights": "Public domain"          // as found upstream, verbatim; evidence only
  },
  "book": {
    "book_code": "HSFD",
    "lang": "en",
    "book_pair": "HSFD",               // "<de_code>/<en_code>" where a pair exists
    "title": "History of the Sabbath and First Day of the Week",
    "author": "J. N. Andrews",
    "year": 1873,
    "slug": "andrews-history-of-the-sabbath",
    "corpus": "pioneers",              // omitted for EGW — absence is what makes the
                                       // must_not filter work without a backfill
    "page_kind": "print",
    "description": "…"                 // optional, hand-written, merged additively
  },
  "id_rule": "uuid5(dns, '<lang>:<book_code>:<para_key>[#<seq>]')",
  "alignment": {                       // optional, de/en only
    "en_code": "HSFD",
    "en_reverse": {"12.3": ["14.2"]}
  },
  "stats": {"blocks_in": 4210, "blocks_out": 4102, "dropped": 108,
            "damage": 0.011, "words": 812443},
  "blocks": [
    {"para_key": "12.3", "page": 12, "para": 3, "seq": 0, "chunk": 0, "chunks": 1,
     "text": "…", "words": 214}
  ]
}
```

Rules:

- `extract` never invents metadata. What it cannot read from the source is left `null`
  and `pack` refuses to proceed until it is filled in (fixes #9 — a filename-derived
  author shifting silently).
- Chunking of over-long blocks happens here, visibly, with `chunk`/`chunks` recorded —
  not inside the embedding step.
- `seq` and `chunk` are different numbers (0.1.4). `seq` disambiguates every block
  sharing a `para_key` and is what `sop/seq` hashes into the point id; `chunk` is the
  piece's index within *its own* paragraph and is what the payload carries. They differ
  only when two source paragraphs key the same — which a scan with an inline citation
  scheme does routinely, since its uncited blocks (headings) are keyed by chapter
  ordinal and collide with the citation keys. A `book.json` written before 0.1.4 has no
  `chunk`; it is read back as `chunk = seq`, which is exact, because such a file could
  not contain a collision.
- Dropped blocks are *counted and listed* in a sidecar report, never silently discarded
  (R8).
- Re-extracting a book with a changed `id_rule` is refused; the rule is fixed at first
  import, because changing it orphans every point already written.

Accepted `--kind`: `epub`, `markdown`, `text`, `sop-json` (the
`generator/data/sop/<lang>/<CODE>.json` shape, which already carries `en_reverse`).

---

## 4. Format B — `.sopack` (the upload artifact)

A ZIP. Per-entry CRC32 gives integrity for free; entries are read as streams, so server
RAM is bounded by one batch regardless of file size.

```
<name>.sopack
├── manifest.json      contract, counts, checksums, canary probe, title fragment ref
├── points.jsonl       one line per point, in vector order
├── vectors.f32        N × dim × 4 bytes, little-endian float32, same order
└── titles.json        additive fragment for the book-title table (sop profile)
```

### manifest.json

```jsonc
{
  "schema": "sopack/1",
  "profile": "sop",                      // "sop" | "bible"
  "pack_id": "pioneers-2026-09-22-a1b2",
  "created_at": "2026-09-22T10:00:00Z",
  "created_by": "sopack 1.0.0 on macOS 15.4 arm64",

  "target": {
    "collection": "sop",
    "vector_name": "fast-multilingual-e5-large",
    "vector_size": 1024,
    "distance": "Cosine"
  },
  "embedding": {                          // asserted against contract.py, exactly
    "model": "intfloat/multilingual-e5-large",
    "library": "fastembed",
    "library_version": "0.8.0",
    "pooling": "mean",                    // from Step 0, never assumed
    "normalized": true,
    "passage_prefix": "passage: "
  },

  "counts": {"points": 60412, "books": 51, "dim": 1024},
  "sha256": {"points.jsonl": "…", "vectors.f32": "…", "titles.json": "…"},

  "books": [
    {"book_code": "HSFD", "lang": "en", "points": 4102,
     "first_id": "…", "title": "History of the Sabbath…", "author": "J. N. Andrews",
     "year": 1873, "corpus": "pioneers", "id_rule": "…", "book_sha256": "…"}
  ],

  "probe": {                              // R11 — see §4.4
    "canaries": [
      {"id": "0b7f…", "collection": "sop",
       "vector_offset": 0, "cosine_expected_min": 0.95}
    ],
    "vectors": "probe.f32"
  }
}
```

### points.jsonl

One JSON object per line, in the same order as `vectors.f32`. The existing shape, kept
deliberately:

```json
{"uid": "en:HSFD:12.3#0", "id": "0b7f…-uuid", "payload": { … }}
```

`id` is precomputed by the CLI. The server recomputes `uuid5(NAMESPACE_DNS, uid)` and
refuses the pack if any line disagrees — a two-microsecond integrity check that catches
a mangled or hand-edited pack.

### Writing is atomic

A pack is built at a **unique** temp path (`.<name>.<rand>.part`, with its vector
spool likewise) and `os.replace`d into the destination only once complete. Two
consequences, both load-bearing:

- a reader can never observe a half-written `.sopack` at the destination;
- a writer that fails deletes **its own** file, not whatever now sits at the shared
  destination.

This is a regression guard, not theory. Deriving the temp names from the output path
meant every writer aiming at one destination shared them, so a second run — or a dying
earlier one, whose cleanup unlinked them — destroyed a live run's work. It cost a real
40-minute embed: all 2486 blocks of the Andrews EPUB completed, then the writer died on
a missing spool file. Covered by `sopack/tests/test_format.py::TestConcurrentWriters`.

(Concurrency is safe for *data*; it is still fatal for *memory* — two packs mean two
2.2 GB models and the OOM killer takes one. Run one at a time.)

### vectors.f32

Raw bytes, no header, no framing. Point *i*'s vector is at byte offset
`i * dim * 4`. The server reads it with stdlib `array.array('f')`:

```python
buf = array.array('f')
buf.frombytes(stream.read(BATCH * dim * 4))   # C-speed, no copy per float
vectors = [buf[k*dim:(k+1)*dim].tolist() for k in range(n)]
```

**Why float32 raw, given the answer was "optimal for the server":**

| | RAM per 128-point batch | Server CPU |
|---|---|---|
| float32 raw (chosen) | 0.5 MB read + ~5 MB JSON body | `frombytes` + `json.dumps` ≈ 30 s for a 60 k-point corpus |
| float16 raw | 0.25 MB | + a dequantisation pass over 61 M floats |
| JSON arrays inline | same after parse | + parse 61 M decimal floats — the dominant cost, minutes |

Raw float32 is the only option with **no** conversion step: the bytes come off disk and
go into a list. Qdrant's REST API needs JSON on the wire either way, so `json.dumps` is
an unavoidable floor, and 30 s of CPU spread over a multi-minute import is nothing. The
file is roughly 4 KB/point — ~250 MB for the whole pioneer corpus, ~10 MB for one
ordinary book. That was explicitly accepted.

### 4.4 The canary probe — how R11 survives the split

R11 says: re-embed something already indexed and compare with its stored vector before
writing. The server cannot do that here — it has no model. So the **pack carries the
proof**:

1. `sopack pack` picks N points that are **already in the live collection** (their ids
   and `raw_text` come from a small `canaries.json` the CLI fetched once, or from a
   `sopack canaries` refresh against Qdrant on the Mac).
2. It embeds their text with the same model instance that embedded the book, and stores
   those vectors in `probe.f32`.
3. The server fetches those same ids from Qdrant **with vectors**, and cosines them
   against the probe. Below 0.95 → abort before a single point is written.

This verifies the pack's embedding space against the live collection's embedding space,
with no model on the server, and catches exactly failure #15: a Mac whose fastembed
silently changed pooling produces a pack that cannot be imported.

The probe is **not skippable**. There is no flag.

### 4.5 Profiles

A profile fixes the payload schema, the id rule, the required payload indexes and the
post-import verification. Everything else in the pipeline is profile-agnostic.

| | `sop` (v1) | `bible` (phase 4) |
|---|---|---|
| Collection | `sop` | `bibles` |
| Id rule | `uuid5(dns, "<lang>:<book_code>:<para_key>[#<seq>]")` | `uuid5(dns, "bible:<bible>:<osis>")` |
| Required payload | `lang, book_code, book_pair, page, para, para_key, raw_text, aligned` | `bible, osis, text` |
| Optional payload | `corpus, author, title, year, slug, page_kind, chunk, chunks` | `canonical_osis, versification_offset` |
| Payload indexes | `lang` kw · `book_code` kw · `page` int | `bible` kw · `osis` kw |
| Identity unit | book_code (+lang) | bible (translation) |
| Collision rule | a `book_code` in use by a different `slug` is a collision | a `bible` name in use is a **re-import**, not a collision |
| Title table | `sop_books.json`, additive merge | none |
| Verify | per-book point count + title entry + a live retrieval hit | per-translation verse count + `bible_lookup` of a known verse |

The unit of a Bible import is a whole translation, and re-importing one is the normal
case (a corrected edition), not the exception — so the `bible` profile turns on the
overwrite path that `sop` keeps off by default. Extract sources for `bible`:
`generator/data/bibles/<name>.json`. Everything downstream — container, probe,
snapshot, undo ledger, job UI, rollback — is shared, which is the point of doing this
now rather than later.

---

## 5. The Mac CLI — `sopack`

```bash
sopack extract ready/andrews-history.epub --kind epub -o books/hsfd.book.json
#   → book.json + hsfd.extract-report.md; refuses on missing required metadata

sopack inspect books/hsfd.book.json           # counts, damage, dropped blocks, codes
sopack canaries --qdrant http://10.10.10.10:6333 -o canaries.json   # once, read-only

sopack pack books/*.book.json --canaries canaries.json -o pioneers.sopack
#   → embeds (the slow part), writes the zip, prints the manifest summary

sopack verify pioneers.sopack                 # checksums, schema, id rule, probe shape
```

- `extract`, `inspect` and `verify` are **stdlib only** — they run in the devcontainer,
  in CI, anywhere. Only `pack` needs fastembed (R12/#1).
- `pack` refuses to start unless it can import everything it will eventually need,
  up front — including anything currently deferred into a function body (R7/#2/#3).
- `pack` asserts the produced dimension equals `contract.VECTOR_SIZE` and that the
  model id, library version and pooling match `contract.py`. A mismatch is an error,
  never a warning.
- No `nproc`, no GNU awk, no bash 4 (R12/#4/#5/#6). Worker count comes from
  `os.cpu_count()`.
- `contract.py` is **one file, imported by both the CLI and the server**, holding the
  model, vector name, size, prefix, pooling, payload schemas per profile and the id
  rules. Client/server drift becomes impossible by construction.

### Distribution

- Source lives at `qdrant/sopack/`, versioned with the server.
- A GitHub Release from the `qdrant` repo publishes `sopack-<version>.tar.gz`.
- The `qdrant` repo is itself the tap: `Formula/sopack.rb` (`depends_on "python@3.14"`,
  arm64 + macOS 14 — the pinned onnxruntime has no other macOS wheel) installs into a
  `libexec` virtualenv from `requirements.lock` (fastembed, onnxruntime, tokenizers,
  huggingface-hub, numpy; hash pins still to add). That lockfile is R7.
- `brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant` once, then
  `brew install theDeniZ/qdrant/sopack`; `brew upgrade` for a new release. The release
  workflow `brew install`s every build on a macOS runner before publishing it, then
  commits the new `url`/`sha256` to the formula. See `Formula/README.md`.
- The e5-large model cache is fastembed's own: `$FASTEMBED_CACHE_PATH`, else
  `$TMPDIR/fastembed_cache` (the formula's caveats recommend setting the variable, as
  macOS purges the temp dir). `sopack doctor` checks that exact directory and that the
  model is healthy — including the onnxruntime ≥1.23 external-data trap (#14).

---

## 6. The server — import service inside the admin app

No new port, no new auth. `app/admin.py` gains the routes; they sit behind the same
`_authorized()` Basic check and the same `_same_origin()` CSRF guard.

### 6.1 Upload (chunked, because the file is large by design)

| Route | Does |
|---|---|
| `POST /import/uploads` | `{name, size, sha256}` → upload id, chunk size (8 MiB) |
| `PUT /import/uploads/<id>/parts/<n>` | raw bytes of one part, appended to the staging file |
| `GET /import/uploads/<id>` | which parts arrived — makes the upload resumable |
| `POST /import/uploads/<id>/complete` | verifies size + sha256, moves to `/data/packs/<id>.sopack` |
| `DELETE /import/uploads/<id>` | discard |

Browser side is ~60 lines of JS: `File.slice()`, sequential PUTs with per-part retry, a
progress bar. **No single request is long**, so the timeout problem disappears
structurally rather than by raising a limit.

### 6.2 Jobs

`POST /import/jobs {pack_id, mode}` where mode is `dry-run` or `apply` → job id.
The job runs in one worker thread under a **global import lock** — never two at once.
State is on disk so it survives a container restart:

```
/data/jobs/<job_id>/
  job.json          status, stage, counts, snapshot name, operator, timings
  log.ndjson        one line per event — machine-readable (R10)
  undo.jsonl        prior payload+vector of every point this job overwrote
  created_ids.txt   ids this job created (exact rollback set)
  sop_books.before.json
  report.md         human summary, downloadable
```

`GET /import/jobs` / `GET /import/jobs/<id>` for the UI to poll. A job found `running`
at boot is marked `interrupted` and can be resumed — upserts are idempotent by id, so
resume is just "continue from the last acknowledged batch" (R5).

### 6.3 Stages

Each stage asserts its own output and refuses to hand a half-result to the next (R8).

| # | Stage | Fails the job when |
|---|---|---|
| 1 | `open` | zip/CRC/schema/checksum mismatch; unknown `schema` or `profile` |
| 2 | `contract` | manifest embedding block ≠ `contract.py`; collection's vector name/size/distance ≠ target |
| 3 | `probe` | any canary cosine < 0.95, or a canary id is missing from the collection |
| 4 | `preflight` | `book_code` taken by a different slug (collision); overwrite detected without `allow_overwrite` |
| 5 | `snapshot` | Qdrant snapshot not created (disk, permissions) — **nothing is written if this fails** |
| 6 | `undo` | prior state of an overwritten id could not be captured |
| 7 | `upsert` | a batch fails after 5 retries with backoff |
| 8 | `indexes` | a required payload index is absent and cannot be created |
| 9 | `titles` | the merge would shrink or remove any language table (R6) |
| 10 | `verify` | per-book count ≠ manifest; a book has no title entry; a live search cannot reach the book |
| 11 | `report` | — |

Stage 4 distinguishes **re-index** from **collision** on `(book_code, slug)` identity,
not on code presence (fixes #10): re-importing the same work is recognised and, for the
`sop` profile in v1, still refused unless the job explicitly asks for it; a *different*
work claiming a taken code is always a hard stop. With the `book_code` payload index in
place the check is a `facet` call, not the 30-second full scroll the one-off needed.

Stage 10 is what makes failure #12 impossible: an imported book that `sop_list_books`
cannot see and `sop_lookup` cannot reach is a **failed** import, not a quiet success.

### 6.4 `dry-run` exercises the real write path (R4)

Stages 1–4 run against the live collection for real. Stages 6–10 then run against a
scratch collection `<collection>__dryrun`, created with the identical vector config and
dropped at the end. Snapshot is skipped (nothing is being mutated). A clean dry-run
therefore means the embedding space is right, the codes are free, and the points
actually upsert and are actually retrievable — which the old `--dry-run` never told you.

### 6.5 Resource budget

| | |
|---|---|
| RAM, steady | ~15 MB: one 128-point batch of vectors (0.5 MB), its JSON body (~5 MB transient), a streamed JSONL line |
| RAM, peak | bounded by batch size, independent of pack size |
| CPU | `array.frombytes` + `json.dumps` — ~30 s total for 60 k points |
| Disk | the pack in `/data/packs` + the job dir; both prunable from the UI |
| New dependencies | **none** — `requests` and the stdlib |

The MCP query path keeps its ~2.5 GB embedder untouched; the import thread adds
megabytes, not gigabytes.

### 6.6 The volume problem (must be fixed in phase 1)

`app/data/sop_books.json` is **inside the image** (`COPY app ./app`), while the only
persistent storage is the `/data` volume. A server-side title merge written there is
silently lost on the next `docker compose up --build` — a fresh instance of failure #12,
built into the deployment.

Fix: `SOP_BOOKS_JSON=/data/sop_books.json` (the env var `sop_tools._book_titles()`
already checks first), seeded from the packaged copy on first boot if absent. The
volume copy becomes authoritative; the packaged one is a seed, and
`export_book_titles.py` is a bootstrap tool that is never run against production.

`_book_titles()` caches into a module global, so the `titles` stage must clear
`sop_tools._titles` when it finishes — otherwise the new books stay invisible until the
container restarts.

---

## 7. Rollback and restore

Two layers, because they fail differently.

**Fine (the usual case).** Every id is deterministic and the job recorded exactly which
ids it created and what it overwrote. `POST /import/jobs/<id>/rollback`:
delete `created_ids.txt`, re-upsert `undo.jsonl`, restore `sop_books.before.json`,
clear the title cache. Seconds, exact, no collateral damage to anything written since.

**Coarse.** The Qdrant snapshot from stage 5. `POST /import/jobs/<id>/restore-snapshot`
puts the whole collection back — losing anything else written after it, which is why it
is the second choice and the UI says so.

R3 is explicit: **restore is exercised before the pipeline is declared done.** The
acceptance run (§9) ends by rolling back a real import on the scratch collection and
asserting the collection is byte-identical to its pre-import state, and by restoring one
snapshot for real. An untested restore is not a restore.

Retention: keep the last N snapshots and job dirs (default 5 / 20), prune from the UI,
report disk used. Snapshots live on the **Qdrant node's** disk — confirm headroom there
before the first run (~900 k points is several GB); a snapshot that cannot be created
aborts the job, which is the fail-safe working as designed.

---

## 8. Requirements coverage

| Req | Satisfied by |
|---|---|
| R1 one durable path | §6, versioned with the repo; a new book set is a new `.sopack`, never new code |
| R2 backup before mutation | stage 5, before any write; job aborts if it fails |
| R3 exercised restore | §7, part of the acceptance test |
| R4 real dry-run | §6.4, scratch collection, real embed/upsert/retrieve |
| R5 idempotent, resumable | deterministic ids; resumable upload; resumable job; `(book_code, slug)` identity |
| R6 additive, never destructive | stage 9 shrink guard; full rebuild is not reachable from the server at all |
| R7 pinned + verified deps | `requirements.lock` in `sopack/`; `sopack doctor`; `pack` imports everything up front |
| R8 no silent partial success | every stage asserts its own output; stage 10 proves retrievability |
| R9 self-contained | pack is self-describing; `generator/data/sop` is an optional `extract` *source*, never required |
| R10 run log and provenance | `job.json` + `log.ndjson` + `report.md`, operator and timings |
| R11 assert the embedding space | §4.4 canary probe, not skippable, plus §2 done first |
| R12 portable | server has no native deps; CLI is stdlib except `pack`; no `nproc`/gawk/bash-4 |

Against the fifteen failures: #1/#2/#3/#7/#14 → `doctor` + lockfile + up-front imports;
#4/#5/#6 → no shell tooling; #8 → `extract` reports what it drops and why; #9 → no
metadata is ever derived from a filename; #10 → `(book_code, slug)` identity; #11/#12/#13
→ additive merge + shrink guard + descriptions living in `book.json` + §6.6; #15 → the
canary probe.

---

## 9. Work breakdown and status

**Phase 0 — ground truth. ✅ DONE.** §2. Measured, not assumed; the contract is frozen
in `sopack/contract.py` and the feared live mismatch does not exist.

**Phase 1 — format + CLI. ✅ BUILT.**
`sopack/{contract,format,book,extract/*,canaries,pack,verify,doctor,cli}.py`,
`sopack/schemas/book.schema.json`, `sopack/pyproject.toml`, `sopack/requirements.lock`
(the full resolved 27-package closure — fastembed alone has 11 direct dependencies).
§6.6 fixed in `app/seed.py` + `Dockerfile` + `app/server.py`.

**Phase 2 — server. ✅ BUILT.**
`app/import_service.py` (the 11 stages), `app/jobs.py` (store, global import lock,
boot recovery), `app/snapshots.py` (R2/R3), `app/uploads.py` (chunked, resumable),
routes + `/import` page in `app/admin.py`, `app/static/import.js`. The shrink guard is
ported into the `titles` stage; `scripts/merge_corpus_titles.py` stays as the offline
equivalent.

**Tests: 108 across 8 files, all passing.**

| File | Tests | Covers |
|---|---:|---|
| `sopack/tests/test_book.py` | 29 | the book.json seam, payload/uid agreement with `contract` |
| `sopack/tests/test_extract.py` | 35 | chunking, damage gate, ref lifting, boilerplate, alignment |
| `sopack/tests/test_pack.py` | 7 | round trip, dim mismatch, wrong fastembed, probe, titles |
| `sopack/tests/test_verify.py` | 4 | offline integrity |
| `app/tests/test_jobs.py` | 10 | atomic save, log seq, import lock, boot recovery |
| `app/tests/test_import_service.py` | 16 | probe rejection, snapshot-failure abort, shrink guard, collision vs re-index, rollback, dry-run, cancel, resume |
| `app/tests/test_admin_import.py` | 7 | auth, CSRF, chunked+resumed upload, checksum, part gap, confirm |
| `app/tests/test_seed.py` | 5 | seeding, no-overwrite, parent dir, invalid JSON, data dirs |

**Phase 3 — acceptance. ▶ IN PROGRESS.** §10.

Proven end to end, with real data and no mocks:

| Check | Result |
|---|---|
| `extract` a real EPUB (`andrews…1873…`) | 3531 → 2486 blocks, damage 0.02 %, `validate()` clean |
| `extract` a real German `sop_json` (`de/BH.json`) | 197 blocks, alignment carried, `validate()` clean |
| `pack` that German book with live canaries | 197 points in 3m01s, `vectors.f32` = 197×1024×4 exactly |
| `verify` the pack offline | clean |
| **probe the real pack against live `sop`** | **8 canaries, worst cosine 1.00000** |
| alignment direction in the packed payload | `de:BH:5.1 → aligned ['7.1']` — **correct**, unlike the live collection ([FINDING-de-alignment.md](FINDING-de-alignment.md)) |
| adversarial: wrong-space vectors | rejected, cosine −0.015 < 0.95, before any write |
| adversarial: pack with no probe | refused — "not skippable, no flag" |
| adversarial: pack lowering its own threshold to 0 | floor still enforced (it is a server constant) |

**Still not done: nothing has been imported into any collection, live or scratch.**
That is the remaining gate, and the one-offs stay until it passes.

**Phase 4 — `bible` profile. ◻ NOT STARTED.** The format, payload schema, id rule,
indexes and verification rules are specified and frozen in `contract.py`; what is
missing is `extract --kind bible_json` from `generator/data/bibles/<name>.json` and the
re-import path. No transport, job, snapshot or UI work — that is the dividend of D8.

**Phase 5 — tap. ◻ PARTIAL.** `Formula/sopack.rb` (this repo is the tap),
`Formula/README.md` and `.github/workflows/release-sopack.yml` exist. The package now
installs correctly (`pyproject.toml` used to ship only a top-level `extract` package and
no `sopack`) and the lockfile resolves for macOS 14 arm64. **No release has been cut
yet**: the workflow's macOS `brew install` job is the first real validation of the
formula. Until then, install from a checkout.

---

## 10. Acceptance test

The requirement doc's migration step 3, made concrete:

1. `sopack extract` all 51 pioneer works from `pd-books/`, `sopack pack` them.
2. Import the pack in `dry-run` mode → passes, scratch collection dropped.
3. Import it for real into a scratch `sop__accept` collection seeded from a snapshot of
   live `sop`.
4. Diff `sop__accept` against live `sop`: point counts per `book_code`, a sample of
   payloads, and cosine of a sample of vectors against the live ones (must be ~1.0 —
   this is the probe proving itself end to end).
5. Roll back the job. Assert `sop__accept` is identical to its pre-import state.
6. Restore one snapshot for real and assert it lands (R3).
7. Import one German book with an `alignment` block and assert `sop_parallel` resolves
   a de↔en pair across it.

Then, and only then, delete `pd-books/qdrant/{build_pioneers_corpus,index_pioneers_qdrant}.py`
and `run_import.sh`, and replace `pd-books/qdrant/README.md` with a pointer here.

---

## 11. Left open deliberately

- **Public exposure of the import endpoint** (D2). The routes are written so the admin
  app can later be split onto its own port or put behind Traefik with a scoped key,
  but nothing is exposed now.
- **`sop` re-import / overwrite** is built (the undo ledger needs it) but gated off in
  v1; the `bible` profile turns it on in phase 4, which is also where it gets proven.
- **Local LLM assistance** in `extract` (D5: none). If it ever comes back, it belongs on
  `book.json` as flagged suggestions, never in the vector path.
- **Snapshot headroom on the Qdrant node** — an operational precondition to confirm
  before the first real run.

## 12. Related

- [IMPORT-PIPELINE.md](IMPORT-PIPELINE.md) — the requirement this implements (R1–R12)
- [../references/ACQUISITION-BRIEF.md](../references/ACQUISITION-BRIEF.md) — where books come from
- [../scripts/merge_corpus_titles.py](../scripts/merge_corpus_titles.py) — the additive merge to port into stage 9
- [../scripts/export_book_titles.py](../scripts/export_book_titles.py) — bootstrap only; note its rebuild-from-scratch semantics (#11)
- `../../generator/src/sdarm/tools/build_sop_vector_index.py`, `build_bible_vector_index.py` — the id rules and payload schemas both profiles must match
