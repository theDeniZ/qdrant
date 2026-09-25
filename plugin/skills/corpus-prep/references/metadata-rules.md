# Metadata rules for `corpus-prep`

Conventions for resolving the 10 fields `sopack propose` drafts, with real
examples from `pd-books/converted/MANIFEST.md` and
`pd-books/converted/_results_pioneers2026.json`. `propose` never fills a
field's `value` unless every candidate agrees **and** at least one candidate
is authoritative (`from` is `title_page`, `sop_json_meta` or `sop_json_dir` —
see `references/cli-contract.md`). Everything else lands in `unresolved` for
you to resolve by research, not by picking the first candidate.

**One important exception**: for `book_code` specifically, the conversion
manifest is evidence of what was *proposed before import*, never of what a
work's code *actually is* — see "Book codes" below. The **live registry**
(`contracts/<contract>/book_codes.json`) and `sop_list_books` are the only
authorities there, and checking them (by title/author, not just by a
candidate code) comes before minting anything.

## The pd-books filename convention

```
<author_key>__<title_kebab>__<year>__<source>.epub
```

Example:
`bates-joseph__explanation-of-the-typical-and-anti-typical-sanctuary__1850__archive.epub`.
`propose` parses this itself (`filename`/`filename_normalized` candidates) whenever
the stem has a plausible 4-digit year segment in the second-to-last `__`-part —
not every source follows it, and when it doesn't, those candidates are simply
absent (not wrong).

## Author name form

Read the surname-first `author_key` as: **first token = surname**, remaining
tokens = given name(s). A token of **≤ 3 letters** is run-together initials
(one period per letter); a **longer** token is a given name, titlecased.

| `author_key` | Resolved form |
|---|---|
| `waggoner-jh` | J. H. Waggoner |
| `haskell-sn` | S. N. Haskell |
| `andrews-jn` | J. N. Andrews |
| `crosier-orl` | O. R. L. Crosier |
| `bates-joseph` | Joseph Bates |
| `white-james` | James White |
| `litch` | Litch (surname only — no given-name tokens) |

This is exactly what `propose`'s `filename` author candidate already computes
— your job is to **confirm** it against the title page/byline (`title_page`
candidate, e.g. `"BY REV. J. N. ANDREWS, OF N. C."` → cleaned to
`J. N. Andrews`) and against the form already used in the corpus
(`sop_list_books(search="<surname>")` — the registry's `codes[<CODE>].title`
doesn't carry authors, so cross-check via the corpus-lookup tools or an
existing `book.json`/`meta.toml` for the same author). Two pioneer works by
the same author (e.g. multiple Waggoner titles) must use the **identical**
author string — a stray comma or initial spacing creates a second "author" in
downstream listings.

Note there are **two J./JH Waggoners** in this corpus: **J. H. Waggoner**
(Joseph Harvey, d. 1889) and **E. J. Waggoner** (Ellet Joseph, his son,
d. 1916). Do not merge them; the `author_key` prefix (`waggoner-jh` vs
`waggoner`) already tells them apart in the 2026 acquisition, but a byline or
OPF creator that just says "Waggoner" needs the full name resolved from
internal evidence (dates, cross-references, publisher) before you write it
into `meta.toml`. Don't rely on a book-code list to disambiguate them either
— see "Book codes" below on why the conversion manifest's codes aren't
authoritative; confirm the author from content evidence, not from which code
a work happens to carry.

## The year trap

**Use the work's first-publication year, never the scanned/digital edition's
year**, and never a later reprint's year unless no earlier printing exists.
`propose` flags this automatically: a `year_trap_warning` is attached to any
candidate greater than the title-page year, or (for a pioneer/non-EGW author)
greater than 1950 outright — read as `"digital-edition date?"`.

Real examples from the 2026 acquisition:

- `andrews-jn__the-sanctuary-and-twenty-three-hundred-days__1872__archive` —
  the **source PDF filename** is
  `andrews-jn__…__1853-2nded1872__archive.pdf` (edition-qualified: 1853 first
  edition, 2nd ed. 1872). The **EPUB** filename correctly uses `1872` (the
  edition actually digitized) — but if you only glanced at the PDF's
  edition-qualified stem you could misread the slug/year segment and shift the
  whole filename parse (this exact bug is failure #9 in
  `docs/IMPORT-PIPELINE.md`). Confirm which printing you actually have before
  trusting any filename segment as the year.
