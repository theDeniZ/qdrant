---
name: corpus-prep
description: Turn a raw book source (EPUB, Markdown, text or sop_json) into a verified .sopack ready for the bible-sop corpus, using the sopack CLI's propose -> extract -> inspect -> pack -> verify loop, with every metadata field resolved and evidenced by research (not guessed). Use when asked to prepare, pack or import a book into the SoP/Bible corpus, to make a .sopack, to work out metadata (book code, author form, year, rights) for a pioneer or Ellen G. White book, or to run sopack on a source file or a folder of sources.
---

# Corpus prep: source book -> verified `.sopack`

Turns one book (or a folder of books) into a verified, importable `.sopack`
using the **sopack** Rust CLI (`qdrant/sopack-rs/`, design:
`qdrant/docs/SOPACK-1.0-PLAN.md` §3.5). Read
[references/cli-contract.md](references/cli-contract.md) (every command,
JSON schema and exit code, generated from the live binary) and
[references/metadata-rules.md](references/metadata-rules.md) (book-code,
author-form, year-trap, corpus/slug/rights conventions, with real examples)
before your first run.

## Stop before import — non-negotiable

This skill's job ends at a **verified pack on disk**. It never uploads,
never talks to the admin API, never touches Qdrant. A write to the live
corpus needs the user's explicit, fresh confirmation in-conversation — the
same rule that gates every YouVersion push in this workspace. Hand the user
the pack path, its sha256, and the exact admin-UI steps (see §9) and stop
there, even if you are fully confident the pack is good.

## 0. Preconditions

```bash
sopack --version
sopack commands --json      # confirms this build has propose/extract/pack/verify
                             # and their current flags — don't assume, check
sopack doctor --quick --json
```

`doctor --quick` treats a missing model/ONNX-Runtime dylib as a **warning**,
not a failure — read its `checks[]` for `model` presence specifically. If the
model is missing:

```bash
sopack model fetch --json --progress json
```

This is the CLI's **only** network action (~2.2 GB, resumable, sha256-
verified on completion) — tell the user before starting a fresh download if
it looks like it will take a while, and relay the NDJSON progress on stderr
as it runs. `pack` itself never downloads; if you skip this and `pack` later
exits 6, its `hint` names this exact command.

If several agents/processes in your environment might load the model at the
same time, serialize the model-loading commands (`pack`, `calibrate`, a
non-`--quick` `doctor`) so only one model copy is resident at once — it needs
roughly 2.5 GB of RAM per copy.

## 1. Propose

```bash
sopack propose <source> --json
```

Parse the `propose` schema's `fields`/`unresolved`/`warnings` (see
cli-contract.md). Expect `book_code` to be `unresolved` on almost every real
book — it is a hand-picked mnemonic, not something `propose` invents (its
`title_heuristic` candidate is a *starting point*, never a resolved value).
On a scanned/OCR'd 19th-century EPUB, expect most or **all** fields to come
back `unresolved`: the `title_page`/byline heuristics only fire on the first
one or two spine documents and commonly pick up the wrong line entirely —
e.g. on `bates-joseph__…-typical-and-anti-typical-sanctuary__1850__archive`,
the `author` `title_page` candidate came back `"THE SCRIPTURES"`, a
false-positive match of the "BY …" byline regex against the title page's own
trailing phrase *"by the Scriptures"* — not a byline at all. This is
expected, not a bug: cross-check every `title_page`-sourced candidate against
the OPF/filename candidates (or the actual page image if you have it) before
trusting it, precisely *because* `title_page` is one of the few sources
`propose` treats as authoritative enough to auto-resolve a field on its own.

Do **not** treat a resolved field as beyond scrutiny either: check every
candidate's `warning` (e.g. a year candidate flagged `"digital-edition
date?"`) even on a field that did resolve, since a warning sits on the
*candidate*, not the resolved value. A `book_code` candidate tagged
`registry:title_match` is the one exception to "book_code almost always
lands in `unresolved`" — see step 2, next.

**Folder batches**: point `propose` at each file in the folder individually
(there is no folder-level `propose`) — `extract --meta-from-sidecars` is
what operates on a whole folder at once, once every file has its own
`<file>.meta.toml` next to it.

## 2. Check whether this book is already in the corpus (re-import check)

**Do this before inventing any metadata**, especially `book_code`. Search the
**live store** by title and author, not only by a guessed code: a work
already indexed keeps its existing code, and re-preparing it is a
**re-import/update**, not a fresh acquisition.

- If the corpus-lookup MCP tools are available: `sop_list_books(search="<a
  few distinctive title words>")` and `sop_list_books(search="<author
  surname>")`. A hit on the same work (matching title *and* author) means
  it is already in the store — note its existing code and **flag this to the
  user prominently**: this run is a re-import/update of an existing book, not
  a new one.
