# bible-sop tools — reference

Nine read-only tools. Tool names may carry a connector prefix in your environment
(e.g. `mcp__bible-sop__sop_lookup`); the bare names are used here.

## Call economy (read this first)

Every call is a network round trip, and most answers need two or three calls,
not ten. Use these rules to keep the count low:

| Instead of | Do |
|---|---|
| one `bible_lookup` per verse or per translation | one call: `ref="Gen.1.1; Rom.8.28-30"`, `bible=["kjv","luther1912"]` |
| one `sop_lookup` per sentence | one call with `queries=[…]` (up to 50) |
| `sop_list_books(lang=…)` once per language | one `sop_list_books(search="<title>")` with no `lang`: every language's codes at once |
| `sop_book_paragraphs` over a page range to see a hit's surroundings | `sop_context(book_code, para_key, lang, before, after)` |
| a fresh `sop_lookup` in German/English for a paragraph you already hold in the other language | `sop_parallel` (de↔en only) |
| `bible_search` over all 11 translations | `bible_search(query, bible=<one>)` in that translation's language |
| re-listing books or translations | keep what `sop_list_books` / `bible_list_translations` returned for the rest of the conversation |

**Always pass `lang` to the `sop_*` tools.** `sop_lookup` and `sop_parallel`
default to German (`lang="de"`) and `sop_context` / `sop_by_bible_ref` default to
English. Leaving `lang` out gives you the wrong language's text without any error.

## Bible

### `bible_lookup(ref, bible?, numbering="edition")`

Verse text by OSIS reference. One call covers everything below. Never loop verse by verse.

| `ref` form | Example |
|---|---|
| verse | `John.3.16` |
| range in a chapter | `John.3.16-18` |
| range across chapters | `John.3.36-John.4.2` |
| whole chapter | `Ps.23` |
| list (`;` or `,`) | `Gen.1.1; Rom.8.28-30` |

- `bible`: one name or a list (`["luther1912","schlachter"]`). Omit it to get every translation.
- `numbering`: `"edition"` reads verse numbers in each translation's own system.
  `"kjv"` reads them as KJV/English numbers and remaps them per translation. See
  [VERSIFICATION.md](VERSIFICATION.md).
- Returns `{"verses": [{osis, bible, text, kjv_osis?}], "not_found"?: [ref]}`,
  ordered by reference, then by the order of `bible`. A ref in `not_found` matched
  nothing in any requested translation.

OSIS book names: `Gen Exod Lev Num Deut Josh Judg Ruth 1Sam 2Sam 1Kgs 2Kgs 1Chr 2Chr
Ezra Neh Esth Job Ps Prov Eccl Song Isa Jer Lam Ezek Dan Hos Joel Amos Obad Jonah Mic
Nah Hab Zeph Hag Zech Mal Matt Mark Luke John Acts Rom 1Cor 2Cor Gal Eph Phil Col
1Thess 2Thess 1Tim 2Tim Titus Phlm Heb Jas 1Pet 2Pet 1John 2John 3John Jude Rev`.

### `bible_search(query, bible?, limit=5, min_score=0.5)`

Semantic search. Use it to find a verse from its wording or theme when you don't
know the reference. Query in the language of the `bible` you search. The top result is a
**candidate**. Read its text to confirm it, then fetch it with `bible_lookup` before quoting.

### `bible_list_translations()`

`{"translations": [{bible, verses, kjv_remapped}]}`. Call it before claiming a
translation is or is not available.

## Spirit of Prophecy (Ellen G. White, plus a pioneer shelf)

Every SoP hit carries `book_code`, `page`, `para_key` (`page.paragraph`) and `text`.
Hits from **non-EGW** works also carry `corpus: "pioneers"`, `author` and
`page_kind`. Check these before attributing a hit (see [COVERAGE.md](COVERAGE.md)).

### `sop_lookup(query | queries, codes?, lang="de", min_score=0.55, limit=1)`

Semantic paragraph search in one language's corpus.

- **Pass 1** is restricted to `codes` (if given) at `min_score`. If it finds nothing,
  **Pass 2** runs corpus-wide at 0.50 and fills `fallbacks`.
- `query`: the text to find, **without** its citation suffix. When you have
  wording in the target language, query in that language. A query in another
  language still works, but scores lower.
- `codes`: book codes in *that language's* code system (see `sop_list_books`).
- `limit`: use 1 when you are confirming a known quotation, 3 when you are choosing between
  editions or compilations, and 5–10 for topical research. The maximum is 20.