- `andrews-jn__the-three-messages-of-revelation-14-6-12__1876__archive` — the
  source PDF is `…__1876-4thed__archive.pdf`: a **4th edition**, but the work's
  first publication was 1876, so `1876` (not the printing date of the 4th
  edition, which may be later) is what belongs in `meta.toml`, confirmed
  against the title page / imprint, not assumed from the edition number.
- `egw__thoughts-from-the-mount-of-blessing__1928__archive` — first published
  **1896**; the only non-Estate scan available is the **1928** printing. The
  MANIFEST records this openly ("labelled 1928 honestly") because *this
  specific acquired edition* is the 1928 printing, not because 1928 is the
  work's year. When your source is provably a later printing of an
  earlier-first-published work and you cannot get an earlier scan, record the
  edition you actually hold and note the first-publication year in
  `[evidence]` — do not silently backdate a candidate `propose` didn't offer.
- An OPF `<dc:date>` that reads `2011`, `2013` or `2021` on a 19th-century
  pioneer or EGW work is almost always the **modern digital edition's**
  date, never the work's — this is exactly the White Estate re-publication
  pattern the MANIFEST's provenance section documents. Reject it outright
  once you've confirmed the printed edition year from the title page/imprint.

## Book codes

Hand-picked **mnemonics**, never a mechanical function of the title —
`CIS` for *The Cross and Its Shadow*, `WDYS` for *Why Do You Swear?*,
`SGOM` for *The Spirit of God: Its Offices and Manifestations*. `propose`'s
`title_heuristic` candidate (first letter of up to 4 significant title words)
is a **starting point only** — it is not expected to reproduce the actual
code, and it never becomes a resolved `value` by itself unless a
`registry:title_match` candidate (see below) supplies one.

### The registry — not the conversion manifest — is authoritative

**Check whether the work is already indexed before minting any code at
all.** `pd-books/converted/MANIFEST.md` and `_results_pioneers2026.json`
record the code *proposed at conversion time, before import* — they are
**not** the live codes. Import has renamed a majority of the 2026 pioneer
batch's proposed codes (grouping an author's pamphlets together, avoiding
collisions, aligning with the live title table, …). Concretely, from that
one batch: the pamphlet proposed as `TATS` was imported as `BP3`; the work
proposed as `SOGO` was imported as `SGOM`; over half of the 22 pioneer codes
in that manifest were renamed on import. **Never cite a manifest code as a
book's actual code without confirming it against the live registry or
`sop_list_books` first.**

The only authorities for a work's real code are:

- **The registry** — `contracts/<contract>/book_codes.json`, regenerated
  from live Qdrant by the importer's admin command. `propose --json`'s
  `book_code` candidates carry `"collision": true/false` against it when
  `--registry` resolved (default: next to the resolved contract).
- **`sop_list_books(search=…)`** (search by title *and* by author, not just
  by a candidate code — a hit only on a code search tells you nothing about
  a *different* code the same work might already carry).

A `book_code` candidate tagged `from: "registry:title_match"` — an offline
registry lookup by title, usually carrying a warning like `"already in the
store as <CODE>"` — is **decisive evidence** once you've separately
confirmed the candidate's author and year match your source (a title match
alone can be wrong across near-identical volume titles, e.g. a two-volume
work). When it's present, reuse that code; treat the run as a **re-import /
update** of an existing book and say so prominently in your report — do not
propose a fresh mnemonic alongside it.

Only once you've confirmed the work is genuinely **not** already indexed do
you mint a new code: something short (2–5 letters), memorable, distinct from
any existing code, checked for a clean `collision: false`. Remember that a
clean collision check on your invented string proves only that *the string*
is free — it does not prove the *work* isn't indexed under something else,
which is why the title/author search above always comes first.

`book_pair` defaults to mirroring `book_code` (same value) — leave it alone
unless the book has a genuinely separate de/en pairing code.

## `corpus`

- Author is **Ellen G. White** → leave `corpus` **absent** (not `"none"`,
  not `null` written explicitly — just omit the key/line). `propose` never
  emits a `corpus` candidate for EGW works at all; `sopack_book::validate`'s
  EGW exemption depends on the key being genuinely absent.
- Any other author → `corpus = "pioneers"`. This is the only value seen in
  the current acquisition (Miller, Bates, Canright, Haskell, Jones, Smith,
  Waggoner (both), White (James), Andrews, Crosier, Fitch, Litch, …).

