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
  url "https://github.com/theDeniZ/qdrant/releases/download/sopack-v0.1.2/sopack-0.1.2.tar.gz"
  sha256 "e145e95800444a72c26877f6119428362eb2b4fea0643c977635f8a1f6504368"

  # NOTE: set the license to match the repository's LICENSE file before
  # publishing. Left unset deliberately rather than guessed.

  # requirements.lock pins onnxruntime 1.30.0, whose only macOS wheel is
  # macosx_14_0_arm64 — there is none for Intel Macs or macOS < 14.
  depends_on arch: :arm64
  depends_on macos: :sonoma
  depends_on "python@3.11"

  # The venv lives OUTSIDE the keg, in var/. Homebrew rewrites the install
  # name of every Mach-O file in a keg after `install`, and prebuilt wheels are
  # not linked with -headerpad_max_install_names: py_rust_stemmers' .so fails
  # that rewrite ("Updated load commands do not fit in the header") and the
  # install exits 1. So `install` only collects wheels (zips, never relinked)
  # and `post_install` — which runs after the relocation pass — builds the venv
  # from them, offline.
  def venv
    var/"sopack/venv"
  end

  def install
    python = Formula["python@3.11"].opt_bin/"python3.11"
    wheels = libexec/"wheels"

    # Throwaway build venv, only to run pip; nothing from it is installed.
    system python, "-m", "venv", buildpath/"build-venv"
    pip = buildpath/"build-venv/bin/pip"
    # The pinned closure is the contract: the embedding library's exact version
    # determines the vector space every pack is written into (contract.py).
    # --only-binary: post_install has no network and no build toolchain.
    # --require-hashes is added here once the lockfile carries hashes.
    system pip, "download", "--only-binary", ":all:", "--dest", wheels,
           "-r", "sopack/requirements.lock"
    system pip, "wheel", "--no-deps", "--wheel-dir", wheels, "./sopack"
    libexec.install "sopack/requirements.lock"

    (bin/"sopack").write <<~SH
      #!/bin/bash
      if [ ! -x "#{venv}/bin/python" ]; then
        echo "sopack: #{venv} is missing — run: brew postinstall sopack" >&2
        exit 1
      fi
      exec "#{venv}/bin/python" -m sopack.cli "$@"
    SH
    chmod 0755, bin/"sopack"
  end

  def post_install
    python = Formula["python@3.11"].opt_bin/"python3.11"
    # Rebuilt from scratch on every install/upgrade so no older version's
    # packages survive in it.
    rm_r venv if venv.exist?
    system python, "-m", "venv", venv
    system venv/"bin/pip", "install", "--no-index", "--find-links", libexec/"wheels",
           "-r", libexec/"requirements.lock"
    system venv/"bin/pip", "install", "--no-index", "--no-deps",
           *Dir[libexec/"wheels/sopack-*.whl"]
  end

  def caveats
    <<~EOS
      The Python environment lives in #{venv} (outside the keg, see the
      formula). `brew uninstall sopack` leaves it behind; remove it with:
        rm -rf #{var}/sopack

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
    system venv/"bin/python", "-c", "import fastembed, numpy, onnxruntime, sopack.pack"
  end
end
