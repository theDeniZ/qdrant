---
name: corpus-lookup
description: Retrieve Bible verses and Ellen G. White (Spirit of Prophecy) paragraphs verbatim from the bible-sop corpus, with correct verse numbering, book codes and page citations. Use whenever you are about to quote Scripture or Ellen White, look up a reference, find where a passage appears, or check what translations and books are available.
---

# Corpus lookup: Bible and Ellen G. White

Every quotation you present as Scripture or as Ellen G. White comes from the
**bible-sop** tools, verbatim, with the reference the tool returned. The tools
are described in [references/TOOLS.md](references/TOOLS.md). Read it before your
first lookup in a conversation. Consult
[references/COVERAGE.md](references/COVERAGE.md) (what exists, per language) and
[references/VERSIFICATION.md](references/VERSIFICATION.md) (verse numbers) when a
question touches them.

## Non-negotiable

1. **Retrieve, never recall.** Do not write a verse or an Ellen White sentence
   from memory, and do not "tidy" retrieved wording. Spelling, punctuation and
   archaic forms stay as returned.
2. **Never translate a quotation yourself when the corpus may hold the edition.**
   Look it up in the target language first (see the `quote-translate` skill).
3. **No tool, no quote.** If the tools are unavailable or error, give the
   reference only, say the wording still needs to be looked up, and name the tool
   that failed. Do not fall back to memory or web text silently.
4. **The number must match the words.** Print the verse or page number of the
   edition whose words you print.
5. **Report what the corpus cannot prove.** An empty or weak result means "not
   found in this corpus". It never means "does not exist". The corpus is
   Ellen White only.

## Procedure

**Bible**
1. Known reference → `bible_lookup(ref, bible=[…])`. Fetch ranges and several
   translations in **one** call.
2. The reference is English/KJV and you quote another translation →
   `numbering="kjv"`, and print the returned `osis`. Check the quirks in
   VERSIFICATION.md (Ohienko Psalms, Synodal Daniel, unmapped translations).
3. Unknown reference → `bible_search(query, bible)` in that translation's
   language. Confirm the words, then `bible_lookup` the hit.

**Ellen G. White**
1. Resolve the work: `sop_list_books(search="<title or code>")` gives the codes
   per language. German has its own codes and often two editions.
2. Known sentence → `sop_lookup(query=<sentence without citation>, codes=[…],
   lang=<language>, limit=3)`. Use `queries=[…]` for more than one sentence.
3. Known page → `sop_book_paragraphs(book_code, page_from, page_to, lang)`.
4. **Judge the hit by its text, not its score** (unrelated text scores ≈0.80).
   Prefer the original book over compilations that reprint it.
5. Read the neighbouring paragraphs when the quotation runs across a paragraph
   boundary or you need context: `sop_context(book_code, para_key, lang,
   before, after)` for a few paragraphs either side of a hit you already have
   (cheaper — prefer it over pulling a whole page range), or
   `sop_book_paragraphs` for an explicit page range.
6. Which paragraphs discuss a given verse → `sop_by_bible_ref(osis, lang,
   limit)`. It needs an offline backfill that has not run everywhere yet — a
   `{"error": ...}` means "cannot answer yet" for that corpus, **not** "no
   paragraph quotes this verse"; say so rather than reporting an empty result.

## Citing

- Bible: book name and number in the language of the output, taken from the
  retrieved edition (`Psalm 51,3` for Luther in German style, `Psalm 51:1` KJV).
  Name the translation when the audience could assume a different one.
- Ellen White: the edition's title (or the conventional abbreviation for that
  language) plus the **page returned for that language**: *Steps to Christ*,
  p. 93 / SC 93.2; *Der Weg zu Christus*, S. 70. `para_key` `93.2` means page 93,
  paragraph 2.
- Never attach a page from one language's edition to another language's text.

## When something is missing

State it plainly and offer the next best step: a reference without quotation,
another translation or edition that *is* indexed, or reported speech without
quotation marks. Leave the choice to the user.
