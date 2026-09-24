# Homebrew: this repository is the tap

There is no separate `homebrew-tap` repository. Homebrew can tap any git URL, and it
finds formulae in a top-level `Formula/` directory, so `theDeniZ/qdrant` serves
`sopack` directly.

## Install (Apple Silicon, macOS 14+)

```bash
brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant   # once — the URL is required,
                                                               # the repo isn't named homebrew-*
brew install theDeniZ/qdrant/sopack
sopack doctor                                                  # loads the model once (~30 s)
```

Upgrade: `brew update && brew upgrade sopack`.

The Python environment is built by the formula's `post_install` into
`$(brew --prefix)/var/sopack/venv`, **not** inside the keg: Homebrew relinks every
Mach-O file in a keg after install, and prebuilt wheels (first casualty:
`py_rust_stemmers`) lack the header padding for that, which fails the install.
The keg only holds the downloaded wheels; `post_install` installs them offline.
Consequences: if `sopack` says the venv is missing, run `brew postinstall sopack`;
`brew uninstall sopack` leaves the venv behind — `rm -rf $(brew --prefix)/var/sopack`.

Intel Macs and macOS < 14 are not supported: the pinned `onnxruntime==1.30.0`
(`sopack/requirements.lock`) ships only a `macosx_14_0_arm64` wheel. Loosening that
pin is not a packaging decision — the embedding stack's exact versions define the
vector space (`contract.py`).

Set `FASTEMBED_CACHE_PATH` to a stable directory (the formula's caveats print a
suggestion): fastembed's default cache is `$TMPDIR/fastembed_cache`, which macOS may
purge, costing another ~2.2 GB download.

## What tapping costs

`brew tap` makes a full git clone of this repo (Homebrew no longer supports shallow
taps) and `brew update` fetches it. The tracked tree is small today (`pd-books/` is
untracked), so this is cheap. **Revisit before the books move into Git LFS here**
(`docs/BOOKS-LFS.md`): a Mac with `git-lfs` installed would download every LFS object
into the tap clone and burn LFS bandwidth quota. At that point either add a
`.lfsconfig` with `fetchexclude = *` (and override it in your own working clones), or
move the formula to a small `theDeniZ/homebrew-tap` repo after all.

## Releasing

1. Bump `__version__` in `sopack/__init__.py` — the only place the version lives
   (`pyproject.toml` reads it from there).
2. Commit **first**, then tag that commit and push both:
   `git tag sopack-v0.1.2 && git push origin HEAD sopack-v0.1.2`.
   A tag that doesn't match `__version__` fails the `build` job and publishes nothing.

`.github/workflows/release-sopack.yml` then:

| Job | Runner | Does |
|---|---|---|
| `build` | ubuntu | tag = `sopack.__version__`; installs from the lockfile exactly as the formula does; unit tests; `sopack --version`; `git archive` of `sopack/` → `sopack-<v>.tar.gz` + sha256 |
| `brew` | macos-14 (arm64) | copies `Formula/sopack.rb` into a throwaway local tap pointing at the built tarball (`file://`), runs `brew install`, asserts `post_install` built the venv, runs `brew test` |
| `publish` | ubuntu | only if both passed: creates the GitHub Release with the tarball, then rewrites `url`/`sha256` in `Formula/sopack.rb` on the default branch and pushes that commit |

Never edit `url`/`sha256` by hand. If the default branch is protected against pushes
from `github-actions[bot]`, the last step fails after the release exists — allow the
bot, or apply the printed url/sha256 yourself.

## Testing the formula locally (without a release)

```bash
cd qdrant
V=$(python3 -c "import re;print(re.search(r'\"(.+)\"', open('sopack/__init__.py').read().split('__version__')[1]).group(1))")
tar -czf /tmp/sopack-$V.tar.gz --exclude=__pycache__ -s "|^|sopack-$V/|" sopack
brew tap-new --no-git local/sopack
TAP="$(brew --repository)/Library/Taps/local/homebrew-sopack"
perl -pe "s|^  url \".*\"|  url \"file:///tmp/sopack-$V.tar.gz\"|; s|^  sha256 \".*\"|  sha256 \"$(shasum -a 256 /tmp/sopack-$V.tar.gz | cut -d' ' -f1)\"|" \
  Formula/sopack.rb > "$TAP/Formula/sopack.rb"
brew install --verbose local/sopack/sopack && brew test local/sopack/sopack
brew uninstall sopack && brew untap local/sopack && rm -rf "$(brew --prefix)/var/sopack"
```
