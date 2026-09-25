# bible-sop corpus — what is (and is not) in it

Figures are from September 2026. Call `bible_list_translations()` /
`sop_list_books()` for the live state; never assume from this table alone.

## Bible translations

| `bible` | Edition | Language | Numbering in the index |
|---|---|---|---|
| `kjv` | King James Version | en | KJV |
| `nkjv` | New King James Version | en | KJV |
| `net` | NET Bible (27,727 verses, incomplete) | en | KJV |
| `luther1912` | Luther 1912 | de | Hebrew OT (remapped with `numbering="kjv"`) |
| `schlachter` | Schlachter 1951 | de | Luther-like; **no remap table**, verify Psalms |
| `elberfelder1905` | Elberfelder 1905 | de | **no remap table**, verify Psalms |
| `synodal` | Russian Synodal | ru | mixed; see [VERSIFICATION.md](VERSIFICATION.md) |
| `ukrogienko` | Ohienko | uk | edition-numbered throughout |
| `spanish` | Reina-Valera | es | verify Psalms / Jonah |
| `japkougo` | 口語訳 (1955) | ja | verify |
| `korean` | 개역한글 (KRV 1961, *not* 개역개정) | ko | verify |

**No Romanian Bible.** For ro, the Romanian SoP corpus quotes Scripture often:
`sop_lookup(lang="ro")` on the verse text often recovers the Cornilescu wording.
Say so when you do this.

An absent Bible **is** provable: `bible_lookup(ref)` without `bible` returns
every translation that has the verse.

## Spirit of Prophecy — Ellen G. White, plus a pioneer shelf

The `sop` corpus is mostly **Ellen G. White**, but it is *not* EGW only.
(Corrected September 2026; this section previously said the opposite and was
wrong.) It also holds **49 pre-1915 Adventist pioneer works** by 11 authors,
imported August 2026, about 60,000 points and 4.69 M words:

| Author | Works | Codes |
|---|---:|---|
| A. T. Jones | 11 | `CWCP` `EB` `ECE` `EMTF` `GEP` `GNT` `NSLS18` `PBE` `ROP` `TTL` `TTR` |
| E. J. Waggoner | 9 | `CHR` `EVCO` `FCC` `GBG` `GOSC` `GT` `PROLI` `SOOCC` `WROM` |
| Uriah Smith | 9 | `DAR` `LUJ` `MND` `MON` `MSp` `S23D` `SOD` `SYPT` `USLP` |
| James White | 4 | `HGA` `LI` `LSJW` `SCLWM` |
| William Miller | 4 | `AAD` `EFS` `RTS` `VOP` |
| J. N. Andrews | 3 | `CTF` `HSFD` `WDYS` |
| Joseph Bates | 3 | `BAB` `SDSPS` `VSDS` |
| J. N. Loughborough | 2 | `GSAM` `RPSDA` |
| S. N. Haskell | 2 | `SDP` `SSP` |
| D. M. Canright | 1 | `MAS` |
| G. I. Butler | 1 | `COS` |

Every pioneer point carries `corpus: "pioneers"`, `author`, `title` and
`year`; EGW points carry no `corpus` key at all, so `must_not corpus=pioneers`
is the EGW-only filter. `sop_list_books` returns `author`, `year` and
`corpus`, and matches on author, so `search="haskell"` works.

**Quoting a pioneer is not quoting the Spirit of Prophecy.** Check `corpus`
before attributing a hit, and say whose words they are.

Three cautions:

- **`page_kind`, returned on every non-EGW hit.** `print` means the printed
  `page.paragraph` reference came from the text and is a real citation.
  `chapter` means `page` is a positional sequence number and is **not** a
  printed page: cite the work, not a page number. EGW hits carry no
  `page_kind`; their `page` is always a printed page.
- **Page ranges do not start at 1** for several works (`FCC` from 7, `GSAM`
  from 3, `LSJW` from 4, `USLP` only 2-4). A `page_from=1` probe returning
  nothing does not mean the book is missing.
