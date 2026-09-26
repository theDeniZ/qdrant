---
name: quote-translate
description: Render Bible and Ellen G. White quotations into another language by retrieving the canonical published wording from the bible-sop corpus (with that edition's verse and page numbers) instead of translating them, and handle what the corpus lacks transparently. Use when translating a text that contains Scripture or Ellen White quotations, or when the user asks how a quotation reads in German, Russian, Spanish, Korean or another language.
---

# Quotation translation from canonical editions

First read [references/TOOLS.md](references/TOOLS.md), then
[references/COVERAGE.md](references/COVERAGE.md) (which languages, editions and
books exist) and [references/VERSIFICATION.md](references/VERSIFICATION.md)
(numbers change between editions).

**Principle.** A quotation in translation must be the wording readers find in
their own published edition, with that edition's reference. Machine or
hand translation is the last resort. When you use it, you mark it.

## Target editions

Decide the target edition per language before starting and state it in one line.
Use the user's or project's rule if one is given. Otherwise:

- Bible: the language's standard translation in the corpus (de `luther1912`,
  then `schlachter`; ru `synodal`; uk `ukrogienko`; es `spanish`; ja `japkougo`;
  ko `korean`; en as the source text uses). Check with `bible_list_translations`.
- Ellen White: the edition the corpus returns for the target `lang`. German often
  has two editions of one work; prefer the one the user names, otherwise report
  both codes and use the closer match.

## Work in batches

First inventory every quotation in the text. Then fetch them together:

- **one `bible_lookup`** holding every reference (`;`-list), for the target
  translation plus `kjv` for comparison, with `numbering="kjv"` for English refs;
- **one `sop_list_books(search=…)`** per cited work, to get the target-language codes;
- **one `sop_lookup(queries=[…], lang=<target>)`** for every EGW sentence that
  `sop_parallel` could not supply.

Then handle the misses one at a time. **Always pass `lang`** to the `sop_*` tools.

## Procedure per quotation

1. **Identify the source** in the source language (see the `quote-verify`
   procedure). You need the verse, or the work and paragraph. A wrong source
   produces a wrong translation.
2. **Bible** → `bible_lookup(ref=<source ref>, bible=<target>, numbering="kjv")`
   when the source reference is English/KJV. Confirm that the returned words mean
   what the source verse says, and apply the VERSIFICATION.md quirks. Quote the
   returned text and **print its `osis` number** in the target language's
   reference style.
3. **Ellen White** →
   - If the source paragraph's `para_key` is already known and the pair is
     German↔English, try `sop_parallel(book_code, para_key, lang=<source>,
     target_lang=<target>)` **first** — it retrieves the published counterpart
     paragraph directly, which is cheaper and more certain than a fresh
     semantic search. It only covers de↔en (ja/ko have no alignment data and
     the tool errors rather than guessing); fall through to the steps below
     whenever it errors or the pair isn't de↔en.
   - Resolve target-language codes: `sop_list_books(search=<work>)`.
   - `sop_lookup(query=…, codes=<target codes>, lang=<target>, limit=3)`. Query
     with a rough target-language rendering of the sentence when you can: a
     target-language query scores far better than the English one. Batch with
     `queries` for many quotations.
   - Accept only a hit whose text **says the same thing** as the source sentence.
     Read neighbouring paragraphs with `sop_context` (cheap, a few paragraphs
     either side) or `sop_book_paragraphs` (an explicit page range) if the
     quotation spans paragraphs, and cut the target text to the same extent as
     the source quotation.
   - Cite the **target edition's** title and the page `sop_lookup` (or
     `sop_parallel`) returned.
4. **Nothing canonical found.** Only after steps 2–3 came back empty or unrelated:
   - Bible: say which translations were checked. For Romanian (no Bible index),
     try `sop_lookup(lang="ro")` on the verse; EGW's Romanian books quote
     Cornilescu.
   - Pioneer authors (`corpus: "pioneers"`) exist in English only, so every
     other language needs a marked own translation.
   - Ellen White: translate faithfully and **mark it**, e.g. a note "(own
     translation; no published <language> edition in the corpus)", or follow the
     project's marker convention. Or drop the quotation marks and render it as
     reported speech. Ask the user which, unless the project defines it.

## Keep a log

For every quotation, record: the source reference; the target reference and
edition; how it was obtained (`bible_lookup` / `sop_lookup` hit with book, page and
score / own translation); and anything the reader or reviewer must check. Give
the log to the user with the translated text. Never drop it silently.

## Do not

- Retranslate retrieved wording to make it "flow". Adjust the surrounding
  sentence instead.
- Carry the English verse or page number into the target text.
- Treat a high score as proof. Unrelated paragraphs score about 0.80.
- Claim a published translation does not exist because the corpus has none.
