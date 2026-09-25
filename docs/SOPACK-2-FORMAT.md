# `.sopack` format `sopack/2` and calibration fixture `sopack.calibration/1`

**Status: normative spec, 2026-09-24.** Written for M1/M2 of
[SOPACK-1.0-PLAN.md](SOPACK-1.0-PLAN.md). Implements the store-neutral design of
[SOPACK-AUTONOMY.md](SOPACK-AUTONOMY.md) §3. Both the Python server
(`sopack/format.py`, `app/import_service.py`) and the Rust crates
(`sopack-format`, `sopack-contract`) implement exactly this. Where this spec and
code disagree, the code is wrong.

The embedding contract is data: [../sopack-rs/contracts/e5-large-v1/contract.toml](../sopack-rs/contracts/e5-large-v1/contract.toml).

---

## 1. Container

A ZIP file. Entries:

| Entry | Compression | Content |
|---|---|---|
| `manifest.json` | deflate | §2 |
| `points.jsonl` | deflate | one JSON object per line: `{"uid": str, "id": str, "payload": {…}}`, UTF-8, `ensure_ascii=False`, `\n`-terminated, in vector order |
| `vectors.f32` | **stored** | `counts.points × counts.dim` little-endian float32, same order as `points.jsonl` |
| `probe.f32` | **stored** | the pack's own embeddings of the calibration fixture entries, `len(probe.entries) × dim` LE float32, fixture order |
| `titles.json` | deflate | optional, `sop` profile only: additive title-table fragment (unchanged from `/1`) |

Unchanged from `sopack/1`: `points.jsonl` line shape, `vectors.f32`, `titles.json`,
id rules, uids, payload schemas, atomic publish (write to a unique temp path in
the destination directory, rename on success). Point ids are
`uuid5(NAMESPACE_DNS, uid)` strings, with `uid` built by the contract's
`[ids.rules]` template.

Readers MUST reject an unknown **major** schema (`sopack/3`) and MUST ignore
unknown manifest keys.

## 2. `manifest.json`

```json
{
  "schema": "sopack/2",
  "profile": "sop",
  "pack_id": "sop-2026-09-24-f0a3",
  "created_by": "sopack 0.9.0 (rust) on Linux 5.15 aarch64",
  "target": {"profile": "sop", "contract": "e5-large-v1"},
  "contract": {
    "id": "e5-large-v1",
    "sha256": "<sha256 of the contract.toml bytes used>",
    "calibration_sha256": "<sha256 of the calibration.json bytes used>"
  },
  "embedding": {
    "model": "intfloat/multilingual-e5-large",
    "pooling": "mean",
    "normalized": true,
    "dim": 1024,
    "distance": "cosine",
    "max_tokens": 512,
    "passage_prefix": "passage: ",
    "runtime": "sopack-rs 0.9.0; ort 2.0.0-rc.13; onnxruntime 1.30.0",
    "device": "cpu",
    "threads": 4,
    "batch_tokens": 512
  },
  "id_rule": "sop/seq",
  "id_rule_doc": "uuid5(dns, '<lang>:<book_code>:<para_key>#<seq>')",
  "counts": {"points": 23, "books": 1, "dim": 1024, "points_bytes": 16026},
  "sha256": {"points.jsonl": "…", "vectors.f32": "…", "probe.f32": "…", "titles.json": "…"},
  "books": [ { "book_code": "WDYS", "lang": "en", "points": 23, "first_id": "…",
               "title": "…", "author": "…", "year": 1861, "corpus": "pioneers",
               "slug": "…", "book_pair": "WDYS", "id_rule": "sop/seq",
               "book_sha256": "…" } ],
  "probe": {
    "kind": "calibration",
    "fixture_sha256": "<same as contract.calibration_sha256>",
    "vectors": "probe.f32",
    "entries": [ {"id": "<fixture entry id>", "profile": "sop", "vector_offset": 0} ],
    "self_check": {"n": 16, "min_cosine": 0.99999999, "mean_cosine": 0.99999999,
                   "threshold": 0.9999}
  }
}
```

Rules:

- `target` has **no** `collection`, `vector_name`, `vector_size` or Qdrant
  `distance` spelling. Those are adapter config on the importer.
- `embedding` is checked by the importer against the contract named in
  `target.contract` on the keys `model, pooling, normalized, dim, distance,
  max_tokens, passage_prefix`. Any difference is fatal. `runtime`, `device`,
  `threads`, `batch_tokens` are provenance (R10) and are not compared. There is
  **no** `library` / `library_version` check any more: acceptance is decided by
  the calibration probe, not by which library produced the vectors.
- `probe.entries` lists **every** fixture entry, in fixture order.
  `self_check` is what the packer measured against the fixture vectors (§3). A
  writer MUST refuse to produce a pack whose `self_check.min_cosine` is below the
  contract's `calibration.pack_min_cosine`.
- `books[]` is unchanged from `/1`.

## 3. Calibration fixture `calibration.json`

Lives next to its contract: `contracts/<id>/calibration.json`. Committed. Made
by the importer-side admin command (reads the store), never by sopack.

```json
{
  "schema": "sopack.calibration/1",
  "contract": "e5-large-v1",
  "created_at": "2026-09-24T12:00:00Z",
  "source": {"store": "qdrant", "note": "vectors copied from the live collections"},
  "entries": [
    {"id": "<point id in the store>", "profile": "sop", "uid": "<uid or null>",
     "lang": "en", "note": "EGW en", "text": "<raw text, WITHOUT the passage prefix>",
     "vector": [1024 floats]}
  ]
}
```

- 16 entries: EGW en + de, several other SoP languages, pioneer (`corpus`),
  several Bible translations, and at least one text over 512 tokens.
- `sha256` of the exact file bytes goes into `contract.toml`
  `[calibration].sha256`, and packs record it.
- The packer embeds `passage_prefix + text` for every entry with the same model
  instance and settings it uses for the books, **before** embedding any book,
  and aborts (exit code 5 in the Rust CLI) if any cosine is below
  `pack_min_cosine`.

## 4. Import-time probe (importer, model-free)

For a `sopack/2` pack:

1. **pack ↔ fixture:** the importer's copy of the fixture must have the sha256
   in `probe.fixture_sha256` (else: refuse, "pack was calibrated against a
   different fixture"). Cosine of each `probe.f32` vector with the fixture
   vector ≥ `probe_min_cosine` (0.95), warn under `probe_expect_cosine` (0.99).
2. **fixture ↔ store:** the adapter fetches the stored vectors of the fixture
   entries whose `profile` maps to this import's collection and cosines them with
   the fixture vectors (≥ `probe_min_cosine`). This proves the target store is in
   the contract's space.
3. **empty store / entries absent:** if the store holds none of the fixture's
   ids for that profile, step 2 has nothing to compare; the importer records the
   contract fingerprint (`contract.sha256`) as the store's space at first import
   and refuses later imports under a different fingerprint.

A `sopack/1` pack keeps the old live-canary probe (canary ids + stored vectors).
Readers accept `/1` and `/2`; 1.0 writers produce only `/2`.