- **Batch:** `queries=[str | {"query", "codes"}]`, up to 50 items. It returns
  `{"results": [{query, hits, fallbacks}]}` in input order, and a top-level `codes`
  is the default for items that give none. Use it whenever you have more than one query.

### `sop_context(book_code, para_key, lang="en", before=2, after=2)`

Returns the paragraphs around one paragraph you already have. It is the cheap way to see
what comes before and after a hit, to finish a quotation that runs past a paragraph
break, or to check a hit in its context.

- `before` / `after`: 0–20 each. Near the start or end of a book you may get fewer.
- Returns `{"context": [{book_code, page, para, para_key, text, is_target}]}` in
  reading order, with one entry marked `is_target: true`. If `para_key` is not in
  that book/language, it returns `{"error": …}`.

### `sop_parallel(book_code, para_key, lang="de", target_lang="en")`

Retrieves the **published counterpart** of one paragraph in the other language,
using paragraph-level alignment. Prefer it to a fresh search whenever the
pair is German↔English.

- **de↔en only.** Any other pair returns an error. That error means the corpus has
  no alignment for the pair. It does not mean the counterpart doesn't exist.
- `book_code` / `para_key` are in the **source** (`lang`) edition. German uses its
  own codes (`BW`), and the result names the target code (`SC`).
- Returns `{"source": {…}, "target": [{…}]}`. `target` can hold several paragraphs,
  or be `[]` with a `note` when that paragraph has no alignment. When that happens,
  fall back to `sop_lookup` in the target language.

### `sop_by_bible_ref(osis, lang="en", limit=20)`

Returns paragraphs that quote or cite one Bible verse. Use it to answer "what does
Ellen White say about this verse?"

- `osis`: **one exact verse** (`John.3.16`). It does not expand ranges. For a passage,
  make one call per key verse.
- Verse keys follow **the numbering of that language's edition**, not KJV. For
  `lang="en"` use KJV numbers. For `lang="de"` use the Luther number (KJV `Ps.51.1`
  is `Ps.51.3` in German, see [VERSIFICATION.md](VERSIFICATION.md)), or query
  English and move to German with `sop_parallel`.
- The index is built by pattern-matching the citations in the text. It finds a lot,
  but it can include a stray wrong verse. Read every paragraph before you use it.
- `limit`: 1–200. Results are **not** ranked by relevance. Scan them and pick.
- Returns `{"results": [{book_code, page, para, para_key, text, bible_refs, …}]}`.

### `sop_book_paragraphs(book_code, page_from, page_to?, lang="de")`

Returns exact paragraphs of one book and page range, in reading order. Use it to
fetch a **cited page**. For context around a hit, use `sop_context` instead. Over
~400 paragraphs the result stops at a page boundary with `truncated: true` and
`next_page_from`.

### `sop_list_books(lang?, search?)`

- No arguments: the languages, with paragraph counts.
- `lang`: that language's books with `titles`, and `en_code` / `en_titles` for
  translated editions. The list is long, so prefer `search`.
- `search`: a case-insensitive match on code, title or author in any language
  (`"Steps to Christ"`, `"Messias"`, `"waggoner"`). Without `lang` it searches every
  language at once. That is the fastest way to find a work's codes everywhere.

## Reading scores (`multilingual-e5-large`)

Scores are compressed high. **Unrelated text still scores about 0.80**, so a
threshold alone never proves a match.

| Score | Meaning (measured) |
|---|---|
| ≥ 0.90 | Usually the passage itself, or a compilation reprinting it |
| 0.85 – 0.90 | Often right, but a paraphrase on a related theme lands here too |
| < 0.85 | Treat as **no match** unless the text plainly says the same thing |

**The text always decides.** Read the returned paragraph and confirm that it
contains the quoted sentence before you use it. For another language, confirm
that it says exactly the same thing.

For topical research the scale works differently. You are looking for paragraphs *about* a theme, so
0.82–0.88 hits can be exactly right. Judge them by what they say.

## Compilations repeat sentences

The corpus holds original books **and** compilations and devotionals that reprint
them (`Pr` *Prayer*, `PM`, `FLB`, `HP`, `ML`, `OHC`, `TMK`, `LHU`, …). The same
sentence can return 3–6 hits with near-identical scores. Prefer the **original
work** the citation names: restrict with `codes`, or pick the hit whose
`book_code` matches the citation. Cite a compilation only when the source cites it.
