# bible-sop tools — reference

Nine read-only tools. Tool names may carry a connector prefix in your environment
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

### `sop_context(book_code, para_key, lang="en", before=2, after=2)`

Neighbouring paragraphs around one already-known paragraph — cheaper than
`sop_book_paragraphs` when you only need a few paragraphs of context around a
verified hit, not a whole page range.

- `para_key`: the anchor paragraph's `"PAGE.PARA"` key (from a `sop_lookup` hit
  or `sop_book_paragraphs` row).
- `before` / `after`: paragraphs to include on each side. Default 2; clamp
  0..20. Fewer than requested near a book's first or last page.
- Returns `{"context": [{book_code, page, para, para_key, text, is_target}, ...]}`,
  ordered by page then paragraph, with exactly one entry carrying
  `is_target: true`. `{"error": "..."}` if `para_key` isn't found for that
  `book_code`/`lang`.

### `sop_parallel(book_code, para_key, lang="de", target_lang="en")`

Paragraph-level de↔en alignment: given one paragraph, returns its counterpart
paragraph(s) in the other language, retrieved verbatim rather than translated.
This is the "retrieve, don't translate" path for a quotation that already has
a published edition on the other side — prefer it over machine or hand
translation whenever `lang`/`target_lang` is a de↔en pair.

- **Only de↔en is available.** Any other `(lang, target_lang)` pair —
  including `ja` or `ko`, which carry no alignment data in the corpus at all —
  returns `{"error": "..."}` rather than a guessed or silently-empty result.
  Never treat that error as "no counterpart exists"; it means the alignment
  isn't in this corpus for that language pair.
- `para_key`: the paragraph's `"PAGE.PARA"` key in the `lang` edition.
- Returns `{"source": {...}, "target": [{...}, ...]}` — `target` can hold more
  than one paragraph (a single paragraph sometimes maps to several on the
  other side), or `[]` with a `"note"` if the source paragraph carries no
  alignment. `{"error": "..."}` if the language pair is unsupported or the
  source paragraph doesn't resolve.

### `sop_by_bible_ref(osis, lang="en", limit=20)`

Find SoP paragraphs that quote or cite a given Bible verse.

- `osis`: a single exact OSIS reference (e.g. `"John.3.16"`) — no range
  expansion; a paragraph citing `"John.3.16-18"` is only found by that exact key.
- `limit`: default 20; clamp 1..200.
- **The `bible_refs` backfill has run.** 69,527 paragraphs carry the payload
  field and are searchable. 120 paragraphs in 9 never-vectorised books
  (`4aSG, 4bSG, PH045, PH083, PH088, PH141, PH153, Te-SG, TithPG`) have no
  vector-index point at all and stay invisible to this tool — that is a
  separate `build_sop_vector_index` gap, not a backfill state.
- **`bible_refs` numbering follows the edition parsed, not KJV.** A German
  paragraph citing a KJV-numbered verse is indexed under Luther/Masoretic
  numbering (KJV `Ps.51.1` → indexed `Ps.51.3`). Querying with a raw
  KJV-numbered `osis` (e.g. an SBL lesson's `sOsis`) against `lang="de"`
  will miss or mis-hit for the ~63 offset Psalms and ~40 OT
  chapter-boundary shifts — remap first (see
  [VERSIFICATION.md](VERSIFICATION.md)) or query `lang="en"`, where KJV
  numbering holds. Extraction is text-pattern based and occasionally
  produces a spurious extra ref (a stray trailing digit after a citation) —
  treat `bible_refs` as a high-recall discovery index and confirm each hit
  by reading the paragraph, not as a curated citation list.
- Returns
  `{"results": [{book_code, page, para, para_key, text, bible_refs}, ...]}`.

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
