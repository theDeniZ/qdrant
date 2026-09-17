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

## Spirit of Prophecy — Ellen G. White only

The `sop` corpus contains **Ellen G. White** writings and nothing else. No
E. J. Waggoner, A. T. Jones, Uriah Smith or other pioneers. An empty result for
another author's sentence means "not in this corpus". It does **not** mean
"no published translation exists".

| lang | paragraphs | books | notes |
|---|---:|---:|---|
| en | 390,905 | 606 | full EGW Estate corpus, incl. letters/manuscripts compilations |
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

**`sop_by_bible_ref` depends on a backfill that has not run yet.** Finding
which EGW paragraphs cite a given verse needs a `bible_refs` payload field
built by a one-off offline extraction; most of the corpus does not carry it
yet. Until that backfill runs, the tool returns an actionable error, not an
empty result — do not read that error as "no paragraph quotes this verse".

## Pagination is per language

`page` is the page of **that language's edition**. Romanian tracks English
pagination; Japanese and Spanish do not (English PP 139 is Spanish PP 118).
Always cite the page the lookup returned for the language you quote. Never carry
an English page number into another language's citation.
