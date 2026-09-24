# sopack changelog

Versions before 0.1.4 predate this file; see the git history for those.

## 0.1.4

**Fixed — `extract` produced a `book.json` that `validate` refused, for any book
whose scan carries an inline citation scheme.** Eleven of the twenty-three works
in the 2026-09 pioneer acquisition failed with

```
para_key '3.1': seq values [0, 1] do not match chunks=1 (expected [0])
```

In such a book the extractor keys cited blocks from the citation and uncited ones
— headings, mostly — from a chapter ordinal, and the two collide: in COOH, `3.1`
is both "Revelation 18:1-5…" and "II. WHAT ARE WE TO UNDERSTAND BY THE FALL OF".
The extractor already handled that (a running `seen[para_key]` gives the second
one `seq=1`, which is what keeps their point ids apart), but `validate` read
`seq` as *chunk index within one paragraph* and asserted `seq == range(chunks)`.
The two are different numbers, and `Block` had only one field for them.

- `Block` gains `chunk`: the piece's index within its own paragraph. `seq` keeps
  its meaning — the per-`para_key` disambiguator that `sop/seq` hashes into the
  point id. This is the split `build_pioneers_corpus.py` has always made (`uid …
  #<seq>` alongside a separate `"chunk": j`), so packs now agree with how the
  49 books already in the collection were written.
- `validate` accepts a `para_key` holding several paragraphs: it requires the
  `seq` values to be dense from 0 and the blocks to partition into whole
  paragraphs of `chunks` pieces, and reports a truncated or mis-numbered
  paragraph as before.
- `to_payload` emits `chunk` from `Block.chunk` instead of `Block.seq`. The two
  are equal unless a `para_key` collides, so **no existing pack or point
  changes**; re-extracting the books that already worked yields identical ids
  and payloads, verified over 18,154 blocks across the 23 pioneer works.
- `dump` writes `chunk`; `load` defaults it to `seq` when absent, which is exact
  for every pre-0.1.4 `book.json` — a file with a collision could not be written.

No change to `contract.py`, the id rules, the pack format or the server: `chunk`
was already an accepted optional payload key.
