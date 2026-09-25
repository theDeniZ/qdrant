# sopack — the Mac side of the corpus import pipeline

Turns a book into a `.sopack` that the server can import into Qdrant without ever
loading an embedding model. Design: [../docs/IMPORT-PIPELINE-PLAN.md](../docs/IMPORT-PIPELINE-PLAN.md).
Requirements it satisfies: [../docs/IMPORT-PIPELINE.md](../docs/IMPORT-PIPELINE.md).

```
source (.epub/.md/.txt/sop JSON)
   │  sopack extract        deterministic, stdlib only, no model
   ▼
book.json                   ← you review, diff and keep this
   │  sopack pack           the slow part: embeds every block
   ▼
name.sopack                 ← upload in the admin UI
```

## Install

```bash
# Apple Silicon, macOS 14+ — this repo is the tap (see ../Formula/README.md):
brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant
brew install theDeniZ/qdrant/sopack
# or, from a checkout:
python3.14 -m venv .venv && .venv/bin/pip install -r requirements.lock
.venv/bin/pip install --no-deps .
```

Run `sopack doctor` first on a new machine. It checks the interpreter, that the
installed fastembed matches the contract, that the model is in the cache fastembed
will actually read (`$FASTEMBED_CACHE_PATH`, else `$TMPDIR/fastembed_cache` — set the
variable on a Mac, macOS purges the temp dir), and — by
actually loading the model and embedding a string — that it produces 1024-dim vectors.
That last check costs ~30 s and is the point of the command; `--quick` skips it and
says so rather than reporting a bare pass.

## Commands

```bash
sopack doctor [--quick]

# source -> book.json  (metadata is never guessed from the filename)
sopack extract book.epub --kind epub -o book.json \
    --book-code HSFD --lang en --title "…" --author "…" --year 1873 --corpus pioneers

# a German SoP corpus file brings its own metadata and alignment map
sopack extract /path/generator/data/sop/de/BH.json --kind sop_json -o bh.book.json

sopack inspect book.json          # counts, damage, what was dropped and why

# embeds every block; self-checks against the committed calibration fixture
# FIRST, before any book, and writes a sopack/2 pack
sopack pack *.book.json -o quarter.sopack
sopack verify quarter.sopack      # offline: checksums, ids, counts, probe shape
```

## Things worth knowing before your first real pack

**The year is a trap.** An EPUB's OPF `<dc:date>` is often the *digital edition's*
date, not the work's. The Gutenberg Andrews file says **2022** for an **1873** book.
Pass `--year` explicitly; `extract` prints a loud NOTE whenever it fell back to the
file's own value, because a wrong year propagates into the title table and every
citation made from it.

**`--workers` is not free.** It defaults to single-process deliberately. fastembed's
parallel mode forks worker processes that each load their own ~2.2 GB copy of the
model; on a machine without several spare gigabytes that does not slow down, it
**hangs** and eventually dies in `_queue.Empty`. Opt in only when you know the RAM is
there.

**Run one `pack` at a time.** Each holds a ~2.2 GB model; two at once on a modest
machine gets one of them killed by the OOM killer (`exit 137`). Your *data* is safe
either way — a pack is built at a unique temp path and renamed into place only when
complete, so a failed or concurrent run can neither publish a partial file nor delete
another run's work — but the second process still dies.

**Budget the time.** Single-process on 4 cores, e5-large embeds roughly one block per
second: the 2486-block Andrews EPUB takes ~40 minutes, a 200-block book ~3. Pack a
quarter's worth of books in one run rather than one at a time.

**`--kind sop_json` takes no metadata flags.** It reads `book_code`, `title` and `year`
from the file's own `meta` block, so passing `--year` there is refused rather than
silently ignored.

**`seq` is not `chunk`.** A `para_key` can hold more than one paragraph: in a scan
with an inline citation scheme, cited blocks are keyed from the citation and uncited
ones (headings) from a chapter ordinal, and the two collide. `seq` runs 0,1,2… across
everything sharing the key and is what the point id is built from; `chunk` is the
piece's index within its own paragraph and is what the payload carries. `inspect`
reports how many para_keys are shared and how many blocks are split. (Before 0.1.4 they were one field and such a book would not validate.)

**The id rule is permanent.** A book's `id_rule` fixes how its point ids are derived.
Change it later and the "same" paragraphs get different ids — you get orphaned
duplicates instead of an update. The two sop rules are not interchangeable:
`sop/plain` is what the EGW corpus uses, `sop/seq` (always a `#<seq>` suffix) is what
the pioneer corpus uses. Both were verified against live point ids.

## The probe — why `pack` needs no `--canaries` any more

`sopack` never talks to a store at all (see
[../docs/SOPACK-AUTONOMY.md](../docs/SOPACK-AUTONOMY.md),
[../docs/SOPACK-2-FORMAT.md](../docs/SOPACK-2-FORMAT.md)) — `sopack/tests/test_neutrality.py`
enforces it: no HTTP client, no store URL, no `collection`/`vector_name` in code. The
canary probe's live-Qdrant scroll is replaced by a **committed calibration fixture**,
`contracts/<id>/calibration.json`, shipped next to `contract.toml`.

`pack` loads and sha256-verifies that fixture, embeds every entry with the same model
instance that will embed your books — **before** embedding any book — and refuses to
build (`CalibrationFailed`, exit code 5) if any cosine against the fixture's own stored
vector falls below `contract.toml`'s `[calibration].pack_min_cosine`. A broken
environment (wrong pooling, a bad onnxruntime build, …) then fails in seconds, not
twenty minutes into a real run. The pack's own fresh fixture embeddings become its
probe, written into the pack alongside a `self_check` summary.

The importer repeats an equivalent, model-free comparison offline against its own copy
of the same fixture, plus a second comparison of the fixture against what is actually
stored in the target collection (SOPACK-2-FORMAT.md §4) — so acceptance is still,
transitively, "this pack's vectors match the collection's space", exactly as the old
canary probe proved, just without sopack ever making a network call to get there.

The fixture is regenerated only when the embedding contract changes (a new model, a new
collection) — a maintained, importer-side admin command
(`python -m app.calibration export`, in the server repo's `app/`), not a sopack command,
since building it is the one operation that still needs to read a live store.

## Layout

| File | |
|---|---|
| `contract.py` | **shared with the server.** Loads `contracts/<id>/contract.toml` (`tomllib`) — model, pooling, prefixes, dim, calibration thresholds, profiles (payload schemas, id rules). No collection names, Qdrant vector names or index types — those are the server's `app/store_adapter.py`. Stdlib only. |
| `format.py` | **shared with the server.** `.sopack` reader/writer (writes `sopack/2`, reads `sopack/1` + `sopack/2`). Stdlib only. |
| `book.py` | the `book.json` seam: `Block`, `Book`, `load`, `dump`, `validate`, `to_payload`, `uid` |
| `extract/` | `epub`, `markdown`, `text`, `sop_json` + the chunker |
| `pack.py` · `verify.py` · `doctor.py` · `cli.py` | the commands |
| `requirements.lock` | the pinned closure — 27 packages, resolved not hand-listed |

`contract.py` and `format.py` are imported by the server too, which is why they must
stay stdlib-only: the server must never gain fastembed, numpy or onnxruntime through
them. Only `pack.py` may import an embedding library.

## Tests

```bash
cd qdrant && PYTHONPATH=. python3.14 -m unittest discover -s sopack/tests -p 'test_*.py'
```
