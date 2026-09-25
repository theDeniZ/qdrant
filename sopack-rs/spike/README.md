# M0 spike: is Rust the way? (SOPACK-1.0-PLAN.md §2)

**Verdict (2026-09-24): GO.** Rust (`ort` + `tokenizers`) reproduces the live
collections' vectors exactly. One caveat changes the plan: **on CPU, Rust is not
faster than Python per model call.** Rust earns its place through P1 (one binary, no
venv) and the packaging problems it removes, not through speed. See "What this
changes" below.

Measured in the devcontainer: aarch64 Linux, 4 cores, 12 GB RAM.

## Results

| Check (§2) | Bar | Result |
|---|---|---|
| Rust vs **stored** vectors: 281 points (48 EGW in 12 languages, 4 pioneer, 33 verses in 11 Bibles, 4 longest paragraphs (truncated at 512), 192 bench paragraphs) | cos ≥ 0.9999 each | **min 0.99999999998**, 0 below bar, every group ≥ 0.99999999998 |
| Rust vs Python fastembed 0.8.0, same texts | — | min 1.0000000 |
| Token ids vs Python `tokenizers` (281 fixture texts + de/en/ja/ko/ru/uk/mixed-punctuation/> 512-token probes) | identical | **289/289 identical** (truncation to 512 included) |
| fastembed-style batching (input order, 32) vs stored | ≥ 0.9999 | min 1.0000000 |
| Length-sorted, token-budget batching (512…16384) vs stored and vs one-at-a-time | ≥ 0.9999 | min 1.0000000 at every budget |
| Thread scaling 4 / 2 / 1, same inputs | identical vectors | **bitwise identical** (max abs diff 0) |
| CoreML (Mac) / CUDA | measure only | **not run**: needs your Mac / a GPU (plan §7.1) |
| Link mode | builds in CI with no network | **open**, see below |

### Throughput (192 bench paragraphs, 27,788 tokens, 4 threads)

| Engine | Batch | Time | Blocks/s |
|---|---|---|---|
| Python fastembed 0.8.0 | 32 (fastembed's usual shape) | 193.5 s | 0.99 |
| Python fastembed 0.8.0 | 1 | 71.7 s | 2.68 |
| Rust | 1 | 70.2 s | **2.73** |
| Rust | sorted, 512-token budget | 70.6 s | 2.72 |
| Rust | sorted, 1024 | 71.3 s | 2.69 |
| Rust | sorted, 2048 | 76.4 s | 2.51 |
| Rust | sorted, 8192 | 98.0 s | 1.96 |

Thread scaling (Rust, 8192 budget): 1 → 2 threads ×1.85, 1 → 4 threads ×2.90.

On the full 281-point mixed set: Python batch 32 took 348.6 s; Rust batch 32 took
286.6 s; Rust one-at-a-time took 99.7 s.

## What this changes in the plan

1. **The CPU speedup is batch size, not language.** Both sides call the same ONNX
   Runtime, so per-call speed is equal (70.2 s vs 71.7 s). Padding a batch of 32 to its
   longest text costs ~2.7×. `sopack pack` 0.1.x uses `batch_size=128`, so changing
   it to 1 would make today's Python roughly 2.7× faster without any Rust. That is a
   one-line change I have **not** made; it is yours to decide.
2. **§3.3 token-budget batching is a GPU lever, not a CPU one.** On CPU, anything
   above ~1024 padded tokens per batch is slower. The Rust default should be a
   budget of 512 on CPU (equal to batch 1, and still gives CUDA/CoreML something to
   work with); large budgets only behind `--device cuda|coreml`. Sorting is
   vector-neutral either way, so it stays.
3. **Threads are deterministic.** Vectors are bitwise identical at 1, 2 and 4
   threads, so auto-scaling (P3) cannot introduce drift on CPU.
4. **The server's `check_embedding` must change before any Rust pack is accepted**
   (it requires `library: "fastembed"`, `library_version: "0.8.0"`). M1's
   contract-as-data work covers this. These measurements are what to record there.

## Open: link mode

- **`load-dynamic`** (used here, with the Python wheel's `libonnxruntime.so.1.30.0`):
  works. This is also why the Rust/Python comparison is exact: same ORT build.
- **`download-binaries`** (static ORT inside the binary): downloads and compiles,
  but **could not be linked in this container**. The prebuilt ORT is C++ built against
  GNU libstdc++. There is no gcc here, and zig (the stand-in linker, see below)
  substitutes its own libc++. That is a limit of this container, not a verdict: it
  has to be tested on a CI runner with gcc (M7), where it is the normal setup.
  Note for the §3.2 "no HTTP client" CI check: `download-binaries` puts `ureq` in
  `ort-sys`'s **build**-dependencies, so the check must use
  `cargo tree -e normal`.
- Pick between them in M7: static (one self-contained file) vs dynamic plus the
  Homebrew `onnxruntime` formula.

## Reproduce

Toolchain in the devcontainer (no root, no gcc here): `rustup` (minimal profile,
Rust 1.98) in `~/.cargo`, and zig 0.16 in `~/.local/zig` as C compiler and linker via
`~/.local/bin/zigcc` (referenced by `.cargo/config.toml`, devcontainer only).

```bash
cd qdrant/sopack-rs/spike
# 1. fixture: read-only scroll of the live collections + Python reference (~6 min)
FASTEMBED_CACHE_PATH=/workspaces/sdarm/.fastembed_cache \
  /workspaces/sdarm/.venv/bin/python3.11 make_fixture.py
# 2. build + full run (~25 min on 4 cores)
~/.cargo/bin/cargo build --release
export ORT_DYLIB_PATH=/workspaces/sdarm/.venv/lib/python3.11/site-packages/onnxruntime/capi/libonnxruntime.so.1.30.0
M=/workspaces/sdarm/.fastembed_cache/models--qdrant--multilingual-e5-large-onnx/snapshots/66076b8dc6e367337e3e90e6fb309fb0f3addaf6
./target/release/sopack-spike $M fixture 4 2 1        # → results/m0-<arch>.json
./target/release/sopack-spike $M fixture sweep 4 0 512 1024 2048 8192
# 3. Python baseline on the same bench texts
FASTEMBED_CACHE_PATH=/workspaces/sdarm/.fastembed_cache \
  /workspaces/sdarm/.venv/bin/python3.11 py_bench.py    # → results/python-bench.json
```

On the Mac (CoreML row): the same, with `ort` features `coreml` and a
`with_execution_providers([CoreML…])` line added. That is the one remaining M0
measurement that needs Apple hardware.

Files: `make_fixture.py`, `src/main.rs`, `py_bench.py`, `results/` (JSON reports),
`results-run1.log`, `results-sweep.log`. `fixture/points.json` (10 MB, contains
stored vectors) is git-ignored; regenerate it with step 1.
