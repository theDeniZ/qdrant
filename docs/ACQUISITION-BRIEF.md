# Acquisition brief — 24 pioneer works for SoP+

Everything needed to send an agent after the missing books and hand you a
folder that is ready to import. **No agent imports anything.** The build and
the Qdrant write stay with you.

Companion documents: [CORPUS-ACQUISITION.md](CORPUS-ACQUISITION.md) for what
is already held, [COVERAGE.md](COVERAGE.md) for what the corpus contains.

---

## 1. Who should do this, and why

| Phase | Work | Model | Agents |
|---|---|---|---|
| A+B+C | Find, judge provenance, download | **Sonnet** | 2, in parallel, split by author group |
| D | Normalise the manifest, verify each file opens and has a text layer | **Haiku** | 1, fresh, after both Sonnets finish |

**Sonnet, not Haiku, for A-C.** The hard part is not searching, it is
telling a 1920s library scan from a modern White Estate digital edition
uploaded to the same site, and telling a full book from a 6-page stub. Both
are judgement calls on ambiguous evidence and Haiku will take the first
plausible file. **Haiku, not Sonnet, for D**, which is counting pages and
reformatting JSON.

**Two agents is the maximum and it is safe here** because the work is
independent, read-only research plus downloads into two different
directories. They never touch the same file. Do not raise it to three: the
provenance rules are the point and they get applied sloppily under
parallelism pressure.

Split the list at the group boundary in section 3: **Agent 1 takes groups 1-3
(13 works), Agent 2 takes groups 4-6 (11 works).**

If you would rather run it in one pass, give one Sonnet the whole list; it is
slower but the provenance judgement stays in one head, which is worth
something.

---

## 2. Shared rules block

Paste this into both prompts verbatim.

```
PROVENANCE RULES — these are the point of the task, not preamble.

1. Never source anything from egwwritings.org or media2.egwwritings.org.
   Not one file, whatever it claims to be.

2. archive.org is NOT automatically safe. 18 archive.org EPUBs in this
   collection turned out to be White Estate digital editions asserting
   "Copyright (c) 2011/2013, Ellen G. White Estate, Inc." in their OPF.
   Discriminator that works:
     - Estate uploads use CamelCase identifiers, e.g.
       ChristsObjectLessons_EllenWhite
     - genuine library scans use lowercase catalogue-style identifiers, carry
       a library `contributor` (Harvard, Duke Divinity, NYPL, U Toronto,
       Library of Congress) and a NOT_IN_COPYRIGHT status
   Prefer a scan of the original pre-1929 printing every time. Record the
   identifier, the contributor and the rights string for every candidate.

3. Reject stubs. The recurring trap in this collection is a 6-page PDF that
   presents as a whole book. Verify page count and file size, and confirm the
   last pages are book text and not a catalogue. State the page count you saw.

4. Report a gap, never substitute. If a work cannot be found from a clean
   source, say so and stop on that work. Do not download a different edition,
   a later revision, a compilation or a reprint and present it as the target.
   A later revised edition is a DIFFERENT WORK for this purpose.

5. Date the printing you actually got, not the work. "The Atonement, 1884" is
   the work; the file may be an 1888 or 1900 printing and the manifest must
   say which.

6. No em dash anywhere in your output.

7. Tool errors are reported, not worked around. If a download fails or a site
   refuses, name the tool, the URL and the error.

HARD STOPS
- Do not run any git command.
- Do not run build_pioneers_corpus.py, index_pioneers_qdrant.py,
  export_book_titles.py or split_corpus.py. You are not importing anything.
- Do not modify, move or delete any existing file. You only add new files
  under the download directory named in your prompt, plus your own manifest.
- Do not write to qdrant/app/, qdrant/references/ or generator/.
```

---

## 3. The list, with reserved book codes

Codes checked 2026-09-20 against all 618 English, 88 German, 8 Japanese and
44 Korean codes in `app/data/sop_books.json`. **None collides.** Reserve them
now so two acquisitions cannot pick the same one later.

### Group 1 — Haskell and J.H. Waggoner (top priority)

| Code | Work | Author | Year |
|---|---|---|---|
| `CIS` | The Cross and Its Shadow | S.N. Haskell | 1914 |
| `ATNW` | The Atonement in the Light of Nature and Revelation | J.H. Waggoner | 1884 |
| `SOGO` | The Spirit of God: Its Offices and Manifestations | J.H. Waggoner | 1877 |
| `FETE` | From Eden to Eden | J.H. Waggoner | 1888 |
| `NTMS` | The Nature and Tendency of Modern Spiritualism | J.H. Waggoner | 1857 |

### Group 2 — the founding sanctuary documents (short, high value)

| Code | Work | Author | Year |
|---|---|---|---|
| `DSE` | Day-Star Extra, 7 February 1846 | O.R.L. Crosier | 1846 |
| `EDSN` | The Hiram Edson manuscript fragment | Hiram Edson | c.1850 |
| `AS23` | The Sanctuary and Twenty-three Hundred Days | J.N. Andrews | 1853 |
| `TMR14` | The Three Messages of Revelation XIV | J.N. Andrews | 1892 ed. |