- `propose`'s `book_code` candidates may include one tagged
  `registry:title_match` (an offline registry lookup by title), typically
  carrying a warning like `"already in the store as <CODE>"`. Treat this as
  **decisive evidence** for `book_code` once you've confirmed the
  candidate's author and year genuinely match your source (a title match
  alone isn't enough if, say, two different volumes of a series share a
  near-identical title) — reuse that code; do not mint a fresh mnemonic
  alongside it.
- The **conversion manifest** (`pd-books/converted/MANIFEST.md`,
  `_results_pioneers2026.json`) is **not** authoritative for `book_code`. It
  records the code *proposed at conversion time, before import* — and import
  has renamed a majority of the 2026 pioneer batch's proposed codes (to
  avoid collisions or align with the live title table). The **registry**
  (`contracts/<contract>/book_codes.json`, regenerated from live Qdrant) and
  `sop_list_books` are the only authorities for a work's actual code. A
  `"collision": false` on a code you invented only means *that string* is
  free right now — it says nothing about whether the work is already
  indexed under a **different** code, which is exactly why the title/author
  search above is mandatory, not optional, even when your own candidate code
  shows no collision.

One real, concrete example from this workspace: a pioneer pamphlet was
proposed at conversion time with the mnemonic `TATS`
(`_results_pioneers2026.json`), but the 2026 import indexed it under a
**different** code once grouped with the author's other pamphlets in the
live store. A fresh `corpus-prep` run on that same source must resolve to
the **live** code (found via `sop_list_books`/the registry), never re-mint
the manifest's `TATS` as if the work were new — packing it under a stale,
unused code would silently create a duplicate copy of an already-indexed
work.

Once you've confirmed the book is (or isn't) already indexed, continue to
step 3 for whatever remains unresolved.

## 3. Research and resolve every unresolved field

For each field in `unresolved`, and any resolved field with a warning on it,
follow [references/metadata-rules.md](references/metadata-rules.md):

- **Author form**: derive from `author_key` (surname-first, ≤3-letter tokens
  become initials), confirm against the title-page byline candidate and, if
  the corpus-lookup tools are available, against the form already used
  elsewhere in the corpus (`sop_list_books(search=…)`) — one author, one
  spelling, everywhere.
- **Year**: the work's **first-publication** year, never the digital
  edition's or a later reprint's, unless no earlier printing exists (and if
  so, say which printing you actually have in `[evidence]`). The filename's
  year segment and the OPF `dc:date` are both just candidates — confirm
  against the title page/imprint.
