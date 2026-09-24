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

# 8 points that are ALREADY in the live collection — the probe's reference
sopack canaries --qdrant http://10.10.10.10:6333 --collection sop -n 8 -o canaries.json

sopack pack *.book.json --canaries canaries.json -o quarter.sopack
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

**The id rule is permanent.** A book's `id_rule` fixes how its point ids are derived.
Change it later and the "same" paragraphs get different ids — you get orphaned
duplicates instead of an update. The two sop rules are not interchangeable:
`sop/plain` is what the EGW corpus uses, `sop/seq` (always a `#<seq>` suffix) is what
the pioneer corpus uses. Both were verified against live point ids.

## The probe — why `canaries.json` is required

The server has no embedding model, so it cannot re-embed anything to check that your
vectors live in the same space as the collection's. Instead **the pack carries the
proof**: `pack` embeds the canaries' text with the same model instance that embedded
your books, and the server cosines those against the canaries' stored vectors.

Below 0.95 the import aborts before a single point is written. There is no flag to skip
it, a pack without a probe is refused, and a pack cannot lower its own threshold — the
floor is a server constant. This is the one guard against the damage that cannot be
seen (a silent pooling change puts every new vector in a different geometry; retrieval
just quietly degrades).

So: refresh `canaries.json` from the collection you are importing into, and build the
pack on the same machine that produced it.

## Layout

| File | |
|---|---|
| `contract.py` | **shared with the server.** Model, vector name/size, pooling, prefix, profiles, payload schemas, id rules. Stdlib only. |
| `format.py` | **shared with the server.** `.sopack` reader/writer. Stdlib only. |
| `book.py` | the `book.json` seam: `Block`, `Book`, `load`, `dump`, `validate`, `to_payload`, `uid` |
| `extract/` | `epub`, `markdown`, `text`, `sop_json` + the chunker |
| `canaries.py` · `pack.py` · `verify.py` · `doctor.py` · `cli.py` | the commands |
| `requirements.lock` | the pinned closure — 27 packages, resolved not hand-listed |

`contract.py` and `format.py` are imported by the server too, which is why they must
stay stdlib-only: the server must never gain fastembed, numpy or onnxruntime through
them. Only `pack.py` may import an embedding library.

## Tests

```bash
cd qdrant && PYTHONPATH=. python3.14 -m unittest discover -s sopack/tests -p 'test_*.py'
```
