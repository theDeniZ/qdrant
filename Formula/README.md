# Homebrew: this repository is the tap

There is no separate `homebrew-tap` repository. Homebrew can tap any git URL, and it
finds formulae in a top-level `Formula/` directory, so `theDeniZ/qdrant` serves
`sopack` directly.

## Install (Apple Silicon, macOS 14+)

```bash
brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant   # once — the URL is required,
                                                               # the repo isn't named homebrew-*
brew install theDeniZ/qdrant/sopack
sopack model fetch                                             # ~2.2 GB model, once
sopack doctor                                                  # loads it + runs the calibration check
```

Upgrade: `brew update && brew upgrade sopack`.

## Binary-only formula

The Rust binary (`sopack` CLI) is prebuilt for macOS ARM64 and packaged with the ONNX Runtime 1.30.0
shared library (`libonnxruntime.dylib`), its license, and the contract schemas.

**The binary locates the ONNX Runtime dynamically.** The formula installs `bin/` and `lib/`
into the keg's `libexec/` (not the keg's `lib/`, which Homebrew would link into
`$(brew --prefix)/lib`, where `libonnxruntime.dylib` would collide with Homebrew's own
`onnxruntime` formula). `$(brew --prefix)/bin/sopack` is a symlink; the binary canonicalises
its own path and loads ORT from `<real exe dir>/../lib/` = `libexec/lib/`. No environment
variables, no extra configuration. `sopack doctor --quick` shows which library it loaded.

`sopack model fetch` downloads the embedding model (~2.2 GB, sha256-verified against the
contract) into `~/Library/Caches/sopack/`; set `SOPACK_CACHE` to choose another directory.
`pack` never downloads anything.

## Platform support

- **macOS**: Apple Silicon (arm64) only, macOS 14 (Sonoma) or later — the platform the bundled
  ONNX Runtime 1.30.0 build and the release runner (`macos-14`) cover.
- **Linux** (x86_64, aarch64; glibc ≥ 2.35): `curl -fsSL https://raw.githubusercontent.com/theDeniZ/qdrant/main/sopack-rs/install.sh | sh`

Changing the bundled ORT version is not just a packaging decision: the calibration gate
(`sopack calibrate`, run by the release workflow) must pass with the new build before it ships.

## What tapping costs

`brew tap` makes a full git clone of this repo (Homebrew no longer supports shallow taps) and
`brew update` fetches it. The tracked tree is small today, so this is cheap. **Revisit before the
books move into Git LFS here** (`docs/BOOKS-LFS.md`): a Mac with `git-lfs` installed would download
every LFS object into the tap clone and burn LFS bandwidth quota. At that point either add a
`.lfsconfig` with `fetchexclude = *` (and override it in your own working clones), or move the
formula to a small `theDeniZ/homebrew-tap` repo after all.

## Releasing

1. Bump `[workspace.package] version` in `sopack-rs/Cargo.toml` — the only place the version lives.
2. Commit **first**, then tag that commit and push both:
   `git tag sopack-v0.9.0 && git push origin HEAD sopack-v0.9.0`.
   A tag that doesn't match `Cargo.toml` fails the `build` job and publishes nothing.

`.github/workflows/release-sopack.yml` then:

| Job | Runner | Does |
|---|---|---|
| `build` | 3 platforms (Linux x64, Linux aarch64, macOS arm64) | version check; unit tests, clippy, fmt; no-HTTP client audit; builds binary; tests `doctor --quick` and `calibrate` (on 2 platforms); tarballs with ORT runtime + contracts + license + README |
| `macOS-test` | macos-14 | copies macOS tarball into a throwaway local tap, runs `brew install`, runs `brew test` |
| `publish` | ubuntu | only if both passed: downloads verified ONNX Runtime 1.30.0 tarballs, extracts libraries/licenses into each per-target tarball, creates GitHub Release with all three, rewrites `url`/`sha256` in `Formula/sopack.rb` on the default branch and pushes that commit |

Never edit `url`/`sha256` by hand. If the default branch is protected against pushes from
`github-actions[bot]`, the last step fails after the release exists — allow the bot, or apply the
printed url/sha256 yourself.

## Upgrading from the old Python formula

If you have the old Python sopack installed:

```bash
brew uninstall sopack
rm -rf $(brew --prefix)/var/sopack         # Old venv, if any
brew install theDeniZ/qdrant/sopack        # Binary formula
sopack doctor
```

The old Python formula (`var/sopack/venv/` + `FASTEMBED_CACHE_PATH`) is gone. The binary is much
faster and self-contained; just keep `~/Library/Caches/sopack/` (the model cache).

## Testing the formula locally (without a release)

```bash
cd qdrant
V=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' sopack-rs/Cargo.toml)
TARGET="aarch64-apple-darwin"
TAR="sopack-$V-$TARGET.tar.gz"

# On a Mac with Rust + the repo's sopack-rs/ ready (same steps as the release workflow):
(cd sopack-rs && cargo build --release --locked -p sopack-cli)
S="sopack-$V-$TARGET"; mkdir -p "$S/bin" "$S/lib" "$S/LICENSE-THIRD-PARTY"
cp sopack-rs/target/release/sopack "$S/bin/"
cp -R sopack-rs/contracts "$S/contracts"; cp sopack-rs/README.md "$S/"
curl -fsSLO https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/onnxruntime-osx-arm64-1.30.0.tgz
tar -xzf onnxruntime-osx-arm64-1.30.0.tgz
cp -P onnxruntime-osx-arm64-1.30.0/lib/libonnxruntime* "$S/lib/"
cp onnxruntime-osx-arm64-1.30.0/LICENSE "$S/LICENSE-THIRD-PARTY/onnxruntime-LICENSE"
tar -czf "$TAR" "$S"

# Install via throwaway tap
brew tap-new --no-git local/sopack
TAP="$(brew --repository)/Library/Taps/local/homebrew-sopack"
SHA=$(shasum -a 256 "$TAR" | cut -d' ' -f1)
perl -pe "s|^  url \".*\"|  url \"file://$PWD/$TAR\"|; s|^  sha256 \".*\"|  sha256 \"$SHA\"|" \
  Formula/sopack.rb > "$TAP/Formula/sopack.rb"
brew install --verbose local/sopack/sopack
brew test --verbose local/sopack/sopack
sopack --version

# Cleanup
brew uninstall sopack
brew untap local/sopack
```