- **Book code**: if step 2 found the work already indexed, use its live
  code — stop here, do not invent one. Otherwise: pick a short, memorable
  mnemonic (not a mechanical acronym), and check it for collisions —
  `propose`'s own `collision` flag when `--registry` resolved (default
  `contracts/<contract>/book_codes.json` next to the resolved contract), and
  cross-check with `sop_list_books` if available. A clean `collision: false`
  confirms only that the *string* is free (step 2 is what confirms the
  *work* itself isn't already indexed under something else).
- **`corpus`**: absent entirely for Ellen G. White works; `"pioneers"` for
  every other author.
- **`slug`**: `{author_key}-{title_kebab}` (year/source dropped), trimmed for
  readability if you like, but traceable back to the source.
- **`acquired_from`**: normalize an `archive.org/download/<id>/…` URL to
  `archive.org/details/<id>`.
- **`rights`**: `"public-domain"` (lowercase, hyphenated) once you've
  confirmed the OPF/acquisition record actually supports it — an Estate
  `<dc:rights>` string still needs the public-domain argument recorded, not
  silently accepted or silently dropped.

## 4. Write `<source>.meta.toml`

Either start from `sopack propose --write-meta` (a template with resolved
fields live and every unresolved one commented out with its candidates) and
edit it, or write the file directly. Either way it must end with **every**
field you resolved recorded, plus an `[evidence]` table entry per field
explaining *why* (title-page location, cross-reference, MANIFEST.md entry,
registry check, …) — required by this skill even though the CLI itself only
checks the field values. See metadata-rules.md's worked example.

## 5. Extract

```bash
sopack extract <source> --out <source-stem>.book.json --json
```

(`<source>.meta.toml` is picked up automatically if present next to
`source` — `--meta` is only needed to point at a differently-named sidecar.)

- **Exit 4** (`needs_metadata`): the error's `field` names exactly what's
  still missing — go back to step 3 for that field, update the sidecar,
  retry. Do not pass the value only as a CLI flag to route around a
  `meta.toml` you're not confident in; if flag and sidecar disagree,
  `extract` itself refuses (exit 2) rather than silently picking one.
- **Exit 3** (`input_invalid`): a structural problem, not a missing field —
  report it; this is a source or CLI bug, not something to work around here.
- **Folder batch**: `sopack extract <dir>/ --meta-from-sidecars -o <out-dir>/
  --json` once every file in the folder has its own sidecar from steps 1–4.
  Failures are per-file; the run only fails outright if every file failed —
  check `errors[]` even on a run that "succeeded".

## 6. Inspect

```bash
sopack inspect <book.json> --json
```

Read `stats.damage` (should be small — a few percent on clean text, higher
on badly OCR'd 19th-century scans is expected and not itself a problem),
`stats.dropped`/`dropped_detail` (skim a few — they should be genuine
junk/too-short fragments like running-head/TOC lines, not real content
silently lost), `collided_para_keys` and `split_blocks`. There is no
chunker-tuning flag on `extract` — if something here looks structurally
wrong (not just "OCR is messy" but e.g. whole chapters missing, block count
wildly off from the word/chapter counts in `pd-books/converted/MANIFEST.md`
or `_results_pioneers2026.json` if this book has an entry there — expect a
**close, not exact** match, since the manifest's word count comes from a
different pypdf-based conversion path than the EPUB chunker), stop and
report it rather than proceeding to the slow `pack` step; this is a
`sopack-extract` question, not something this skill's loop can fix by
retrying.

## 7. Pack

```bash
sopack pack <book.json> -o <out.sopack> --progress json --json 2>progress.ndjson
```

Relay progress to the user as it runs (stderr NDJSON, `embed` stage is
token-weighted so its `%` is the most meaningful one to narrate). Then:

- **Exit 0**: read the `pack` result's `calibration_min_cosine` /
  `calibration_mean_cosine` — both should be comfortably above the
  contract's `pack_min_cosine` (0.9999 for `e5-large-v1`; `sopack contract
  show --json` if you need the exact number).
- **Exit 7** (`interrupted`): re-run the **exact same command** — it resumes
  from `<out>.sopack.partial/`'s checkpoint. Only add `--fresh` if you
  deliberately want to discard partial progress.
- **Exit 5** (`calibration_failed`): **stop and report to the user.** Never
  retry with different `--device`/`--threads`/`--batch-tokens` settings to
  try to force a pass — a calibration failure means this machine's model/ORT
  combination does not reproduce the contract's vector space, and packing
  anyway would silently corrupt retrieval quality for everyone who queries
  this book later (`docs/IMPORT-PIPELINE.md` failure #15 is exactly this
  class of bug, previously undetected for months).
- **Exit 6** (`resources`): follow the `hint` (usually `sopack model fetch`,
  or a memory/disk shortfall) and retry once resolved.

## 8. Verify

```bash
sopack verify <out.sopack> --json
```

Must return `"clean": true` and an empty `"errors"` array. Anything else is
a pack you do not hand to the user as done — report the `errors[]` verbatim.

## 9. Hand off to the user — do not import

Compute the pack's own sha256 (`sha256sum <out.sopack>` / `shasum -a 256
<out.sopack>` — this is exactly what the admin upload API's `POST
/import/uploads` body needs) and report:

- **Whether this is a re-import/update of an already-indexed work** (step 2)
  — say so up front, in the first line of the report, not buried in the
  metadata table; this changes what the user should expect the admin UI's
  dry-run to show (an update to an existing point set, not a new addition).
- Pack path and sha256, book/profile/points/id_rule from the `pack` result.
- A metadata table: every field you resolved, its value, and the evidence
  you recorded in `[evidence]` — this is the reviewable trail, not just the
  final numbers.
- Anything you flagged but didn't block on (a high damage ratio, a coarse
  TOC, an author-form judgment call) so the user can double-check it.
- The next step is **theirs**: open the admin UI (`http://127.0.0.1:8081`,
  tunnel with `ssh -L 8081:127.0.0.1:8081 <host>` if remote), drop the
  `.sopack` file on the "Upload a .sopack" panel, and run the import job as
  **dry-run first**, then **apply** only after reviewing the dry-run's
  report — never do this step yourself. Full wire contract:
  `qdrant/docs/IMPORT-API.md`.

## Folder batches, end to end

1. `sopack propose <file> --write-meta` (or `--json` + write by hand) for
   every file in the folder.
2. For each file, run the re-import check (step 2 above) before resolving
   `book_code` — a folder batch is exactly where reusing a live code instead
   of minting a fresh one matters most, since a folder often mixes genuinely
   new titles with re-imports of already-indexed ones.
3. Resolve and finish each `<file>.meta.toml` per steps 3–4 above.
4. `sopack extract <dir>/ --meta-from-sidecars -o <out-dir>/ --json` once.
5. `sopack inspect` each resulting `book.json` (step 6).
6. `sopack pack <out-dir>/*.book.json -o <combined>.sopack --progress json
   --json` — packing several `book.json`s together is fine as long as they
   share one `profile`; a mixed pack is refused outright.
7. `sopack verify` and hand off exactly as in step 9.
