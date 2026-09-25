# Homebrew binary formula for the sopack CLI (Rust).
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
  # url and sha256 are rewritten by .github/workflows/release-sopack.yml on every release tag
  url "https://github.com/theDeniZ/qdrant/releases/download/sopack-v0.9.0/sopack-0.9.0-aarch64-apple-darwin.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  # Prebuilt Rust binary + Microsoft ONNX Runtime 1.30.0 (bundled, the version
  # the calibration gate was measured with). No Python, no venv, no post_install.
  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    # Tarball layout (Homebrew cd's into sopack-<v>-aarch64-apple-darwin/):
    #   bin/sopack  lib/libonnxruntime*.dylib  contracts/  LICENSE-THIRD-PARTY/  README.md
    #
    # bin/ and lib/ go into libexec, NOT into the keg's lib/: a keg lib/ is
    # linked into $(brew --prefix)/lib, where libonnxruntime.dylib would collide
    # with Homebrew's own `onnxruntime` formula (a different version). The binary
    # canonicalises its own path through the bin/ symlink and loads
    # libexec/lib/libonnxruntime.dylib from `<real exe dir>/../lib`.
    #
    # Homebrew rewrites the install id of Mach-O files in the keg. If a future
    # ORT dylib lacks header padding for that, `brew install` fails in the
    # release workflow's brew job, before anything is published.
    libexec.install "bin", "lib"
    bin.install_symlink libexec/"bin/sopack"
    pkgshare.install "contracts"
    doc.install "README.md", Dir["LICENSE-THIRD-PARTY/*"]
  end

  def caveats
    <<~EOS
      Download and cache the embedding model (~2.2 GB):
        sopack model fetch

      This caches it in ~/Library/Caches/sopack (override with SOPACK_CACHE).

      Verify the setup:
        sopack doctor
    EOS
  end

  test do
    assert_match "sopack #{version}", shell_output("#{bin}/sopack --version")
    # No model in the test sandbox: --quick reports that as a warning and must
    # still exit 0; it also proves the bundled ONNX Runtime is found.
    output = shell_output("#{bin}/sopack doctor --quick --json --progress none")
    assert_match(/"onnxruntime".*?"1\.30\.0 at /m, output)
  end
end
