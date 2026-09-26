---
name: corpus-prep
description: Turn a raw book source (EPUB, Markdown, text or sop_json) into a verified .sopack ready for the bible-sop corpus, using the sopack CLI's extract -> inspect -> pack -> verify loop, with every metadata field resolved and evidenced by research (not guessed). Use when asked to prepare, pack or import a book into the SoP/Bible corpus, to make a .sopack, to work out metadata (book code, author form, year, rights) for a pioneer or Ellen G. White book, or to run sopack on a source file or a folder of sources.
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
the pack path, its sha256, and the exact admin-UI steps (see §7) and stop
there, even if you are fully confident the pack is good.

## 0. Preconditions

```bash
sopack --version
sopack commands --json      # confirms this build has extract/inspect/pack/verify
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

## 1. Read the source and resolve the metadata

`sopack` packs exactly the metadata it is given — it drafts nothing and
knows nothing about what is already imported. Resolve every field yourself
from the source and the acquisition records, per
[references/metadata-rules.md](references/metadata-rules.md):

- **Required**: `book_code`, `lang`, `title`; plus `author` and `year` for
  every non-EGW work (`corpus` set). `extract` refuses (exit 4) without them.
- **Optional**: `corpus`, `slug`, `acquired_from`, `rights`, `book_pair`,
  `page_kind`.

Where to look: the title page and imprint (read the first spine documents
of the EPUB yourself), the OPF metadata, the pd-books filename
(`<author_key>__<title_kebab>__<year>__<source>`), and the acquisition
records (`local-archive/imported/pioneers/converted/MANIFEST.md`, `_results_pioneers2026.json`,
`downloads/**/ACQUIRED-*.json`). Treat every one of these as evidence to
cross-check, not as an answer — a scanned 19th-century EPUB's OPF often
carries the digital edition's date, and a "BY …" line can be part of the
title (*"… by the Scriptures"*).

- **Author form**: derive from `author_key` (surname-first, ≤3-letter tokens
  become initials), confirm against the title-page byline — one author, one
  spelling, everywhere.
- **Year**: the work's **first-publication** year, never the digital
  edition's or a later reprint's, unless no earlier printing exists (and if
  so, say which printing you actually have in `[evidence]`).
- **Book code**: the one the user gives you. For a book the user says is
  already imported, that is its live code. Otherwise pick a short, memorable
  mnemonic (not a mechanical acronym). Do **not** try to work out on your
  own whether the book is already in the corpus — the importer decides that
  from the store's state at dry-run (§7).
- **`corpus`**: absent entirely for Ellen G. White works; `"pioneers"` for
  every other author.
- **`slug`**: `{author_key}-{title_kebab}` (year/source dropped), trimmed for
  readability if you like, but traceable back to the source. For a book the
  user says is a re-import, use the slug it was imported with — the
  importer compares slugs on an existing code.
- **`acquired_from`**: normalize an `archive.org/download/<id>/…` URL to
  `archive.org/details/<id>`.
- **`rights`**: `"public-domain"` (lowercase, hyphenated) once you've
  confirmed the OPF/acquisition record actually supports it — an Estate
  `<dc:rights>` string still needs the public-domain argument recorded, not
  silently accepted or silently dropped.

Ask the user for anything the evidence doesn't settle — never guess a field.

## 2. Write `<source>.meta.toml`

Write the sidecar next to the source: every field you resolved, plus an
`[evidence]` table entry per field explaining *why* (title-page location,
cross-reference, MANIFEST.md entry, the user's instruction, …) — required by
this skill even though the CLI itself only checks the field values. See
metadata-rules.md's worked example. (Plain CLI flags work too; the sidecar
is what keeps the evidence reviewable.)

## 3. Extract

```bash
sopack extract <source> --out <source-stem>.book.json --json
```

(`<source>.meta.toml` is picked up automatically if present next to
`source` — `--meta` is only needed to point at a differently-named sidecar.)

- **Exit 4** (`needs_metadata`): the error's `field` names exactly what's
  still missing — go back to step 1 for that field, update the sidecar,
  retry. Do not pass the value only as a CLI flag to route around a
  `meta.toml` you're not confident in; if flag and sidecar disagree,
  `extract` itself refuses (exit 2) rather than silently picking one.
- **Exit 3** (`input_invalid`): a structural problem, not a missing field —
  report it; this is a source or CLI bug, not something to work around here.
- **Folder batch**: `sopack extract <dir>/ --meta-from-sidecars -o <out-dir>/
  --json` once every file in the folder has its own sidecar from steps 1–2.
  Failures are per-file; the run only fails outright if every file failed —
  check `errors[]` even on a run that "succeeded".

## 4. Inspect

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
wildly off from the word/chapter counts in `local-archive/imported/pioneers/converted/MANIFEST.md`
or `_results_pioneers2026.json` if this book has an entry there — expect a
**close, not exact** match, since the manifest's word count comes from a
different pypdf-based conversion path than the EPUB chunker), stop and
report it rather than proceeding to the slow `pack` step; this is a
`sopack-extract` question, not something this skill's loop can fix by
retrying.

## 5. Pack

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

## 6. Verify

```bash
sopack verify <out.sopack> --json
```

Must return `"clean": true` and an empty `"errors"` array. Anything else is
a pack you do not hand to the user as done — report the `errors[]` verbatim.

## 7. Hand off to the user — do not import

Compute the pack's own sha256 (`sha256sum <out.sopack>` / `shasum -a 256
<out.sopack>` — this is exactly what the admin upload API's `POST
/import/uploads` body needs) and report:

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
- What the dry-run's `preflight` may say, and what it means — **the
  importer, not this skill, decides a book's identity**, from the store:
  - *book_code collision (existing slug … != incoming …)*: a different work
    already holds this code → pick another code with the user, re-extract.
  - *new book(s) duplicate a live title … already imported as `<CODE>`*:
    the work is already in the corpus under `<CODE>` → if it is the same
    work, re-extract under `<CODE>` (and its slug) and import as a re-index;
    if it is a separate volume/edition, the user ticks *Allow same title*.
  - *point(s) already exist; refusing without allow_overwrite*: a re-index
    of the same book → the user ticks *Allow overwrite* if that is intended.

## Folder batches, end to end

1. Resolve the metadata of every file in the folder and write its
   `<file>.meta.toml` (steps 1–2).
2. `sopack extract <dir>/ --meta-from-sidecars -o <out-dir>/ --json` once.
3. `sopack inspect` each resulting `book.json` (step 4).
4. `sopack pack <out-dir>/*.book.json -o <combined>.sopack --progress json
   --json` — packing several `book.json`s together is fine as long as they
   share one `profile`; a mixed pack is refused outright.
5. `sopack verify` and hand off exactly as in step 7 — the dry-run reports
   identity problems per book.
