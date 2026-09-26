---
name: corpus-lookup
description: Retrieve Bible verses and Ellen G. White (Spirit of Prophecy) paragraphs verbatim from the bible-sop corpus, with correct verse numbering, book codes and page citations, in as few tool calls as possible. Use whenever you are about to quote Scripture or Ellen White, look up a reference, find where a passage appears, read the context around a passage, or check which translations and books are available.
---

# Corpus lookup: Bible and Ellen G. White

Every quotation you present as Scripture or as Ellen G. White comes from the
**bible-sop** tools, verbatim, with the reference the tool returned. The tools
and the **call-economy table** are in [references/TOOLS.md](references/TOOLS.md).
Read it before your first lookup in a conversation. Consult
[references/COVERAGE.md](references/COVERAGE.md) (what exists, per language) and
[references/VERSIFICATION.md](references/VERSIFICATION.md) (verse numbers) when a
question touches them.

## Non-negotiable

1. **Retrieve, never recall.** Don't write a verse or an Ellen White sentence
   from memory, and don't "tidy" retrieved wording. Spelling, punctuation and
   archaic forms stay as returned.
2. **Never translate a quotation yourself when the corpus may hold the edition.**
   Look it up in the target language first (see the `quote-translate` skill).
3. **No tool, no quote.** If the tools are unavailable or return an error, give
   only the reference. Say the wording still needs to be looked up, and name the
   tool that failed. Don't fall back to memory or web text without saying so.
4. **The number must match the words.** Print the verse or page number of the
   edition whose words you print.
5. **Say who wrote it.** A hit that carries `corpus: "pioneers"` was written by
   the `author` it names (Uriah Smith, A. T. Jones, …), not by Ellen White.
6. **Report what the corpus cannot prove.** An empty or weak result means "not
   found in this corpus". It never means "does not exist".

## Plan the calls, then make them

Before calling anything, list what you need: every verse, every EGW sentence,
every language. Then group it:

- **All Bible text → one `bible_lookup`**: a `;`-list of refs, with a list for
  `bible`.
- **All EGW sentences in one language → one `sop_lookup(queries=[…], lang=…)`.**
- **Work codes → one `sop_list_books(search="<title>")`** without `lang`. It
  returns the code in every language.

A typical request takes two or three calls. If you are making a tenth, stop
and batch what is left.

## Procedure

**Bible**
1. Known reference → `bible_lookup(ref, bible=[…])`. Fetch ranges and several
   translations in **one** call.
2. The reference is English/KJV and you quote another translation →
   `numbering="kjv"`, and print the returned `osis`. Check the quirks in
   VERSIFICATION.md (Ohienko Psalms, Synodal Daniel, translations without a
   remap table).
3. Unknown reference → `bible_search(query, bible=<one translation>)` in that
   translation's language. Confirm the words, then `bible_lookup` the hit.

**Ellen G. White**
1. Resolve the work: `sop_list_books(search="<title or code>")`. German has its
   own codes and often two editions. Skip this step if no work is named.
2. Known sentence → `sop_lookup(query=<sentence without citation>, codes=[…],
   lang=<language>, limit=1)`. Raise `limit` to 3 if compilations or a second
   edition compete. **Always pass `lang`.**
3. Known page → `sop_book_paragraphs(book_code, page_from, page_to, lang)`.
4. **Judge the hit by its text, not its score.** Unrelated text scores ≈0.80.
   Prefer the original book over compilations that reprint it.
5. Need more of the passage (the quotation crosses a paragraph break, or you
   want context) → `sop_context(book_code, para_key, lang, before, after)`.
   Don't fetch a page range for this.
6. Have a German paragraph and need the English one, or the reverse →
   `sop_parallel(book_code, para_key, lang, target_lang)`. Don't run a new search.

## Citing

- Bible: the book name and number in the language of the output, taken from the
  retrieved edition (`Psalm 51,3` for Luther in German style, `Psalm 51:1` KJV).
  Name the translation when the audience might assume a different one.
- Ellen White: the edition's title (or the conventional abbreviation for that
  language) plus the **page returned for that language**: *Steps to Christ*,
  p. 93 / SC 93.2; *Der Weg zu Christus*, S. 70. `para_key` `93.2` means page 93,
  paragraph 2.
- Pioneer works: author and title. Cite a page only when `page_kind` is `print`.
  A `chapter` value is a sequence number, not a page.
- Never attach a page from one language's edition to another language's text.

## When something is missing

Say so plainly and offer the next best step: a reference without quotation,
another translation or edition that *is* indexed, or reported speech without
quotation marks. Leave the choice to the user.
