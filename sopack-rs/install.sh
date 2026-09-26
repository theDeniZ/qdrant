#!/bin/sh
# sopack installer for Linux (also works on macOS)
#
# Detects arch/OS, downloads the latest sopack release from GitHub,
# verifies the sha256, extracts to PREFIX (default ~/.local), and
# installs symlinks into bin/.
#
# Usage:
#   curl -s https://github.com/theDeniZ/qdrant/raw/main/sopack-rs/install.sh | sh
#
# Environment:
#   SOPACK_VERSION=1.0.0    # Specific version (default: latest)
#   PREFIX=$HOME/.local     # Install base dir (default: $HOME/.local)

set -eu

# Detect OS and architecture
detect_os_arch() {
    case "$(uname -s)" in
        Linux)
            OS="linux"
            ;;
        Darwin)
            OS="macos"
            ;;
        *)
            printf "Error: unsupported OS %s\n" "$(uname -s)" >&2
            exit 1
            ;;
    esac

    case "$(uname -m)" in
        x86_64)
            ARCH="x64"
            ;;
        aarch64 | arm64)
            ARCH="aarch64"
            ;;
        *)
            printf "Error: unsupported architecture %s\n" "$(uname -m)" >&2
            exit 1
            ;;
    esac

    case "$OS" in
        linux)
            TARGET="$ARCH-unknown-linux-gnu"
            ;;
        macos)
            if [ "$ARCH" != "aarch64" ]; then
                printf "Error: sopack requires macOS ARM64; use 'brew install' instead\n" >&2
                exit 1
            fi
            TARGET="aarch64-apple-darwin"
            ;;
    esac

    printf "Detected: %s %s (%s)\n" "$OS" "$ARCH" "$TARGET" >&2
}

# Fetch latest release version from GitHub API
get_latest_version() {
    if [ -n "${SOPACK_VERSION:-}" ]; then
        printf "%s\n" "$SOPACK_VERSION"
        return
    fi

    LATEST_TAG=$(curl -s "https://api.github.com/repos/theDeniZ/qdrant/releases" \
        | grep -o '"tag_name": "sopack-v[^"]*"' | head -1 | cut -d'"' -f4 | sed 's/sopack-v//')

    if [ -z "$LATEST_TAG" ]; then
        printf "Error: could not determine latest sopack version from GitHub API\n" >&2
        exit 1
    fi

    printf "%s\n" "$LATEST_TAG"
}

# Download file and verify SHA256
download_and_verify() {
    local url="$1"
    local sha_expected="$2"
    local file="$3"

    printf "Downloading %s...\n" "$file" >&2
    if ! curl -L -o "$file" "$url"; then
        printf "Error: failed to download %s\n" "$url" >&2
        exit 1
    fi

    # Use sha256sum if available, else shasum -a 256
    if command -v sha256sum >/dev/null 2>&1; then
        sha_actual=$(sha256sum "$file" | cut -d' ' -f1)
    elif command -v shasum >/dev/null 2>&1; then
        sha_actual=$(shasum -a 256 "$file" | cut -d' ' -f1)
    else
        printf "Error: neither sha256sum nor shasum found\n" >&2
        exit 1
    fi

    if [ "$sha_actual" != "$sha_expected" ]; then
        printf "Error: SHA256 mismatch for %s\n  expected: %s\n  got: %s\n" "$file" "$sha_expected" "$sha_actual" >&2
        rm -f "$file"
        exit 1
    fi

    printf "Verified: %s\n" "$file" >&2
}

main() {
    PREFIX="${PREFIX:-$HOME/.local}"

    # Create temp dir
    TMPDIR=$(mktemp -d)
    trap "rm -rf $TMPDIR" EXIT

    cd "$TMPDIR"

    detect_os_arch
    VERSION=$(get_latest_version)
    printf "Installing sopack %s\n" "$VERSION" >&2

    # GitHub Release URLs
    TARBALL="sopack-$VERSION-$TARGET.tar.gz"
    SHA256_FILE="sopack-$VERSION-$TARGET.tar.gz.sha256"
    RELEASE_URL="https://github.com/theDeniZ/qdrant/releases/download/sopack-v$VERSION"

    # Download tarball SHA256 file first
    SHA256_URL="$RELEASE_URL/$SHA256_FILE"
    if ! curl -L -o "$SHA256_FILE" "$SHA256_URL"; then
        printf "Error: failed to download %s\n" "$SHA256_URL" >&2
        exit 1
    fi

    SHA256_EXPECTED=$(cut -d' ' -f1 "$SHA256_FILE")

    # Download and verify tarball
    TARBALL_URL="$RELEASE_URL/$TARBALL"
    download_and_verify "$TARBALL_URL" "$SHA256_EXPECTED" "$TARBALL"

    # Extract
    printf "Extracting...\n" >&2
    tar -xzf "$TARBALL"

    # Install
    INSTALL_DIR="$PREFIX/opt/sopack-$VERSION"
    mkdir -p "$INSTALL_DIR" "$PREFIX/bin"

    # Copy everything from the extracted tarball
    EXTRACTED_DIR=$(ls -d sopack-*/)
    cp -r "$EXTRACTED_DIR"/* "$INSTALL_DIR/"

    # Create symlink
    rm -f "$PREFIX/bin/sopack"
    ln -s "../opt/sopack-$VERSION/bin/sopack" "$PREFIX/bin/sopack"

    printf "\n" >&2
    printf "Installation complete!\n" >&2
    printf "\n" >&2
    printf "To start using sopack:\n" >&2
    printf "  1. Ensure %s/bin is in your PATH\n" "$PREFIX" >&2
    printf "  2. Run: sopack model fetch    (downloads ~2.2 GB, one-time)\n" >&2
    printf "  3. Run: sopack doctor         (verifies setup)\n" >&2
    printf "\n" >&2
    printf "Version: %s\n" "$VERSION" >&2
    printf "Installed to: %s\n" "$INSTALL_DIR" >&2
}

main
