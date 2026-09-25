# conformance/

Golden inputs and outputs both the Python (`qdrant/sopack/`) and Rust
(`qdrant/sopack-rs/crates/`) implementations must reproduce identically.
Plan: [`../../docs/SOPACK-1.0-PLAN.md`](../../docs/SOPACK-1.0-PLAN.md) §4.

## extract/ (M3)

`extract/manifest.json` lists every fixture the Rust conformance test
checks: 5 small synthetic sources under `extract/fixtures/` (epub, markdown,
text, and two sop_json variants — one plain English, one German with
`en_reverse` alignment) plus 3 real pioneer-corpus EPUBs referenced by path
under `qdrant/pd-books/converted/` (not copied here — different sizes:
6.4 KB, 22 KB, 89 KB). Each entry names the extract `kind` and the metadata
options passed to `extract()`.

`extract/make_extract_goldens.py` is the **committed generator**. It writes
the fixture sources deterministically (fixed content, no timestamps), then
runs the real Python `sopack.extract.extract()` against every manifest
entry (fixtures and real books alike) and writes `extract/goldens/<name
>.book.json` with `sopack.book.dump()`. Regenerate with:

```bash
cd qdrant
PYTHONPATH=. /workspaces/sdarm/.venv/bin/python3.11 sopack-rs/conformance/extract/make_extract_goldens.py
```

The Rust side is
[`crates/sopack-extract/tests/conformance_extract.rs`](../crates/sopack-extract/tests/conformance_extract.rs):
it reads the same manifest, runs `sopack_extract::extract()`, dumps the
resulting `Book` with `sopack_book::dump()`, and asserts the bytes are
**identical** to the golden — schema, key order, indentation, escaping, and
all. A real-book entry whose source file is not present locally is skipped
(not failed), matching the plan's "do not copy huge files into conformance"
rule; the test asserts at least one entry *was* actually checked so an
all-skipped run cannot pass silently.

Run it: `cargo test -p sopack-extract --test conformance_extract -- --nocapture`
(the `--nocapture` line reports how many entries were checked vs. skipped).

### Result (2026-09-24, Rust `ort`/`tokenizers` versions per root `Cargo.toml`)

All **8/8** goldens are byte-identical: `small_epub`, `small_markdown`,
`small_text`, `small_sop_json`, `small_sop_json_de`, `wdys`,
`come_out_of_her`, `seal_of_the_living_god`.

### Intended differences

**None.** Every byte of every golden — including `source.sha256` (from the
real file bytes), `source.file` (the exact relative path string passed to
both sides), and OPF-derived `book.title`/`book.author`/`book.year` scraped
from real archive.org EPUB metadata — matches exactly. If a future contract
or extractor change introduces an unavoidable difference, document it here
with the reason, which golden(s) it affects, and why it cannot be closed,
per the plan's "aim for zero" rule.

### Why byte-identical was achievable

The two riskiest fidelity points going in were:

- **`sopack/extract/chunk.py`'s `SENT_RE`** (`(?<=[.!?;:])["'’]?\s+`) needs a
  lookbehind; **`sopack/extract/epub.py`'s** script/style stripper and
  paragraph-tag matcher (`<(script|style)\b.*?</\1>`,
  `<(p|h[1-6]|li|blockquote)\b[^>]*>(.*?)</\1>`) need backreferences. The
  `regex` crate supports neither. `sopack-extract` uses
  [`fancy-regex`](https://docs.rs/fancy-regex) — a backtracking engine, like
  CPython's `re` — for every ported pattern, so backreference/lookbehind
  matching semantics (including which of two same-named nested tags a
  non-greedy backreferenced match prefers) agree with Python exactly.
- **OPF `dc:*` metadata** (`sopack/extract/epub.py:_opf_metadata`) resolves
  elements by **namespace URI**, not literal `dc:` prefix, via
  `ElementTree.find(".//dc:title", {"dc": "..."})`. `sopack-extract`'s
  `opf_metadata` (in `crates/sopack-extract/src/epub.rs`) does the same
  with `quick-xml`: it tracks `xmlns:*` bindings seen anywhere in the
  document (not fully scope-nested — a simplification that holds for every
  real EPUB in this corpus, which all declare the namespace once on
  `<metadata>`) and resolves `prefix:local` against that map before
  matching `local` against `title`/`creator`/`date`/`rights`.

## ids/ (M2)

Golden `{rule, fields, uid, id}` cases proving the Rust `IdRule` template
engine (`crates/sopack-contract/src/idrule.rs`: `{field}` substitution +
`uuid5(NAMESPACE_DNS, uid)`) agrees with the Python reference
(`sopack/contract.py`'s `_fmt_uid`/`point_id`), not just with itself.

`ids/make_id_goldens.py` is the **committed generator**. It imports
`sopack.contract` directly (the same module `pack.py`/`format.py` use — no
duplicated id logic) and calls `uid_for`/`point_id` for a fixed list of
cases covering every `[ids.rules]` template in
`contracts/e5-large-v1/contract.toml` (`sop/plain`, `sop/seq`, `bible/v1`),
`seq` at `0` and non-zero, and Unicode in `book_code`/`para_key`/`bible`
(German umlauts, Japanese, Korean, Russian Cyrillic). Regenerate with:

```bash
cd qdrant
PYTHONPATH=. /workspaces/sdarm/.venv/bin/python3.11 sopack-rs/conformance/ids/make_id_goldens.py
```

**`seq` is deliberately never *absent* in these cases.** `contract._fmt_uid`
is `template.format(**fields)` with no defaulting of any field, `seq`
included — a template needing a field that is not in `fields` raises. The
"`seq` defaults to `0` when the uid has no `#`" behavior lives one layer up,
in `sopack.format._fields` (mirrored in `sopack-format::idfields::fields_from`),
and is covered by that crate's own unit tests instead — it is a
`points.jsonl`-writing concern, not an id-rule-template one.

