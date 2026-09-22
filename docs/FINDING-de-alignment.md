# Finding: the German `aligned` field in the live `sop` collection is inverted

**Found 2026-09-22 while building the import pipeline. Not caused by it.**
**Severity: silent wrong answers. ~97 % of German alignments are incorrect.**

## What is wrong

`generator/data/sop/de/<CODE>.json` carries an `en_reverse` map. Its keys are
**English** para_keys and its values are the **German** para_keys they correspond to:

```jsonc
// de/BH.json  (BH = Biblische Heiligung, en_code SL = The Sanctified Life)
"en_reverse": { "7.1": ["5.1"], "8.2": ["5.4"], "20.1": ["13.3"] }
```

Proved by reading the texts on both sides:

| | text |
|---|---|
| `EN SL 7.1` | "The sanctification set forth in the Sacred Scriptures has to do with the entire being—spirit, soul, and body" |
| `DE BH 5.1` | "Heiligung im Sinne der Bibel umfaßt den ganzen Menschen: Geist, Seele…" |

That is the same paragraph. So `en_reverse[EN] = [DE]`.

But [`build_sop_vector_index.py`](../../generator/src/sdarm/tools/build_sop_vector_index.py)
reads the map in the opposite direction. Its DE loop does:

```python
_index_book("de", de_code, …, lambda pk, _rev=en_reverse: _rev.get(pk) or None)
```

— looking a **German** para_key up in a map keyed by **English** para_keys. And the EN
side is built from the same misreading (`for de_pk, en_pks in en_reverse.items()`), so
both directions are wrong.

The third row above shows what the collection actually stores: the indexer pairs
`EN 7.1` with `DE 7.1` — *"Biblische Heiligung besteht nicht in heftigen
Gefühlsaufwallungen"* — an unrelated paragraph.

## Why it was not obvious

It does not fail loudly, it produces *plausible* wrong answers. German and English
editions have overlapping page ranges, so a German para_key often exists as a key in the
map by coincidence — 58 % of `BH`'s do. Those coincidental hits make the field look
populated (62 % of live DE points carry a non-null `aligned`) while the pairing is
meaningless.

## Measured impact

1500 live German points, compared against the correct reading of their own
`en_reverse`:

| | count | share |
|---|---:|---:|
| stored alignment is **wrong** | 876 | 58.4 % |
| alignment **lost** (stored null, one exists) | 464 | 30.9 % |
| correctly null | 111 | 7.4 % |
| stored alignment correct | 49 | **3.3 %** |

The 49 correct ones are cases where the German and English para_keys happen to be
identical. English points fare worse: only 3.1 % carry any `aligned` at all.

So `sop_parallel` — the de↔en paragraph pairing the translation workflow relies on —
returns a wrong paragraph roughly whenever it returns one.

## What it affects

- `sop_parallel` in `app/sop_tools.py` and `translator/sop_tools_mcp.py`
- the `aligned` payload field on every `lang: "de"` and `lang: "en"` point in `sop`
- **not** retrieval: `sop_lookup`, `sop_book_paragraphs` and every vector search are
  unaffected, because the vectors and the text are correct. Only the cross-language
  pointer is wrong.

## Fixing it is cheap — no re-embedding

`aligned` is payload, not vector. The correction is a payload-only update over the
existing points (`POST /collections/sop/points/payload`), which costs minutes, not the
hours a re-embed would. Two steps:

1. Correct the direction in `build_sop_vector_index.py` — for a DE point, the English
   counterparts are `[k for k, v in en_reverse.items() if de_para_key in v]`; for an EN
   point they are `en_reverse.get(en_para_key)`.
2. Backfill the payload for existing points rather than rebuilding the collection.

**This is deliberately not done here.** It is a generator-repo change plus a write to
the live collection, and neither is in the scope of the import pipeline. It needs its
own decision.

## Credit / provenance

Surfaced by the extraction agent while porting the alignment logic, which noticed the
live indexer's lambda disagreed with the real data shape. Verified independently
against the live collection and the source JSON before being written down here. The new
pipeline's `sopack/extract/sop_json.py` + `book.py` implement the **correct** direction,
so packs built by it are not affected.
