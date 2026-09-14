# bible-sop tools — reference

Six read-only tools. Tool names may carry a connector prefix in your environment
(e.g. `mcp__bible-sop__sop_lookup`); the bare names are used here.

## Bible

### `bible_lookup(ref, bible?, numbering="edition")`

Verse text by OSIS reference. One call covers everything below; never loop verse by verse.

| `ref` form | Example |
|---|---|
| verse | `John.3.16` |
| range in a chapter | `John.3.16-18` |
| range across chapters | `John.3.36-John.4.2` |
| whole chapter | `Ps.23` |
| list (`;` or `,`) | `Gen.1.1; Rom.8.28-30` |

- `bible`: one name or a list (`["luther1912","schlachter"]`). Omit for every translation.
- `numbering`: `"edition"` reads verse numbers in each translation's own system;
  `"kjv"` reads them as KJV/English numbers and remaps per translation. See
  [VERSIFICATION.md](VERSIFICATION.md).
- Returns `{"verses": [{osis, bible, text, kjv_osis?}], "not_found"?: [ref]}`,
  ordered by reference, then by the order of `bible`. A ref in `not_found` matched
  nothing in any requested translation.

OSIS book names: `Gen Exod Lev Num Deut Josh Judg Ruth 1Sam 2Sam 1Kgs 2Kgs 1Chr 2Chr
Ezra Neh Esth Job Ps Prov Eccl Song Isa Jer Lam Ezek Dan Hos Joel Amos Obad Jonah Mic
Nah Hab Zeph Hag Zech Mal Matt Mark Luke John Acts Rom 1Cor 2Cor Gal Eph Phil Col
1Thess 2Thess 1Tim 2Tim Titus Phlm Heb Jas 1Pet 2Pet 1John 2John 3John Jude Rev`.

### `bible_search(query, bible?, limit=5, min_score=0.5)`

Semantic search: find a verse from its wording or theme when the reference is
unknown. Query in the language of the `bible` you search. The top result is a
**candidate**. Confirm it by reading the text, then fetch it with `bible_lookup`
before quoting.

### `bible_list_translations()`

`{"translations": [{bible, verses, kjv_remapped}]}`. Call it before claiming a
translation is or is not available.

## Spirit of Prophecy (Ellen G. White)

### `sop_lookup(query | queries, codes?, lang="de", min_score=0.55, limit=1)`

Semantic paragraph search in one language's corpus.

- **Pass 1** is restricted to `codes` (if given) at `min_score`. If it finds nothing,
  **Pass 2** runs corpus-wide at 0.50 and fills `fallbacks`.
- `query`: the quotation's text **without** its citation suffix. Query in the
  **target language** when you have wording in it. A query in another language
  still works, but scores lower.
- `codes`: book codes in *that language's* code system (see `sop_list_books`).
- `limit`: 3 is a good default when choosing between editions or compilations.
- **Batch:** `queries=[str | {"query", "codes"}]`, up to 50 items, returns
  `{"results": [{query, hits, fallbacks}]}` in input order. Use it whenever you have
  more than one quotation.
- Each hit: `{book_code, page, para_key, score, text}`. `para_key` is `page.paragraph`.

### `sop_book_paragraphs(book_code, page_from, page_to?, lang="de")`

Exact paragraphs of one book and page range, ordered by page and paragraph. Use it
to read the context around a hit, or to fetch a cited page directly. Over ~400
paragraphs the result stops at a page boundary with `truncated: true` and
`next_page_from`.

### `sop_list_books(lang?, search?)`

- No arguments: languages with paragraph counts.
- `lang`: that language's books with `titles`, and `en_code` / `en_titles` for
  translated editions.
- `search`: a case-insensitive match on code or title in any language
  (`"Steps to Christ"`, `"Messias"`). Without `lang` it searches every language at
  once, which is the fastest way to find a work's codes everywhere.

## Reading scores (`multilingual-e5-large`)

Scores are compressed high. **Unrelated text still scores about 0.80**, so a
threshold alone never proves a match.

| Score | Meaning (measured) |
|---|---|
| ≥ 0.90 | Usually the passage itself, or a compilation reprinting it |
| 0.85 – 0.90 | Often right, but a paraphrase on a related theme lands here too |
| < 0.85 | Treat as **no match** unless the text plainly says the same thing |

**The deciding test is always the text.** Read the returned paragraph and confirm
that it contains the quoted sentence (or its exact meaning, for another language)
before you use it.

## Compilations repeat sentences

The corpus holds original books **and** compilations and devotionals that reprint
them (`Pr` *Prayer*, `PM`, `FLB`, `HP`, `ML`, `OHC`, `TMK`, `LHU`, …). The same
sentence can return 3–6 hits with near-identical scores. Prefer the **original
work** the citation names: restrict with `codes`, or pick the hit whose
`book_code` matches the citation. Cite a compilation only when the source says so.
