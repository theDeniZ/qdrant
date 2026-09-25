# sopack changelog

Versions before 0.1.4 predate this file; see the git history for those.

## 0.2.0 — store-neutral packs (M1, SOPACK-1.0-PLAN.md)

**`sopack/` no longer connects to a store, or to anything over HTTP, at
all.** Implements SOPACK-AUTONOMY.md's store-independence design and
SOPACK-2-FORMAT.md (normative). A test enforces this:
`sopack/tests/test_neutrality.py` greps the package for HTTP client
imports, store URLs, and the words `collection`/`vector_name` in real code
(not docs/comments).

- **Pack format `sopack/2`.** `PackWriter` now always writes `sopack/2`:
  `target` is `{"profile", "contract"}` (no Qdrant collection/vector/distance
  fields), a new `contract` block records the contract id + its own sha256 +
  the calibration fixture's sha256, and `probe` carries the pack's own
  embeddings of the **committed calibration fixture** plus a `self_check`
  summary — not a live-canary scroll. `PackReader` accepts both `sopack/1`
  (legacy) and `sopack/2`, and rejects any other major schema outright.
- **Calibration replaces canaries.** `sopack canaries`, `sopack pack
  --canaries` and `sopack/canaries.py` are gone. `sopack.pack.pack()` no
  longer takes a `canaries` argument; it loads and sha256-verifies the
  contract's committed `calibration.json` (or an explicit override —
  `calibration=`), embeds every fixture entry with the same model instance
  **before embedding any book**, and refuses to build
  (`CalibrationFailed`, exit code 5) if the self-check scores below
  `contract.toml`'s `[calibration].pack_min_cosine`. Building the fixture
  itself is now an importer-side admin command,
  `python -m app.calibration export` (server repo `app/`), not a sopack
  command — it is the one place that still needs to read a live store, and
  sopack must never do that.
- **The contract is data.** `sopack/contract.py` loads
  `sopack-rs/contracts/<id>/contract.toml` with `tomllib` instead of hard-coding
  the embedding contract in Python; `collection`, Qdrant vector-name and
  payload-index types are no longer part of it at all (moved to the
  server's `app/store_adapter.py` — an adapter config, not the contract).
  `check_embedding` compares only the keys SOPACK-2-FORMAT.md §2 lists,
  with no `library`/`library_version` check — acceptance is decided by the
  calibration probe, not by which library produced the vectors.
- **Default `pack` batch size is now 1**, not 128: M0 measured single-text
  batches at 2.73 blocks/s on CPU vs 0.99 blocks/s at batch 32 — a large
  batch is a GPU lever, not a CPU one (SOPACK-1.0-PLAN.md §2/§3.3).
- Server-side (`app/`): the importer now talks to a backend only through a
  `StoreAdapter` (`app/store_adapter.py`) — `QdrantAdapter` (today's
  behaviour, reorganised) and an `InMemoryAdapter` used by tests to prove a
  `.sopack` imports the same way into a second backend. The `sopack/2` probe
  (`import_service.run_calibration_probe`) is backend-agnostic by
  construction; the legacy `sopack/1` live-canary probe keeps working
  unchanged for existing packs.

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
