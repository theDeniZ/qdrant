# Homebrew formula for the sopack CLI.
#
# This repository IS the tap — there is no separate homebrew-tap repo:
#   brew tap theDeniZ/qdrant https://github.com/theDeniZ/qdrant
#   brew install theDeniZ/qdrant/sopack
#
# `url` and `sha256` are rewritten by .github/workflows/release-sopack.yml on
# every `sopack-v*` tag, and only committed after a real `brew install` + `brew
# test` of the new release passed on a macOS runner. Do not edit them by hand.
class Sopack < Formula
  desc "Prepare and verify corpus import packs for the bible-sop Qdrant collections"
  homepage "https://github.com/theDeniZ/qdrant"
  url "https://github.com/theDeniZ/qdrant/releases/download/sopack-v0.1.0/sopack-0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  # NOTE: set the license to match the repository's LICENSE file before
  # publishing. Left unset deliberately rather than guessed.

  # requirements.lock pins onnxruntime 1.30.0, whose only macOS wheel is
  # macosx_14_0_arm64 — there is none for Intel Macs or macOS < 14.
  depends_on arch: :arm64
  depends_on macos: :sonoma
  depends_on "python@3.11"

  def install
    python = Formula["python@3.11"].opt_bin/"python3.11"
    venv = libexec/"venv"

    # A dedicated venv keeps the onnxruntime/fastembed stack out of the user's
    # own site-packages, and pins it to the interpreter the lockfile was
    # resolved against.
    system python, "-m", "venv", venv
    system venv/"bin/pip", "install", "--upgrade", "pip"

    # The pinned closure is the contract: the embedding library's exact version
    # determines the vector space every pack is written into (contract.py).
    # --require-hashes is added here once the lockfile carries hashes.
    system venv/"bin/pip", "install", "-r", "sopack/requirements.lock"
    system venv/"bin/pip", "install", "--no-deps", "./sopack"

    (bin/"sopack").write <<~SH
      #!/bin/bash
      exec "#{venv}/bin/python" -m sopack.cli "$@"
    SH
    chmod 0755, bin/"sopack"
  end

  def caveats
    <<~EOS
      The first `sopack pack` downloads the ~2.2 GB embedding model into
      fastembed's cache, which defaults to a temp directory macOS may purge.
      Keep it by exporting a stable location, e.g. in ~/.zprofile:
        export FASTEMBED_CACHE_PATH="$HOME/Library/Caches/sopack/fastembed"
      Then run `sopack doctor` (it loads the model once) before a real pack.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/sopack --version")
    # Imports every dependency in the pinned closure without loading the model.
    system libexec/"venv/bin/python", "-c", "import fastembed, numpy, onnxruntime, sopack.pack"
  end
end