## `slug`

Reviewed slugs drop the year and source suffix from the pd-books filename:
`{author_key}-{title_kebab}` — e.g. `andrews-why-do-you-swear` (from
`andrews__why-do-you-swear__1861__archive`), or
`bates-joseph-typical-and-anti-typical-sanctuary` (shortened further by hand
from the full kebab title — slugs may be trimmed for readability as long as
they stay unique and traceable to the source). `propose`'s
`filename_normalized` candidate computes the un-trimmed
`author_key-title_kebab` form; shortening it by hand is fine, inventing an
unrelated slug is not. A `slug` is independent of `book_code` — trimming it
for readability never justifies skipping the book-code registry/re-import
check above.

## `acquired_from`

Normalise an `archive.org/download/<id>/...` URL to its details-page form:
`archive.org/details/<id>` — `propose`'s `opf:dc:source_normalized` candidate
already does this (e.g.
`https://archive.org/download/whydoyouswear00andr/whydoyouswear00andr.pdf` →
`archive.org/details/whydoyouswear00andr`, matching
`conformance/extract/goldens/wdys.book.json`). Prefer the normalized
candidate's value over the raw OPF source URL.

## `rights`

`"Public domain"` (however capitalized/spaced in the OPF) → `"public-domain"`
— lowercase, spaces to hyphens. `propose`'s `opf:dc:rights_normalized`
candidate already applies this transform; use its value.

Every file under `pd-books/converted/` was checked individually for a live
copyright assertion (`docs/... MANIFEST.md`'s provenance section) — do not
assume `rights` is `public-domain` just because a source is "old"; confirm the
OPF's `<dc:rights>` (or the acquisition's documented finding) says so. A file
carrying `<dc:rights>Copyright © … Ellen G. White Estate, Inc.</dc:rights>`
still needs the same public-domain **legal** argument the MANIFEST makes
(published pre-1929, author dead 70+ years in the EU) recorded in
`[evidence]`, not silently passed through as Estate-copyrighted.

## `lang`

`propose` cross-checks OPF `dc:language` against a body-text heuristic
(script detection for Cyrillic/Hangul/CJK, stopword counts for en/de/fr/es).
They should agree; if they don't, read a page of body text yourself before
picking one — the heuristic is deliberately narrow (not a language-id model)
and can be wrong on very short or heavily OCR-damaged samples.

## Writing `<source>.meta.toml`

Plain TOML, one line per field, plus an `[evidence]` table (free text per
field name) recording *why* each value was chosen — required by this skill
even though the CLI itself only requires the field values:

This example is a **re-import**: the source is already indexed live (found
via `sop_list_books(search="Typical and Anti-typical Sanctuary")` and
confirmed by author/year), so `book_code` reuses the existing live code
rather than the manifest's stale, pre-import proposal — note the
`[evidence]` entry says so explicitly, not just "no collision":

```toml
book_code = "BP3"
lang = "en"
title = "An Explanation of the Typical and Anti-typical Sanctuary by the Scriptures, with a Chart"
author = "Joseph Bates"
year = 1850
corpus = "pioneers"
slug = "bates-joseph-typical-and-anti-typical-sanctuary"
acquired_from = "archive.org/details/<id>"
rights = "public-domain"

[evidence]
title = "title page p.1, matches opf:dc:title"
author = "title page byline \"BY JOSEPH BATES\"; matches filename author_key bates-joseph"
year = "title page imprint 1850; matches filename year segment"
book_code = "already indexed live as BP3 (sop_list_books title+author match, confirmed against contracts/e5-large-v1/book_codes.json); this is a RE-IMPORT — the conversion manifest's proposed code (TATS) was renamed on import and is not used"
corpus = "author is not Ellen G. White"
acquired_from = "opf:dc:source normalized from the archive.org download URL"
rights = "opf:dc:rights 'Public domain' normalized"
```

`sopack propose --write-meta` writes a starting template with every resolved
field live and every unresolved one commented out with its candidates listed
underneath (value, source, evidence, warning, collision) — editing that
template (uncomment/adjust the line you settled on, add `[evidence]` by hand)
is the fastest path and keeps you from retyping resolved values. `book_pair`
is deliberately left out of the template (it mirrors `book_code`); only add
it if the book genuinely needs a different pairing code.