The Rust side is
[`crates/sopack-contract/tests/conformance_ids.rs`](../crates/sopack-contract/tests/conformance_ids.rs):
it loads `ids/golden.json`, resolves each case's rule from the embedded
`e5-large-v1` contract, and asserts both the built uid string and the
final `uuid5` id match the Python-generated golden exactly.

Run it: `cargo test -p sopack-contract --test conformance_ids`

### Result (2026-09-24)

**13/13** golden cases match byte-for-byte (uid string and id both).

## packs/ (M2)

Three directions, per SOPACK-1.0-PLAN.md §4 ("a small `.sopack` written by
Rust that Python `verify` + `PackReader` accept, and the reverse"):

### Direction 1 — Rust reads a real `sopack/1` pack

[`crates/sopack-format/tests/conformance_v1_read.rs`](../crates/sopack-format/tests/conformance_v1_read.rs)
opens `qdrant/packs/wdys.sopack` (a real pack from the Python pipeline, 23
points, profile `sop`) with `sopack_format::PackReader`, asserts
`check()` is clean, streams every point through `batches()` with id
verification on, and separately runs the library `verify()` — mirroring
`sopack/1`'s reduced check surface (no `target`/`contract`/`embedding`
cross-check against the store-neutral contract; those blocks use a
different, retired vocabulary this crate no longer models — see
SOPACK-AUTONOMY.md — while the schema-agnostic checks: sha256, byte counts,
profile/id_rule validity, dim, still run in full).

**Result: pass.** `check()` and `verify()` both report zero errors; all 23
points stream and re-verify their ids.

Run it: `cargo test -p sopack-format --test conformance_v1_read`

### Direction 2 — Python reads a Rust-written `sopack/2` pack

`crates/sopack-format/examples/make_v2_conformance_pack.rs` writes
`packs/rust_v2_sample.sopack`: 3 points built from the real, committed
calibration fixture's own "sop"-profile entries (real text, real 1024-d
vectors — reused verbatim rather than freshly embedded, since this proves
the *format*, not a model run) under a synthetic `book_code` `"CONF"`, plus
a probe covering **every** fixture entry (all profiles) with `self_check`
computed by cosining those same reused vectors against themselves (exactly
`1.0`, trivially clearing `pack_min_cosine`).

`packs/check_pack_py.py` is the **committed Python check script**: it opens
`rust_v2_sample.sopack` with the real `sopack.format.PackReader` (`check()`
+ `batches()` with id verification) and `sopack.verify.verify()`.

Regenerate the pack and re-run the check:

```bash
cargo run -p sopack-format --example make_v2_conformance_pack
cd /workspaces/sdarm/qdrant && PYTHONPATH=. /workspaces/sdarm/.venv/bin/python3.11 \
    sopack-rs/conformance/packs/check_pack_py.py
```

**Result (2026-09-24): pass.** `sopack.format.py` already supports
`sopack/2` (the concurrent Python-side migration landed before this leg was
written, so this ran for real rather than being left as a stub — see the
script's own docstring). Output: `check()` clean, all 3 points streamed and
id-verified, `verify()` clean.

### Direction 3 — Rust reads a Python-written `sopack/2` pack

[`crates/sopack-format/tests/conformance_v2_read_python_written.rs`](../crates/sopack-format/tests/conformance_v2_read_python_written.rs)
scans `qdrant/packs/*.sopack` at test time for one with `schema ==
"sopack/2"` and a `created_by` that doesn't mention "rust" (i.e. one the
*Python* CLI wrote, not `make_v2_conformance_pack`'s own output sitting in
the neighboring `conformance/packs/` directory). If found, it runs the same
`check()`/`batches()`/`verify()` trio as direction 1.

**Status (2026-09-24): not yet available, not a failure.** Producing a real
`sopack/2` pack from the Python side needs an actual model run
(`sopack.pack`, fastembed + the ~2.2 GB model) — M1/M4 territory, not this
crate's. The test detects this (prints a note, passes trivially) rather than
failing, and needs **no code change** to start exercising real coverage the
moment such a pack exists anywhere under `qdrant/packs/`.
