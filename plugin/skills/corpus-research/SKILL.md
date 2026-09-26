---
name: corpus-research
description: Research what Scripture and Ellen G. White say on a topic, or what Ellen White writes about a given Bible verse, using the bible-sop corpus with batched multi-angle queries, and present verbatim, cited findings grouped by theme. Use when the user asks what the Bible or Ellen White teaches about a subject, wants supporting passages for a study, sermon or article, asks "what does EGW say about <verse>", or wants to explore a theme across Scripture and the Spirit of Prophecy.
---

# Topical research in the Bible and Ellen G. White

Read [references/TOOLS.md](references/TOOLS.md) first, especially its call-economy
table and the note on scores for topical research. Consult
[references/COVERAGE.md](references/COVERAGE.md) for what each language holds,
and [references/VERSIFICATION.md](references/VERSIFICATION.md) before quoting any
non-KJV Old Testament verse.

The `corpus-lookup` rules still apply: retrieve, never recall; the number must
match the words; name the author of every hit; and an empty result means "not
in this corpus".

## 1. Frame the question (no tool calls)

- **Language** of the answer. Research in **English** first, where the corpus is
  largest (≈390k EGW paragraphs). Move the results you keep into the user's language at
  the end (step 5).
- **Scope**: Bible, Ellen White, or both. Include pioneer authors only if the user
  asks for them. Otherwise drop `corpus: "pioneers"` hits.
- **Four to eight search angles.** Semantic search matches *wording*, so one
  abstract query ("faith") finds little. Write each angle the way the sources
  would put it:
  - a plain statement of the idea ("Faith is trusting God's word without seeing"),
  - EGW's own vocabulary, if you know it ("the righteousness of Christ imputed",
    "the great controversy"),
  - a concrete scenario or image ("the disciples in the storm on Galilee"),
  - the objection or opposite, if the user wants balance.

## 2. Search wide in two calls

- **Ellen White:** one `sop_lookup(queries=[…angles…], lang="en", limit=8,
  min_score=0.80)`. Add `codes` only if the user limits the search to particular books.
- **Scripture:** `bible_search` takes a single query, so make one call per angle
  that needs verses, in the translation the user reads (`bible="kjv"` by
  default), with `limit=5`. When you already know the classic texts, skip the
  search and fetch them all in **one** `bible_lookup` `;`-list.

## 3. Verse-centred research

When the question is about a **verse** ("what does EGW say about John 3:16?"):

1. `sop_by_bible_ref(osis="John.3.16", lang="en", limit=40)`. Results are not
   ranked, so scan them all.
2. For a passage, repeat the call for its **key** verses only (2–4 calls). There is
   no range search.
3. Other languages: query English, then use `sop_parallel` to reach German. For other
   languages, use `sop_lookup` in that language on the sentences you kept.
   `sop_by_bible_ref` in German uses Luther numbering (see TOOLS.md).
4. Combine this with a `sop_lookup` on the verse's *wording*. That catches passages
   that allude to the verse without citing it.

## 4. Sift and deepen

- **Keep a hit only if its text actually says something on the question.** For
  themes, 0.82–0.88 can be right and 0.90 can be off-topic. Scores only rank the
  hits. They don't prove anything.
- **Merge duplicates.** A sentence found in the original book and in several
  compilations counts once. Keep the original (`DA`, `GC`, `SC`, `PP`, `MB`,
  `COL`, `MH`, `Ed`, …) and drop the compilation copies.
- **Deepen only the best 3–6 hits** with `sop_context(before=1, after=1)` (or a
  little more), so each quotation is complete and read in context. Don't fetch
  context for every hit.
- If an angle finds nothing useful, rephrase it once. If the rephrased query also
  fails, report that angle as not found.

## 5. Other languages (only for what you keep)

- German: `sop_parallel(book_code, para_key, lang="en", target_lang="de")` for
  each kept paragraph. It is exact and cheap.
- Other languages: one `sop_lookup(queries=[…], lang=<target>, limit=3)` with
  **your own rough rendering** of each kept sentence in the target language, using
  `codes` for the same work. Accept a hit only if it says the same thing.
- Bible: one `bible_lookup(ref=<all refs>, bible=<target>, numbering="kjv")`.
  Print the returned `osis`.
- If a language has no match, follow the `quote-translate` skill: mark any
  quotation you translate yourself.

## 6. Present

Group the findings by **idea**, not by source. For each idea:

- one line summarising what the sources say (your own words, clearly yours);
- the supporting quotations, **verbatim**, each with its reference (verse number
  of the edition quoted; EGW title/abbreviation + page for that language;
  author + title for a pioneer).

End with **what you searched** (angles, languages, verses) and **what the
corpus didn't cover**, so the user knows how far the answer goes. Don't
present a theme as absent from Ellen White's writings because this corpus returned
nothing.

Keep it proportionate. A quick question gets 3–5 strong quotations, not everything
the corpus returned.
