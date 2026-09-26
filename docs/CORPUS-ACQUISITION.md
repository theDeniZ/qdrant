# SoP+ acquisition list — pre-1915 Adventist pioneers

> **2026-09-26:** the book files and old scripts referenced below moved out of
> `qdrant/pd-books/` into `/workspaces/sdarm/local-archive/` (paths rewritten; full map in
> `local-archive/MANIFEST.tsv`). Imports no longer use `build_pioneers_corpus.py` /
> `export_book_titles.py`: build a `.sopack` with the `sopack` CLI (skill `corpus-prep`) and
> import it in the bible-sop admin UI, which also updates the title table.


Target: extend the `sop` Qdrant collection from "Ellen G. White plus an
unlabelled pioneer shelf" to a deliberate **SoP+** corpus: Ellen G. White, the
pre-1915 Adventist pioneers, and a small tagged appendix of the sources the
pioneers themselves used.

**Originals first.** English-language originals only. Translations are a
separate exercise and are not planned here.

**Revised 2026-09-20** against `local-archive/scripts/pd-books-pioneer-import/pioneers_report.md` and
`local-archive/imported/pioneers/catalog/catalog-manifest.json`. An earlier draft of this file listed
several works as missing that are in fact already indexed; the pioneer import
of 2026-08-23 was more complete than the corpus appeared, because none of it
had titles. See [the title fix](#the-title-defect-fixed-2026-09-20).

## Already indexed — 49 pioneer works, 11 authors

Imported 2026-08-23 by `local-archive/scripts/pd-books-pioneer-import/index_pioneers_qdrant.py`,
~60,000 points, 4.69 M words, all carrying `corpus: "pioneers"`.

| Author | Work | Year | Code |
|---|---|---:|---|
| A. T. Jones | Tremont Temple Lectures | 1888 | `TTL` |
|  | The National Sunday Law | 1889 | `NSLS18` |
|  | The Two Republics | 1891 | `TTR` |
|  | The Rights of the People | 1895 | `ROP` |
|  | The Empires of the Bible | 1897 | `EB` |
|  | The Great Empires of Prophecy | 1898 | `GEP` |
|  | Ecclesiastical Empire | 1901 | `ECE` |
|  | The Great Nations of To-Day | 1901 | `GNT` |
|  | The Place of the Bible in Education | 1903 | `PBE` |
|  | The Consecrated Way to Christian Perfection | 1905 | `CWCP` |
|  | An Exposition of Matthew Twenty-Four | 1915 | `EMTF` |
| D. M. Canright | Matter and Spirit | 1871 | `MAS` |
| E. J. Waggoner | Fathers of the Catholic Church | 1888 | `FCC` |
|  | The Gospel in the Book of Galatians | 1888 | `GBG` |
|  | Christ and His Righteousness | 1890 | `CHR` |
|  | Sunday: The Origin of Its Observance | 1892 | `SOOCC` |
|  | The Gospel in Creation | 1894 | `GOSC` |
|  | Waggoner on Romans | 1896 | `WROM` |
|  | Prophetic Lights | 1897 | `PROLI` |
|  | The Everlasting Covenant | 1900 | `EVCO` |
|  | The Glad Tidings | 1900 | `GT` |
| G. I. Butler | The Change of the Sabbath | 1889 | `COS` |
| J. N. Andrews | Why Do You Swear? | 1866 | `WDYS` |
|  | History of the Sabbath and First Day of the Week | 1873 | `HSFD` |
|  | The Complete Testimony of the Fathers | 1876 | `CTF` |
| J. N. Loughborough | Rise and Progress of the Seventh-day Adventists | 1892 | `RPSDA` |
|  | The Great Second Advent Movement | 1909 | `GSAM` |
| James White | Life Incidents | 1868 | `LI` |
|  | Sketches of the Christian Life and Public Labors of William Miller | 1875 | `SCLWM` |
|  | Life Sketches | 1880 | `LSJW` |
|  | His Glorious Appearing | 1896 | `HGA` |
| Joseph Bates | The Seventh Day Sabbath, a Perpetual Sign | 1846 | `SDSPS` |
|  | A Vindication of the Seventh-Day Sabbath | 1848 | `VSDS` |
|  | The Autobiography of Elder Joseph Bates | 1868 | `BAB` |
| S. N. Haskell | The Story of the Seer of Patmos | 1905 | `SSP` |
|  | The Story of Daniel the Prophet | 1908 | `SDP` |
| Uriah Smith | The State of the Dead and the Destiny of the Wicked | 1873 | `SOD` |
|  | The Sanctuary and the Twenty-Three Hundred Days | 1877 | `S23D` |
|  | Daniel and the Revelation | 1882 | `DAR` |
|  | Synopsis of the Present Truth | 1883 | `SYPT` |
|  | Man's Nature and Destiny | 1884 | `MND` |
|  | The United States in the Light of Prophecy | 1884 | `USLP` |
|  | The Marvel of Nations | 1887 | `MON` |
|  | Modern Spiritualism | 1896 | `MSp` |
|  | Looking Unto Jesus | 1897 | `LUJ` |
| William Miller | Evidence from Scripture and History of the Second Coming of Christ | 1842 | `EFS` |
|  | Miller's Reply to Stuart's 'Hints on Prophecy' | 1842 | `RTS` |
|  | Views of the Prophecies and Prophetic Chronology | 1842 | `VOP` |
|  | Apology and Defence | 1845 | `AAD` |

Damage is under 1 % for all but two: `WROM` 1.8 % and `DAR` 1.1 %. The two
Haskell works (`SDP`, `SSP`) score 0.6-0.7 % overall but open on scrambled
two-column pages; treat any Haskell hit as needing a look at the scan before
it is quoted.

## Held in pd-books, not indexed — 4

| Work | Author | Why not | What it needs |
|---|---|---|---|
| Civil Government and Religion, 1889 | A.T. Jones | never catalogued; the only image-only English PDF in the collection | an OCR pass, then the normal build |
| Thoughts on the Prophecies of Daniel, 1899 | Uriah Smith | never catalogued; superseded by `DAR`, but it is the separate earlier volume | catalogue, then build |
| What is the Church?, 1913 | A.T. Jones | two-column scan interleaved, word spaces lost, body text duplicated, 14.8 % damage | a better scan |
| Bible Student's Manual of Chronology and Prophecy, 1841 | William Miller | a glossary broken mid-entry every line, ~14 words per block | a better scan, or skip |

The first two are the only free wins in the folder. Everything else in
`downloads/pioneers/` is already in the collection.

## Not held at all — the actual shopping list

Priority order for a devotional and sanctuary-facing corpus.

| Work | Author | Year | Why |
|---|---|---:|---|
| **The Cross and Its Shadow** | S.N. Haskell | 1914 | the single biggest hole: sanctuary typology, and the one Haskell title not held |
| The Atonement in the Light of Nature and Revelation | J.H. Waggoner | 1884 | **J.H. Waggoner is absent entirely**, and this is his major work |
| The Spirit of God: Its Offices and Manifestations | J.H. Waggoner | 1877 | |
| From Eden to Eden | J.H. Waggoner | 1888 | |
| Day-Star Extra, 7 February 1846 | O.R.L. Crosier | 1846 | short, and the origin of the Most Holy Place exposition |
| The Hiram Edson manuscript fragment | Hiram Edson | c.1850 | short, historically central |
| The Sanctuary and Twenty-three Hundred Days | J.N. Andrews | 1853 | Andrews' own, distinct from Smith's `S23D` |
| The Three Messages of Revelation XIV | J.N. Andrews | 1892 ed. | |
| The Opening Heavens | Joseph Bates | 1846 | four short Bates tracts, all foundational on the sanctuary and the sealing |
| Second Advent Way Marks and High Heaps | Joseph Bates | 1847 | |
| A Seal of the Living God | Joseph Bates | 1849 | |
| An Explanation of the Typical and Anti-typical Sanctuary | Joseph Bates | 1850 | |
| Prophetic Expositions, 2 vols | Josiah Litch | 1842 | Litch is absent entirely |
| The Probability of the Second Coming of Christ about A.D. 1843 | Josiah Litch | 1838 | |
| Memoirs of William Miller | Sylvester Bliss | 1853 | the source James White condensed into `SCLWM` |
| "Come Out of Her, My People" | Charles Fitch | 1843 | short sermon, Fitch is absent |
| The Church: Its Organization, Order and Discipline | J.N. Loughborough | 1907 | |
| The Saviour of the World | W.W. Prescott | 1929 | Prescott is absent; 1929 so US-PD, EU-PD from 2015 (d. 1944), verify |
| The Doctrine of Christ | W.W. Prescott | 1920 | verify |
| Questions and Answers | M.C. Wilcox | 1911 | verify |

Titles and years above come from general knowledge, not from a source in
hand. Confirm each against the scan before ingesting.

**Ready-to-run acquisition:** [ACQUISITION-BRIEF.md](ACQUISITION-BRIEF.md)
holds the agent prompts, the 24 reserved book codes (collision-checked
2026-09-20) and the import runbook for these works plus the two already on
disk but never catalogued.

## Periodicals and Bulletins — the largest prize, and the largest job

This is where most pioneer writing actually lives. None of it is in the
collection. All of it is on documents.adventistarchives.org as OCR'd PDFs.

| Run | Years | Notes |
|---|---|---|
| The Review and Herald | 1850-1915 | the main body of pioneer writing, including most EGW articles |
| The Signs of the Times | 1874-1915 | Waggoner and Jones as editors |
| The Youth's Instructor | 1852-1915 | |
| General Conference Bulletins | 1863-1915 | verbatim sermons, including the 1888, 1893 and 1895 sessions |
| Present Truth (Rochester) | 1849-1850 | the first paper, 11 issues |
| The Advent Review | 1850 | |
| The American Sentinel | 1886-1900 | Jones on religious liberty |
| The Bible Echo (Australia) | 1886-1915 | |
| Present Truth (UK) | 1884-1915 | |
| Sabbath School lesson quarterlies | pre-1915 | direct ancestors of SBL |

Practical notes: these are periodicals, not books, so the citation unit is
paper, volume, issue, page, article title and author. That needs a payload
shape the current `book_code` plus `page` model does not have. Decide the
schema before ingesting a single issue. Also, the OCR on the older scans is
poor and multi-column, exactly the `SDP` / `SSP` problem at a thousand times
the scale, so a quality gate is mandatory.

Suggested order: the General Conference Bulletins first. They are bounded,
they are the verbatim sermons, and the 1888, 1893 and 1895 sessions are the
most quoted material in the whole run.

## Appendix — the pioneers' own sources, tagged separately

Not Adventist, and never to be quoted as Adventist teaching, but directly
useful. Tag with a distinct `corpus` value, for example `sources`, so a
drafting agent can be told these are research-only.

| Work | Author | Year | Why |
|---|---|---|---|
| History of Protestantism, 3 vols | J.A. Wylie | 1878 | a documented source behind the Reformation chapters of *The Great Controversy* |
| History of the Reformation of the Sixteenth Century | J.H. Merle d'Aubigné | 1846 | the same |
| Book of Martyrs | John Foxe | 1563 | the same |
| Bible History Old Testament, 7 vols | Alfred Edersheim | 1876 | **indexed** `BHOTV1`-`BHOTV7` |
| The Temple: Its Ministry and Services | Alfred Edersheim | 1874 | sanctuary typology |
| The Life and Times of Jesus the Messiah | Alfred Edersheim | 1883 | gospel background |
| Antiquities of the Jews, Whiston translation | Josephus | 1737 tr. | background |
| The Holiest of All (Hebrews) | Andrew Murray | 1894 | Christ's intercession |
| Commentary on the Whole Bible | Matthew Henry | 1710 | verse-level help where EGW is silent |
| Commentary on the Bible | Adam Clarke | 1831 | the same |
| Notes on the New Testament | Albert Barnes | 1834 | the same |
| Commentary Critical and Explanatory | Jamieson, Fausset, Brown | 1871 | the same |

The four commentaries are the practical answer to a recurring problem in plan
work: a day built on a passage Ellen White never comments on, with nothing
under the writer but the verse.

---

## The title defect, fixed 2026-09-20

**Symptom.** 48 of the 49 imported pioneer works came back from
`sop_list_books(lang="en")` with `"titles": []`, so the whole shelf was
invisible to every agent and looked like it had never been imported.

**Cause.** `sop_list_books` does not read titles from Qdrant. It reads the
static table `qdrant/app/data/sop_books.json`, and
`local-archive/scripts/qdrant/export_book_titles.py` built that table **only** from the generator's
local mirror (`generator/data/sop/book_map.json` plus `en/<CODE>.json`). The
pioneers were indexed straight into Qdrant and have no files in that mirror,
so they got no entry. The import itself was fine: every point already carries
`title`, `author`, `year` and `corpus` in its payload.

**Fix.** `local-archive/scripts/qdrant/export_book_titles.py` now takes two further sources and
merges them over the mirror:

```bash
# offline, from the build artifact
python3 local-archive/scripts/qdrant/export_book_titles.py /workspaces/sdarm/local-archive/imported/sop-indexes \
    --merge local-archive/scripts/pd-books-pioneer-import/pioneers_corpus.jsonl

# or source-agnostic, from whatever is actually indexed
python3 local-archive/scripts/qdrant/export_book_titles.py /workspaces/sdarm/local-archive/imported/sop-indexes \
    --qdrant-url "$QDRANT_URL"
```

`--dry-run` reports the per-language delta and writes nothing. Re-run it after
**every** future import, from whichever pipeline.

`app/data/sop_books.json` has been regenerated: **618 English codes, zero
untitled**, with `author`, `year` and `corpus` recorded for the 49 pioneer
works. `sop_list_books` now returns those three fields (patched in both
`qdrant/app/sop_tools.py` and `local-archive/scripts/translator-stdio-mcp/sop_tools_mcp.py`, which are kept
identical) and matches on author, so `search="haskell"` resolves to `SDP` and
`SSP`. **No re-import is needed. The service has to be redeployed for it to
take effect.**

## Remaining repairs

1. ~~**`page_kind` is not surfaced.**~~ **Fixed 2026-09-20.** `sop_lookup`,
   `sop_book_paragraphs`, `sop_context` and `sop_by_bible_ref` now return
   `corpus`, `author` and `page_kind` on every non-EGW hit (helper `_prov` in
   `qdrant/app/sop_tools.py`, mirrored in `local-archive/scripts/translator-stdio-mcp/sop_tools_mcp.py`).
   Ellen White hits carry none of those keys and their output is unchanged.
   `page_kind: "print"` means `page.paragraph` is the printed reference and is
   citable; `page_kind: "chapter"` means `page` is a positional sequence
   number and must not be cited as a page. Live after redeploy.
2. **Page ranges do not start at 1.** `FCC` starts at page 7, `GSAM` at 3,
   `LSJW` at 4, `USLP` runs only 2-4, `GT` only 2-15. A `page_from=1` probe
   returns nothing and looks like a broken record. These books are fine.
3. **`WROM` duplicates `WOR`.** Both are Waggoner on Romans: `WOR` (1,829
   paragraphs) came with the EGW Estate mirror, `WROM` (2,735 points) from the
   pd-books import, which deliberately avoided the taken code. The Estate
   ebook's generic "About the Author: Ellen G. White" front matter is what made
   `WROM` look like an EGW work. Decide which edition wins and drop the other.
4. **`MND` and `SOD` are the same Uriah Smith book**, *The State of the Dead
   and the Destiny of the Wicked*, digitized twice. Deduplicate.
5. **Re-OCR `SDP` and `SSP`** (Haskell). Both open on scrambled two-column
   pages. The `--only <slug>` flag on `index_pioneers_qdrant.py` makes this a
   single-work re-run.
6. **`corpus` has no payload index yet.** `sop` carries indexes on `lang`,
   `page`, `book_code` and `bible_refs` but not `corpus`, so every
   corpus-filtered search is a full scan of 899,187 points. Create it:
   `python3 scripts/split_corpus.py index-payload --execute`.

## Splitting the collection

Measured 2026-09-20 against the live instance: **899,187 points, of which
59,965 are pioneers and 839,222 are Ellen White.**

`qdrant/scripts/split_corpus.py` (stdlib only, dry run by default) offers
`index-payload`, `count`, `copy` and `delete`. A copy reads vectors from the
source and writes them unchanged, so **nothing is re-embedded** — the 59,965
pioneer points are a network transfer of minutes, not an embedding run.

```bash
export QDRANT_URL=http://10.10.10.10:6333
python3 scripts/split_corpus.py index-payload --execute          # do this regardless
python3 scripts/split_corpus.py copy --to pioneers --pioneers    # dry run
python3 scripts/split_corpus.py copy --to pioneers --pioneers --execute
python3 scripts/split_corpus.py count --pioneers --collection pioneers
python3 scripts/split_corpus.py delete --pioneers --verify-in pioneers \
    --execute --yes-i-verified-the-copy
```

`copy` mirrors the `lang` / `book_code` / `page` / `corpus` payload indexes
onto the new collection. `delete` refuses unless `--verify-in` names a
collection that already holds at least as many matching points.

**Recommendation: do not split.** EGW points carry no `corpus` key at all, so
`must_not corpus=pioneers` already gives an exact EGW-only search, and once
`corpus` is indexed it costs nothing. One collection keeps a single ranked
result list across both corpora, needs no change to `_COLLECTION` in
`sop_tools.py`, and needs no second query and hand-written merge.

The one argument for splitting is governance rather than performance: a
filter is a promise in code, a separate collection is a promise in
infrastructure. If the rule "a Spirit-of-Prophecy query must never be able to
reach a pioneer paragraph" needs to hold even when someone forgets the
filter, split. Otherwise index `corpus` and move on.

## Ingestion cautions

- `build_sop_vector_index --lang en --resume`. **Never `--recreate`**: it drops
  the German corpus.
- Point IDs are `uuid5(lang:code:para_key)`. Re-ingesting a book under a
  `book_code` that already exists will silently overwrite curated text with
  duplicate lower-quality text wherever paragraph keys coincide. Check the code
  is free before every run.
- Page numbers must be the printed page of the edition actually ingested, since
  that is what gets cited.
- Check a sample page of OCR before committing a book to the collection.

## Open decision for the user

The collection is named `sop` and the tooling calls it "Spirit of Prophecy".
Once the pioneers are a deliberate part of it, the name is wrong and, worse,
it invites an agent to quote Uriah Smith as though he carried the same
authority as Ellen White. Either rename to SoP+ and make the `corpus` tag
load-bearing in every skill, or keep two collections. This should be settled
before the first bulk ingestion, not after.
