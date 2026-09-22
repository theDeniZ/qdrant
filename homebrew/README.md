# Homebrew tap for sopack

The `sopack` CLI is distributed via a Homebrew tap for macOS.

## Setup

1. **Create the tap repository** (one-time setup for the maintainer):
   ```bash
   # Create an empty public GitHub repository at https://github.com/theDeniZ/homebrew-tap
   # Clone it locally and add the Formula directory
   git clone https://github.com/theDeniZ/homebrew-tap.git
   cd homebrew-tap
   mkdir -p Formula
   cp ../qdrant/homebrew/sopack.rb Formula/
   git add Formula/sopack.rb
   git commit -m "Add sopack formula"
   git push origin main
   ```

2. **Update the formula** when releasing a new version:
   - Edit `Formula/sopack.rb` in the tap repo:
     - Update the `url` to point to the new release tarball
     - Update the `sha256` hash (run `sha256sum sopack-X.Y.Z.tar.gz`)
   - Commit and push

## Installation

### First install
```bash
brew install theDeniZ/tap/sopack
```

The formula installs `sopack` into a `libexec` virtualenv with all dependencies (fastembed, onnxruntime, numpy, tokenizers, huggingface-hub).

### Upgrade to a new version
```bash
brew upgrade theDeniZ/tap/sopack
```

Or install a specific version:
```bash
brew install theDeniZ/tap/sopack@1.0.0
```

## Verification

After installation, verify it works:
```bash
sopack --version
sopack --help
```

A `doctor` subcommand checks that the embedding model is cached and healthy before a 19-minute download:
```bash
sopack doctor
```

## Development

To test the formula locally before release:
```bash
brew tap theDeniZ/homebrew-tap --shallow
brew install --build-from-source theDeniZ/tap/sopack
```

Or test just the formula:
```bash
brew formulae inspect sopack.rb
```
