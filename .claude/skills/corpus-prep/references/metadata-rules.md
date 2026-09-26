# Metadata rules for `corpus-prep`

Conventions for resolving the metadata fields `sopack extract` takes, with
real examples from `local-archive/imported/pioneers/converted/MANIFEST.md` and
`local-archive/imported/pioneers/converted/_results_pioneers2026.json`. `sopack` drafts nothing:
every value comes from you, resolved by research against the source, never
by picking the first plausible string.

**Required**: `book_code`, `lang`, `title` always; `author` and `year` for
every non-EGW work (`corpus` set). **Optional**: `corpus`, `slug`,
`acquired_from`, `rights`, `book_pair`, `page_kind`.

## The pd-books filename convention

```
<author_key>__<title_kebab>__<year>__<source>.epub
```

Example:
`bates-joseph__explanation-of-the-typical-and-anti-typical-sanctuary__1850__archive.epub`.
Parse it yourself when the stem has a plausible 4-digit year segment in the
second-to-last `__`-part — not every source follows the convention, and when
it doesn't, the filename simply isn't evidence.

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

**Confirm** it against the title page/byline (e.g. `"BY REV. J. N. ANDREWS,
OF N. C."` → cleaned to `J. N. Andrews`) and against the form used for the
same author in an existing `book.json`/`meta.toml`. Two pioneer works by
the same author (e.g. multiple Waggoner titles) must use the **identical**
author string — a stray comma or initial spacing creates a second "author" in
downstream listings.

Note there are **two J./JH Waggoners** in this corpus: **J. H. Waggoner**
(Joseph Harvey, d. 1889) and **E. J. Waggoner** (Ellet Joseph, his son,
d. 1916). Do not merge them; the `author_key` prefix (`waggoner-jh` vs
`waggoner`) already tells them apart in the 2026 acquisition, but a byline or
OPF creator that just says "Waggoner" needs the full name resolved from
internal evidence (dates, cross-references, publisher) before you write it
into `meta.toml` — from content evidence, not from which code a work
happens to carry.

## The year trap

**Use the work's first-publication year, never the scanned/digital edition's
year**, and never a later reprint's year unless no earlier printing exists.
Any year later than the title-page year — or, for a pioneer/non-EGW author,
later than 1950 at all — is almost certainly a digital-edition date.

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
  `[evidence]` — do not silently backdate it.
- An OPF `<dc:date>` that reads `2011`, `2013` or `2021` on a 19th-century
  pioneer or EGW work is almost always the **modern digital edition's**
  date, never the work's — this is exactly the White Estate re-publication
  pattern the MANIFEST's provenance section documents. Reject it outright
  once you've confirmed the printed edition year from the title page/imprint.

## Book codes

Hand-picked **mnemonics**, never a mechanical function of the title —
`CIS` for *The Cross and Its Shadow*, `WDYS` for *Why Do You Swear?*,
`SGOM` for *The Spirit of God: Its Offices and Manifestations*. The code
you pass is the code the points get; `sopack` never changes it.

**Identity is the importer's decision, not yours.** Whether a code is
taken, and whether the work is already imported under another code, is
decided by the importer's `preflight` from the store's own state, and shown
by the dry-run. You do not look it up. What you do:

- **The user names the code** for a book they know is imported (a
  re-import) — use it, with the slug it was imported with.
- **Otherwise mint one**: 2–5 letters, memorable.
- **Do not trust acquisition records for codes.** `MANIFEST.md`,
  `_results_pioneers2026.json` and the `ACQUIRED-*.json` manifests record
  the code *proposed before import*; import renamed over half of the 2026
  pioneer batch (the pamphlet proposed as `TATS` is live as `BP3`, `SOGO`
  as `SGOM`). Packed under such a code, a work is refused at dry-run as a
  duplicate title — which names the live code to re-pack under.

`book_pair` defaults to mirroring `book_code` (same value) — leave it alone
unless the book has a genuinely separate de/en pairing code.

## `corpus`

- Author is **Ellen G. White** → leave `corpus` **absent** (not `"none"`,
  not `null` written explicitly — just omit the key/line).
  `sopack_book::validate`'s EGW exemption depends on the key being
  genuinely absent.
- Any other author → `corpus = "pioneers"`. This is the only value seen in
  the current acquisition (Miller, Bates, Canright, Haskell, Jones, Smith,
  Waggoner (both), White (James), Andrews, Crosier, Fitch, Litch, …).

## `slug`

Reviewed slugs drop the year and source suffix from the pd-books filename:
`{author_key}-{title_kebab}` — e.g. `andrews-why-do-you-swear` (from
`andrews__why-do-you-swear__1861__archive`), or
`bates-joseph-typical-and-anti-typical-sanctuary` (shortened further by hand
from the full kebab title — slugs may be trimmed for readability as long as
they stay unique and traceable to the source). Shortening the
`author_key-title_kebab` form by hand is fine, inventing an unrelated slug
is not. On a re-import, keep the slug the book was imported with: the
importer treats a different slug on an existing code as a different work
and refuses.

## `acquired_from`

Normalise an `archive.org/download/<id>/...` URL to its details-page form:
`archive.org/details/<id>` (e.g.
`https://archive.org/download/whydoyouswear00andr/whydoyouswear00andr.pdf` →
`archive.org/details/whydoyouswear00andr`, matching
`conformance/extract/goldens/wdys.book.json`).

## `rights`

`"Public domain"` (however capitalized/spaced in the OPF) → `"public-domain"`
— lowercase, spaces to hyphens.

Every file under `local-archive/imported/pioneers/converted/` was checked individually for a live
copyright assertion (`docs/... MANIFEST.md`'s provenance section) — do not
assume `rights` is `public-domain` just because a source is "old"; confirm the
OPF's `<dc:rights>` (or the acquisition's documented finding) says so. A file
carrying `<dc:rights>Copyright © … Ellen G. White Estate, Inc.</dc:rights>`
still needs the same public-domain **legal** argument the MANIFEST makes
(published pre-1929, author dead 70+ years in the EU) recorded in
`[evidence]`, not silently passed through as Estate-copyrighted.

## `lang`

Take OPF `dc:language` as a candidate and confirm it by reading a page of
body text — an OPF language tag is sometimes a default, not a fact.

## Writing `<source>.meta.toml`

Plain TOML, one line per field, plus an `[evidence]` table (free text per
field name) recording *why* each value was chosen — required by this skill
even though the CLI itself only requires the field values:

This example is a **re-import** the user asked for: the book is live as
`BP3`, so `book_code` and `slug` are the live ones, not the manifest's
pre-import proposal:

```toml
book_code = "BP3"
lang = "en"
title = "An Explanation of the Typical and Anti-typical Sanctuary by the Scriptures, with a Chart"
author = "Joseph Bates"
year = 1850
corpus = "pioneers"
slug = "explanation-of-the-typical-and-anti-typical-sanctuary"
acquired_from = "archive.org/details/<id>"
rights = "public-domain"

[evidence]
title = "title page p.1, matches opf:dc:title"
author = "title page byline \"BY JOSEPH BATES\"; matches filename author_key bates-joseph"
year = "title page imprint 1850; matches filename year segment"
book_code = "user: re-import of the live BP3 (the conversion manifest's pre-import TATS is not used)"
slug = "user: the slug BP3 was imported with"
corpus = "author is not Ellen G. White"
acquired_from = "opf:dc:source normalized from the archive.org download URL"
rights = "opf:dc:rights 'Public domain' normalized"
```
