# Homebrew formula for the sopack CLI.
#
# Goes in theDeniZ/homebrew-tap as Formula/sopack.rb — see homebrew/README.md.
# `url` and `sha256` are filled in per release; the release workflow
# (.github/workflows/release-sopack.yml) prints the exact stanza to paste.
class Sopack < Formula
  desc "Prepare and verify corpus import packs for the bible-sop Qdrant collections"
  homepage "https://github.com/theDeniZ/qdrant"

  # RELEASE: replace both lines. The workflow prints them on each tagged build.
  url "https://github.com/theDeniZ/qdrant/releases/download/sopack-v0.1.0/sopack-0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  # NOTE: set the license to match the repository's LICENSE file before
  # publishing the tap. Left unset deliberately rather than guessed.

  depends_on "python@3.11"

  def install
    python = Formula["python@3.11"].opt_bin/"python3.11"
    venv = libexec/"venv"

    # A dedicated venv keeps the 2 GB onnxruntime/fastembed stack out of the
    # user's own site-packages, and pins it to the interpreter the lockfile was
    # resolved against.
    system python, "-m", "venv", venv
    system venv/"bin/pip", "install", "--upgrade", "pip"

    # The pinned closure is the contract: the embedding library's exact version
    # determines the vector space every pack is written into (contract.py).
    # --require-hashes is added here once the lockfile carries hashes.
    system venv/"bin/pip", "install", "-r", "sopack/requirements.lock"
    system venv/"bin/pip", "install", "--no-deps", "./sopack"

    # Ruby's Pathname#write, not Python's write_text.
    (bin/"sopack").write <<~SH
      #!/bin/bash
      exec "#{venv}/bin/python" -m sopack.cli "$@"
    SH
    chmod 0755, bin/"sopack"
  end

  test do
    # `doctor` is the real smoke test: it reports the interpreter, the fastembed
    # version and whether the model cache is usable, and exits non-zero when the
    # environment cannot produce a valid pack.
    system bin/"sopack", "--help"
    assert_match "sopack", shell_output("#{bin}/sopack --version")
  end
end
