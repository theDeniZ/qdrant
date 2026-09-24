# sopack → Rust — potential migration

**Status: proposal, not started. 2026-09-24.** Nothing here is decided except the
order of work: the vector-equivalence spike (§4, step 0) is the go/no-go gate, and
no porting starts before it passes.

Related: [IMPORT-PIPELINE.md](IMPORT-PIPELINE.md) (requirements R1–R12, failures
#1–#15), [IMPORT-PIPELINE-PLAN.md](IMPORT-PIPELINE-PLAN.md) (the current design),
[../Formula/sopack.rb](../Formula/sopack.rb), [../sopack/README.md](../sopack/README.md).

---

## 1. Why

Packaging a Python CLI with a native-wheel closure through Homebrew has become the
most fragile part of `sopack`:

1. **`post_install` is deprecated.** `brew install sopack` prints
   `Calling post_install is deprecated! Use post_install_steps instead.`
   Homebrew 6.0.16 deprecated the imperative hook; 7.0.0 rejects it in official taps
   and warns third-party taps **until 11 December 2027**, after which it is expected
   to stop running (silently). The replacement, `post_install_steps`, is
   declarative: it cannot run Ruby, only literal-argv `run` steps, so building a venv
   there means shipping a packaged helper script just to be allowed to call `pip`.
2. **The venv lives outside the keg.** Homebrew rewrites the install names of every
   Mach-O file in a keg after `install`; prebuilt wheels are not linked with
   `-headerpad_max_install_names`, and `py_rust_stemmers`' `.so` fails that rewrite
   (`Updated load commands do not fit in the header`). The workaround builds the venv
   in `post_install` under `var/sopack/venv`, so `brew uninstall sopack` leaves it
   behind and the caveats have to ask for a manual `rm -rf /opt/homebrew/var/sopack`.
3. **Platform lock.** The pinned `onnxruntime==1.30.0` has only a
   `macosx_14_0_arm64` wheel, hence `depends_on arch: :arm64` +
   `depends_on macos: :sonoma` — no Intel Macs, no macOS < 14.
4. **Model cache in `$TMPDIR`.** fastembed defaults its cache to a temp directory
   macOS purges; the caveats ask the user to export `FASTEMBED_CACHE_PATH`.

A single compiled binary removes 1–3 outright and lets the tool own 4.

## 2. Language choice

| Option | Verdict | Reason |
|---|---|---|
| **Rust** | **Recommended** | `fastembed-rs` exists and supports `MultilingualE5Large` with **mean pooling** (same as Python fastembed 0.8.0) from the same `qdrant/multilingual-e5-large-onnx` export. The `tokenizers` crate is the same Hugging Face library Python's `tokenizers` wraps, so tokenization is identical by construction. Built from source by `cargo`, Homebrew relinks it without trouble. |
| Ruby | Rejected | No fastembed equivalent. The Ruby embedding stack (`informers`, `tokenizers`, `onnxruntime` gems) is native extensions — the tokenizer one is itself Rust — so it hits the same Mach-O relocation failure, plus a gem environment to manage. No gain over Python. |
| Go | Not pursued | ONNX Runtime and HF tokenizers only via cgo bindings; more glue than Rust for the same result. |
| Stay on Python | Fallback (§6) | Fixes the deprecation, not the architecture. |

## 3. What changes and what does not

**Stays Python, unchanged:** the server. `app/import_service.py:38-39` imports
`sopack.contract` and `sopack.format.PackReader`; the image copies `sopack/` for
those stdlib-only modules. The Python package remains the **reference
implementation** of the pack format, the id rules and the contract.

**Becomes Rust:** the Mac-side CLI — `extract` (epub / markdown / text / sop_json +
chunking), `inspect`, `canaries`, `pack`, `verify`, `doctor`. Roughly 2,900 lines of
Python plus tests today.

**Formula becomes:** a plain source build, no venv, no `var/`, no post-install hook.

```ruby
depends_on "rust" => :build
depends_on "onnxruntime"          # Homebrew's, if linked dynamically — see §5.3
def install
  system "cargo", "install", *std_cargo_args(path: "sopack-rs")
end
test do
  assert_match version.to_s, shell_output("#{bin}/sopack --version")
end
```

The arm64/Sonoma restriction can likely be dropped (Homebrew's `onnxruntime`
formula builds for Intel too) — to be confirmed on a runner.

## 4. Plan

0. **Spike — vector equivalence (go/no-go).** In `qdrant/sopack-rs/`, a throwaway
   binary that embeds the 8 canary points (`sopack canaries … -n 8`) with
   fastembed-rs using `"passage: "` prefix, mean pooling, L2 normalisation, and
   compares each against the stored vector in the live `sop` and `bibles`
   collections.
   - **Pass:** every cosine ≥ **0.9999** (the Python path measures 1.00000).
   - The contract's hard floor (`PROBE_MIN_COSINE = 0.95`) is *not* the bar here —
     a port that merely clears it is a different vector space in waiting.
   - Also compare token ids for a handful of texts (German, English, long > 512
     tokens to exercise truncation) against Python `tokenizers` output.
   - Needs rustup in the devcontainer (not installed today). The devcontainer can
     reach Qdrant.
1. **Contract update.** `contract.EMBEDDING` currently requires
   `library: "fastembed"`, `library_version: "0.8.0"`, and `check_embedding` rejects
   any difference — a fastembed-rs pack would be refused by the server. Change the
   contract deliberately, recording the spike's measurements in its docstring (as the
   2026-09-22 measurements are recorded now). Options: accept a set of
   `(library, version)` pairs, or declare the embedding by model + pooling +
   normalisation and let the canary probe be the proof. Decide after step 0.