- **OCR.** `SDP` and `SSP` (Haskell) open on scrambled two-column pages. Check
  any Haskell hit before quoting it. `WROM` duplicates `WOR` and `MND`
  duplicates `SOD`.

**The pioneer shelf is English only.** `DAR` in `de` returns nothing at any
page, so a pioneer quotation has no canonical wording in de/ru/uk.

An empty result for an *Ellen White* sentence still means "not in this
corpus", and still does **not** mean "no published translation exists".

Planned additions and repairs: [CORPUS-ACQUISITION.md](CORPUS-ACQUISITION.md).

| lang | paragraphs | books | notes |
|---|---:|---:|---|
| en | 390,905 | 606 | EGW Estate corpus (letters/manuscripts compilations included) **plus the 49 pioneer works above** |
| de | 104,993 | 92 | **own DE codes** (`BW`, `WZC`, `DM`, `GK` …), several editions per work |
| es | 67,784 | 49 | English codes |
| pt | 63,050 | 47 | English codes |
| ru | 62,352 | 49 | English codes |
| ko | 59,296 | 44 | English codes, all nine *Testimonies* |
| ro | 47,908 | 38 | English codes |
| fr | 42,947 | 32 | English codes |
| it | 33,704 | 24 | English codes |
| ja | 12,426 | 8 | AA COL DA GC MB PK PP SC only |
| uk | 11,209 | 7 | AA COL DA GC PP SC SR only; no 1888-era sources |
| zh | 2,613 | 1 | CD only |

**Book codes.** German uses its own codes, and one English work can have several
German editions (Steps to Christ → `BW` and `WZC`; Desire of Ages → `DM` and `LJ`).
Every other language uses the **English** code (`SC`, `DA`, `GC`). Resolve codes
with `sop_list_books(search="<title>")`; do not guess.

**Coverage is per language and per book.** A language being present does not
mean the book you need is present. The real negative signal is: unrelated hits
(below ~0.85, or text that says something else) **and** no plausible
`fallbacks` after querying in that language.

**Paragraph-level alignment (`sop_parallel`) is de↔en only.** The corpus
carries a paragraph-to-paragraph cross-reference between the German and
English editions of the same work, which is what lets you *retrieve* a
published counterpart instead of translating one. No other language pair is
aligned — in particular **ja and ko carry no alignment data at all**, so
`sop_parallel` returns an explicit error for them rather than a guess.

**`sop_by_bible_ref` is live.** The `bible_refs` backfill has run: 69,527 SoP
paragraphs carry the payload field and are searchable by verse. 120
paragraphs are still invisible to it because they have no vector-index point
at all (9 never-indexed books: `4aSG, 4bSG, PH045, PH083, PH088, PH141,
PH153, Te-SG, TithPG`) — that gap needs a `build_sop_vector_index` run for
those books, not another backfill.

**`bible_refs` are numbered in the edition that was parsed, not in KJV.**
The extractor read each paragraph's own citation, so a German paragraph
quoting a KJV-numbered verse is indexed under the **Luther/Masoretic**
number for that verse (KJV Ps.51.1-2 → indexed as `Ps.51.3`, `Ps.51.4`).
Querying `sop_by_bible_ref(osis=…, lang="de")` with the KJV-numbered `sOsis`
straight from an SBL lesson will miss or mis-hit for the ~63 offset Psalms
and ~40 OT chapter-boundary shifts — remap the ref to the target edition's
numbering first (see [VERSIFICATION.md](VERSIFICATION.md)), or query
`lang="en"` where KJV numbering holds throughout. Extraction is also
text-pattern based, so an occasional spurious ref slips in (a stray trailing
digit after a citation can register as its own verse) — treat `bible_refs`
as a high-recall discovery index and confirm every hit by reading the
paragraph, not as a curated citation list.

## Pagination is per language

`page` is the page of **that language's edition**. Romanian tracks English
pagination; Japanese and Spanish do not (English PP 139 is Spanish PP 118).
Always cite the page the lookup returned for the language you quote. Never carry
an English page number into another language's citation.
