# Changelog

All notable changes to the `sopack` Rust CLI. Versions before `1.0.0` are
pre-release milestones on the way to the cut-over described in
[`../docs/SOPACK-1.0-PLAN.md`](../docs/SOPACK-1.0-PLAN.md).

## 0.9.0 — 2026-09-24

First feature-complete build of the `sopack` binary (M2–M5 of the plan): a
single Rust CLI that extracts, packs, verifies and doctors `.sopack`
bundles with no Python and no live store dependency.

### Added

- **CLI binary** (`crates/sopack-cli`, bin `sopack`) wiring every library
  crate (`sopack-contract`, `sopack-format`, `sopack-book`,
  `sopack-extract`, `sopack-embed`, `sopack-progress`) into one command
  surface: `extract`, `inspect`, `propose`, `pack`, `calibrate`, `verify`,
  `doctor`, `model fetch|import|verify|path`, `schema`, `commands`,
  `contract show|list`.
- **`--json` machine contract** on every command: one JSON result or error
  document on stdout, progress always on stderr, validated against a
  committed schema (`schemas/*.v1.json`) and a stable 8-entry exit-code
  table (0 ok … 7 interrupted).
- **`sopack pack` resume**: periodic checkpointing to
  `<out>.sopack.partial/`, a fingerprint of the books/contract/options a
  run started with (refusing to resume under different ones), and a
  crash-then-resume path proven byte-identical to an uninterrupted run
  against a real book and the real embedding model.
- **Metadata workflow**: `sopack propose` (candidate drafting with
  evidence, never silent guessing) → `<source>.meta.toml` sidecars
  (`sopack propose --write-meta`, `sopack extract --meta`/auto-pickup) →
  offline book-code collision warnings against `contracts/<id>/book_codes.json`.
- **Device self-verification**: `--device auto|cpu|coreml|cuda`, every
  non-CPU candidate calibrated against the contract's fixture before use;
  `auto` falls back to CPU on a low score, an explicit device refuses.
- `README.md` (this crate's user-facing guide) and this changelog.

### Refactors

- `sopack-book`'s temporary `contract_lite` duplicate replaced by a thin
  bridge onto `sopack-contract` (now the single source of truth for
  profiles/id-rules), with the same public function names so no call site
  needed a semantic change.
- `sopack-embed::EmbedSpec` and `sopack-embed::calibration::CalibrationFixture`
  each gained an `impl From<&sopack_contract::Contract>` /
  `impl From<&sopack_contract::Calibration>` conversion
  (`sopack-embed/src/contract_spec.rs`), removing every hand-parse of
  `contract.toml` that used to live in this crate's own tests/examples.
- `sopack-format::PackWriter` gained a `pack_id()` getter so a caller can
  report the id actually in use (given explicitly, defaulted, or restored
  from a resumed checkpoint) without re-deriving it.

### Verified against the real embedding model (ignored-by-default tests, run under `flock`)

- `sopack calibrate`: min cosine 0.999999999987, mean 0.999999999997
  against the 16-entry fixture (threshold 0.9999).
- `sopack pack` of `qdrant/packs/wdys.book.json` (23 points): accepted by
  Rust `verify`, accepted by the Python reference reader
  (`conformance/packs/check_pack_py.py`), accepted by the Python importer's
  `run_calibration_probe` against an in-memory adapter
  (`conformance/packs/check_import_probe_py.py`, new), and matches the
  Python-built `qdrant/packs/wdys.sopack` at worst-case cosine 1.00000000
  across all 23 shared point ids.
- Crash-after-one-batch + resume produced a `.sopack` byte-identical to an
  uninterrupted run (166,947 bytes, same `pack_id`/`created_by`).

### Fixed after the first integration pass (2026-09-25)
- `pack`: the overall `%` froze during the whole embed phase. Calibration now reports its
  own `calibrate` stage, and one `embed` stage spans every checkpoint chunk, sized in real
  tokens. Model verification has a plan weight.
- ONNX Runtime lookup canonicalises the executable path, so a symlinked `sopack`
  (Homebrew, install.sh) finds `../lib` next to the real binary.
- `propose`: a `registry:title_match` candidate plus a RE-IMPORT warning when the work is
  already in the store (evidence carries the stored author and year). A byline that is
  part of the title ("… by the Scriptures") is no longer taken as the author.
- `pack`: removed an orphaned `stage_start` whose total was a word count.