2. **Shared constants.** Move the data parts of `contract.py` (schema ids,
   `EMBEDDING`, vector name/size/distance, profiles' required/optional keys and
   indexes, id-rule templates) into a JSON file both sides read — Python at import,
   Rust via `include_str!`. The logic stays duplicated; the numbers must not.
3. **Port**, module by module, each with its Python tests translated:
   `format` (ZIP: `manifest.json` + `points.jsonl` + `vectors.f32` LE float32 +
   `titles.json`) → `contract` id rules (uuid5 over `NAMESPACE_DNS`) → `book` →
   `extract/*` → `pack` → `verify` → `canaries` → `doctor` → CLI with the same
   subcommands and flags.
4. **Cross-implementation conformance in CI** (the guard against failure #15, client
   and server drifting):
   - Golden fixtures: point ids for each id rule, a byte-exact small pack.
   - Every Rust-written pack in CI must pass Python `sopack verify` **and**
     `import_service`'s reader (`PackReader`) — the same checks the server runs.
   - Both extractors on the same fixture sources must produce the same `book.json`
     (or the difference is documented as intended).
5. **Release workflow.** `release-sopack.yml` keeps its shape: version check, tests,
   tarball, real `brew install` + `brew test` on a macOS runner before the formula is
   bumped. Add the §4.4 conformance job.
6. **Cut-over.** Ship the Rust formula; keep the Python CLI installable from a
   checkout (it is still the reference) until one real quarter has been packed and
   imported through the Rust binary with probe cosines ≥ 0.9999.

## 5. Risks and details to carry over

1. **Float drift across ONNX Runtime versions.** Python pins onnxruntime 1.30.0;
   fastembed-rs's `ort` crate may bundle another. Small fp differences are expected
   and acceptable only if the spike shows ≥ 0.9999; pin the `ort` / runtime version
   in `Cargo.lock` and treat bumping it like bumping fastembed today (re-measure).
2. **Model cache location.** fastembed-rs defaults to `.fastembed_cache` in the
   **current directory**. The binary must set it explicitly —
   `~/Library/Caches/sopack` on macOS, `$XDG_CACHE_HOME/sopack` elsewhere, override
   by env var — which also retires the `FASTEMBED_CACHE_PATH` caveat.
   `doctor` must check the path the binary will actually use (the lesson of the
   `$TMPDIR` check in `doctor.py`).
3. **ONNX Runtime linking.** Either statically linked / downloaded by `ort` at build
   time (self-contained, but Homebrew builds are sandboxed without network — check
   `ort`'s build behaviour) or dynamically against Homebrew's `onnxruntime` formula
   (clean, but ties the runtime version to Homebrew's). Decide in the spike.
4. **R7 — fail in seconds.** Today `import sopack.pack` asserts the fastembed version
   before any slow work. The Rust equivalent: `pack` loads the model and embeds one
   probe string before touching the input, and refuses on a size mismatch.
5. **`--workers`.** Python's parallel mode forks processes that each load a 2.2 GB
   model and hang when RAM is short. In Rust, parallelism should be ONNX Runtime
   intra-op threads inside one process — one model copy — which may make the flag
   unnecessary.
6. **Pack atomicity.** Keep the current behaviour: build at a unique temp path,
   rename into place only when complete.
7. **EPUB extraction parity.** The Python extractor encodes hard-won rules (OPF
   `<dc:date>` is often the digital edition's year, the loud NOTE on fallback,
   damage/drop reporting in `inspect`). Port the tests first and make them pass;
   don't re-derive the rules.

## 6. Fallback — stay on Python

If the spike fails (cosine < 0.9999 and not fixable by pooling/normalisation/runtime
settings), keep Python and fix the formula instead:

- Replace `post_install` with `post_install_steps` that `run` a packaged helper
  (e.g. `libexec/bin/sopack-bootstrap`) which builds the venv offline from the wheels
  in `libexec/wheels`.
- Try building the venv **inside the keg** (`libexec/venv`) — steps may write to
  `prefix` — so `brew uninstall` removes it, and drop the `rm -rf` caveat. Unverified:
  whether a later relocation pass touches it. Confirm with a real `brew install` /
  `brew upgrade` / `brew uninstall` cycle on the macOS runner.
- Platform lock and model-cache caveat remain.

No hurry either way: third-party taps keep working with a warning until
2027-12-11.

## 7. Sources

- Homebrew 7.0.0 release notes — <https://brew.sh/2026/09/13/homebrew-7.0.0/>
- Formula Cookbook, `post_install_steps` — <https://docs.brew.sh/Formula-Cookbook>
- Homebrew/brew #22372 "Add formula install steps" — <https://github.com/Homebrew/brew/pull/22372>
- fastembed-rs, `text_embedding/impl.rs` (MultilingualE5Large → `Pooling::Mean`) —
  <https://github.com/Anush008/fastembed-rs>
- qdrant/fastembed #384 (e5 should use mean pooling) — <https://github.com/qdrant/fastembed/issues/384>
- ONNX export — <https://huggingface.co/Qdrant/multilingual-e5-large-onnx>