### Group 3 — Bates, the four missing tracts

| Code | Work | Author | Year |
|---|---|---|---|
| `OPHV` | The Opening Heavens | Joseph Bates | 1846 |
| `SAWM` | Second Advent Way Marks and High Heaps | Joseph Bates | 1847 |
| `SLGD` | A Seal of the Living God | Joseph Bates | 1849 |
| `TATS` | An Explanation of the Typical and Anti-typical Sanctuary | Joseph Bates | 1850 |

### Group 4 — Millerite

| Code | Work | Author | Year |
|---|---|---|---|
| `PEX1` | Prophetic Expositions, vol. 1 | Josiah Litch | 1842 |
| `PEX2` | Prophetic Expositions, vol. 2 | Josiah Litch | 1842 |
| `PROB` | The Probability of the Second Coming of Christ about A.D. 1843 | Josiah Litch | 1838 |
| `MWM` | Memoirs of William Miller | Sylvester Bliss | 1853 |
| `COOH` | "Come Out of Her, My People" | Charles Fitch | 1843 |

### Group 5 — later pioneers

| Code | Work | Author | Year |
|---|---|---|---|
| `CHOD` | The Church: Its Organization, Order and Discipline | J.N. Loughborough | 1907 |
| `SOTW` | The Saviour of the World | W.W. Prescott | 1929 |
| `DOCP` | The Doctrine of Christ | W.W. Prescott | 1920 |
| `QAWX` | Questions and Answers | M.C. Wilcox | 1911 |

### Group 6 — already downloaded here, never catalogued

These two are **already on disk**. No search needed: confirm the file, record
it in the manifest, and for `CGRJ` note that it needs an OCR pass because it
is the only image-only English PDF in the collection.

| Code | Work | Author | Year | File |
|---|---|---|---|---|
| `CGRJ` | Civil Government and Religion | A.T. Jones | 1889 | `downloads/pioneers/jones-waggoner/jones__civil-government-and-religion__1889__archive.pdf` |
| `TODS` | Thoughts on the Prophecies of Daniel | Uriah Smith | 1899 | `downloads/pioneers/smith-miller/smith__thoughts-on-prophecies-of-daniel__1899__archive.pdf` |

**Titles and years above come from general knowledge, not from a source in
hand.** The agent confirms each against the title page it actually finds and
corrects the manifest where they differ. A correction is a success, not a
failure.

---

## 4. Agent prompt — copy this

Substitute the bracketed parts per agent.

```
Task: acquisition research and download for the SDARM SoP+ corpus. Read-only
research, plus downloads into one directory. You are NOT importing anything.

Working directory: /workspaces/sdarm/qdrant/pd-books
Your download directory: downloads/pioneers/[GROUP-DIR]
Your manifest:          downloads/pioneers/[GROUP-DIR]/ACQUIRED-[N].json

Your works: [PASTE THE GROUP TABLES, INCLUDING THE RESERVED CODES]

Context you need and will not otherwise have:
- This is a corpus of pre-1915 Adventist pioneer literature that feeds a
  Qdrant vector index used to retrieve quotations for published devotional
  material. A wrong edition or a damaged scan becomes a misquotation in print.
- 49 pioneer works are already held. Yours are the missing ones.
- Sources, in order of preference: the Adventist Pioneer Library (the pioneer
  collection at m.egwwritings.org is off limits as a FILE source, but its
  catalogue is fine for confirming a title, author and year),
  documents.adventistarchives.org, archive.org library scans,
  Project Gutenberg, CCEL, Maranatha Media.
- The container has working internet. Use WebSearch and WebFetch to find and
  judge candidates, and curl to download. archive.org's metadata API
  (https://archive.org/metadata/<identifier>) gives you identifier,
  contributor, rights and file list in one call: use it on every candidate
  before downloading.

[PASTE THE SHARED RULES BLOCK FROM SECTION 2 HERE, VERBATIM]

File naming, matching the convention already in this folder:
    <author-surname>__<work-slug>__<year>__<source>.<ext>
e.g. waggoner-jh__the-atonement__1884__archive.pdf
Prefer EPUB, then a PDF with a real text layer, then an image-only PDF
(flagged as needing OCR). Say which you got.

For each work, write one object into your manifest JSON:

{
  "book_code": "ATNW",
  "slug": "waggoner-jh-the-atonement",
  "title": "<title exactly as the title page gives it>",
  "author": "J. H. Waggoner",
  "year_work": 1884,
  "year_printing": 1888,
  "language": "en",
  "file": "downloads/pioneers/<dir>/<filename>",
  "bytes": 12345678,
  "format": "pdf-text | pdf-image | epub",
  "pages": 412,
  "source_repo": "archive | gutenberg | ccel | adventistarchives | maranatha",
  "source_url": "<direct download URL>",
  "archive_identifier": "<identifier, if archive.org>",
  "contributor": "<library, if archive.org>",
  "rights": "<verbatim rights string from the source>",
  "estate_risk": "none | suspected | confirmed",
  "estate_evidence": "<why you concluded that>",
  "completeness": "<how you verified it is the whole book: page count, last-page check>",
  "needs_ocr": false,
  "confidence": "high | medium | low",
  "notes": "<anything the next person must know>"
}

Work through your list one work at a time. Budget roughly 10 minutes per
work; if a work resists after that, record it as not found with what you
tried and move on. Finishing 9 of 13 cleanly beats 13 with three wrong
editions.

Return to the orchestrator:
1. the manifest path and a one-line-per-work summary table
2. every work you could NOT source, with what you tried and why each
   candidate failed
3. every work where the title, author or year differs from the table above
4. every file you flagged estate_risk other than "none", with the evidence
5. total bytes downloaded
```

