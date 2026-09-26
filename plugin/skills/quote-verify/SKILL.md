---
name: quote-verify
description: Check every Bible and Ellen G. White quotation in a text against the bible-sop corpus by finding its source, comparing its wording and checking its reference, then report what matches, what differs and what cannot be found. Use when the user asks to verify, fact-check or source quotations in a draft, sermon, article, lesson or plan, or asks "where is this quote from?".
---

# Quotation verification

First read [references/TOOLS.md](references/TOOLS.md), then the Non-negotiable
list of the lookup discipline below. Consult
[references/COVERAGE.md](references/COVERAGE.md) and
[references/VERSIFICATION.md](references/VERSIFICATION.md) as needed.

Lookup discipline: retrieve, never recall. Judge hits by their text, not their
score. An empty result means "not in this corpus", never "does not exist". The
corpus holds Ellen White plus a small English shelf of pioneer authors (Smith,
Jones, Waggoner, …), and no other writers.

**You verify; you do not rewrite.** Change the user's text only when asked, and
then only the quotations and references you verified.

## 1. Inventory

Extract every quotation into a numbered list: quoted text, attributed author,
the reference as given (or "none"), language, and location in the document
(heading or paragraph). Include quotations without quotation marks when a
reference follows them, and references with no quoted words (these are checked
for existence only). Show the count before checking.

## 2. Check

Check in **batches, not one quotation at a time**. That means one `bible_lookup` for every
Bible reference in the document (a `;`-list, with a list for `bible`), one
`sop_list_books(search=…)` per cited work, and one `sop_lookup(queries=[…],
lang=…)` per language for every Ellen White sentence. Then follow up only on
the items that failed. A 20-quotation document should take about five calls,
not forty.

**Bible quotations**
- With a reference: fetch it in the translation the text appears to use (and
  `kjv` when the reference is English), using `numbering="kjv"` for English
  references in other editions.
- Without a reference, or the reference yields other words: `bible_search` on
  the quoted words in that language, then `bible_lookup` the candidate.
- Identify the translation when unstated: fetch the verse in every translation
  of that language and pick the one whose words match.

**Ellen White quotations**
- Resolve the cited work with `sop_list_books(search=…)`. Look the sentence up
  with those `codes` in the quotation's language, `limit=3`.
- Nothing matches in the cited work → search corpus-wide in the same language
  (drop `codes`), then in English if the quotation is a translation.
- A cited page → `sop_book_paragraphs` on that page (±1) to confirm the page.
- The quotation looks trimmed, merged, or cut mid-sentence against the hit →
  pull a few paragraphs of surrounding text with `sop_context(book_code,
  para_key, lang, before, after)` rather than a whole page range — it is the
  cheap way to see what precedes/follows a hit.
- Attributed to a pioneer (Uriah Smith, A. T. Jones, E. J. Waggoner, J. N.
  Andrews, James White, …) → check it like an EGW quotation, in English only.
  Look up codes with `sop_list_books(search="<author>")`.
- A hit carries `corpus: "pioneers"` but the text credits Ellen White → verdict
  **Wrong reference** (misattributed author). Name the real author.
- Attributed to anyone else → **Out of scope**. The corpus cannot check it.

## 3. Classify each quotation

| Verdict | Meaning |
|---|---|
| **Verified** | Words match the retrieved text (ignoring typography); reference correct |
| **Wording differs** | Source found; words changed, shortened without ellipsis, or merged from two places |
| **Wrong reference** | Words found, but under a different verse, book, page or edition |
| **Other translation** | Words match a different Bible translation or EGW edition than stated or implied |
| **Paraphrase** | The idea is in the source, but not in these words; should not be in quotation marks |
| **Not found** | No passage says this. Report what you searched (tools, languages, codes) |
| **Out of scope** | Author or work not in the corpus; cannot be judged here |

Never upgrade "Not found" to "misattributed" or "invented". The corpus is not
complete enough to prove a negative.

## 4. Report

One table, in document order: `#`, location, verdict, the quoted text (short),
the retrieved text **verbatim** for every non-verified item, and the correct
reference as returned (verse number of the edition quoted; page of the language
edition). Then a short list of recommended fixes. For each fix, show the exact
replacement text and reference. Finish with totals per verdict.