Agent 1: `[GROUP-DIR]` = `jh-waggoner-haskell`, `[N]` = 1, groups 1-3.
Agent 2: `[GROUP-DIR]` = `millerite-later`, `[N]` = 2, groups 4-6.

Both directories are new. Group 6's two files stay where they are; Agent 2
records them by their existing path and downloads nothing for them.

---

## 5. Haiku verification pass

Send this after both Sonnets return.

```
Task: mechanical verification of two acquisition manifests. No judgement
calls, no downloads, no searching. Report, do not fix.

Read:
  /workspaces/sdarm/qdrant/pd-books/downloads/pioneers/jh-waggoner-haskell/ACQUIRED-1.json
  /workspaces/sdarm/qdrant/pd-books/downloads/pioneers/millerite-later/ACQUIRED-2.json

For every entry, check and report:
1. the file exists at "file" and its size on disk matches "bytes"
2. no two entries share a "book_code" or a "slug"
3. no "book_code" appears in /workspaces/sdarm/qdrant/app/data/sop_books.json
   under any language
4. every required key is present and non-empty, except "notes"
5. EPUBs: unzip -l opens without error and the archive contains an .opf
6. PDFs: report page count, and whether a text layer exists
   (pdftotext on page 1 returns more than 200 characters; if pdftotext is
   unavailable, say so and skip this check rather than guessing)
7. any entry with estate_risk not "none", or confidence "low"

Output one table plus a list of failures. Change nothing. Do not run git.
```

---

## 6. Import runbook — yours, after the agents finish

Nothing below is run by an agent.

1. **Review the manifests.** Every `estate_risk` that is not `none`, every
   `confidence: low`, and every title or year the agents corrected. This is
   the step the whole provenance discipline exists for.

2. **OCR what needs it.** `needs_ocr: true` entries, plus `CGRJ`. The Fraktur
   pipeline in `pd-books/pipeline/` is German-specific; English image PDFs
   want plain `tesseract -l eng`. Toolchain setup notes are in
   `pd-books/STATE.md`.

3. **Add the works to the catalog.** `pd-books/catalog/works.mjs`, then
   rebuild with `node catalog/build_catalog.mjs`. The manifest fields map
   onto the catalog entry directly: `slug`, `title`, `author`,
   `year` (use `year_work`), `language`, `sourceFile`, `sourceEdition` (use
   `year_printing`), `sourceRepo`.

4. **Reserve the codes.** Add each `book_code` to the `CODES` table in
   `pd-books/qdrant/build_pioneers_corpus.py`. Works whose text carries inline
   `ABBR page.para` references get their code from the file and need no entry;
   everything else needs one.

5. **Build and inspect.**
   ```bash
   cd /workspaces/sdarm/qdrant/pd-books/qdrant
   python3 build_pioneers_corpus.py
   ```
   Read `pioneers_report.md` before going further. Anything over about 1 %
   damage wants a look at the scan. The build drops blocks over 25 % damage on
   their own, so a low overall score can still hide a scrambled opening.

6. **Dry run, then index.**
   ```bash
   QDRANT_URL=... python3 index_pioneers_qdrant.py --dry-run
   QDRANT_URL=... python3 index_pioneers_qdrant.py --parallel 8
   ```
   The preflight refuses on a `book_code` collision, which is the safety net
   under step 4. `--only <slug>` re-runs a single work.

7. **Regenerate the titles. Do not skip this.** This is the step that was
   missed last time and made the whole 2026-08-23 import invisible:
   ```bash
   cd /workspaces/sdarm/qdrant
   python3 scripts/export_book_titles.py /workspaces/sdarm/generator/data/sop \
       --qdrant-url "$QDRANT_URL" --dry-run
   python3 scripts/export_book_titles.py /workspaces/sdarm/generator/data/sop \
       --qdrant-url "$QDRANT_URL"
   ```
   It warns if any code still has no title.

8. **Index `corpus` if you have not yet.**
   `python3 scripts/split_corpus.py index-payload --execute`

9. **Redeploy** the qdrant service, and run `plugin/build_plugin.py` plus a
   plugin redeploy if any reference document changed.

10. **Update the docs**: move the acquired works from "Not held at all" to
    "Already indexed" in [CORPUS-ACQUISITION.md](CORPUS-ACQUISITION.md), and
    refresh the author table in [COVERAGE.md](COVERAGE.md).
