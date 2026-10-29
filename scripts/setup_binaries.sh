#!/usr/bin/env bash
# ==============================================================================
# zapret2 Desktop GUI - Upstream Linux Binaries Setup Script
# ==============================================================================
# Linux counterpart of setup_binaries.ps1: fetches the pinned upstream
# bol-van/zapret2 release archive, verifies its SHA256, and installs the
# native Linux engine (nfqws2) into bin/ next to the Windows binaries.
#
# Upstream DOES ship Linux builds — verified against the v1.0.6 release
# on 2026-10-29: zapret2-v1.0.6.tar.gz contains
# binaries/linux-x86_64/nfqws2 (statically linked), and its SHA256
# matches upstream's own sha256sum.txt from the same release. No fork
# build is needed for the engine.
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$SCRIPT_DIR/../bin"
mkdir -p "$BIN_DIR"

# ==============================================================================
# PINNED UPSTREAM HASHES (supply-chain hardening, see SECURITY.md H-1)
# ------------------------------------------------------------------------------
# These SHA256 hashes pin the exact upstream release archive AND the exact
# engine binary extracted from it. The script aborts if either does not
# match, so a compromised upstream release asset cannot silently slip a
# trojaned engine into our build.
#
# HOW TO UPDATE when upstream publishes a new version:
#   1. Download the new archive manually from the official release page.
#   2. Compute its hash:  sha256sum <file>.tar.gz
#      and the hash of binaries/linux-x86_64/nfqws2 inside it.
#   3. Sanity-check both independently (second download, upstream's
#      published sha256sum.txt in the same release, community reports).
#   4. Update Url + Sha256 + BinarySha256 below in ONE commit and mention
#      the upstream version in the commit message.
# ==============================================================================
ZAPRET2_LABEL="bol-van/zapret2 v1.0.6"
ZAPRET2_URL="https://github.com/bol-van/zapret2/releases/download/v1.0.6/zapret2-v1.0.6.tar.gz"
ZAPRET2_SHA256="eb412aebc82cdef1f09b6aa9df45c8ff60fb88683b6663e6c4a9275d87ea2db4"
NFQWS2_SHA256="25049b2bff954c02051e1102a06d34f2f0f946b069d350c9065a86ecd88267c1"
NFQWS2_ARCHIVE_PATH="zapret2-v1.0.6/binaries/linux-x86_64/nfqws2"

verify_pinned_hash() {
    local path="$1" expected="$2" label="$3"
    local actual
    actual="$(sha256sum "$path" | awk '{print $1}')"
    if [ "$actual" != "$expected" ]; then
        echo "SECURITY: SHA256 mismatch for $label!" >&2
        echo "  expected: $expected" >&2
        echo "  actual:   $actual" >&2
        echo "The upstream archive may have been tampered with. Aborting build." >&2
        exit 1
    fi
    echo "    [+] SHA256 verified: $label"
}

download() {
    local url="$1" out="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fSL -o "$out" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$out" "$url"
    else
        echo "Neither curl nor wget is available to download $url" >&2
        exit 1
    fi
}

TARGET="$BIN_DIR/nfqws2"

if [ -f "$TARGET" ]; then
    # Already installed: verify the file on disk against the pin, not just
    # fresh downloads, so a stale or swapped binary fails the build with
    # the same SECURITY error as a bad download (mirrors the split-pin
    # behaviour in setup_binaries.ps1).
    echo "==> nfqws2 already present, verifying against pin..."
    verify_pinned_hash "$TARGET" "$NFQWS2_SHA256" "nfqws2 ($ZAPRET2_LABEL)"
else
    echo "==> Downloading $ZAPRET2_LABEL from GitHub..."
    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT
    ARCHIVE="$TMP_DIR/zapret2-v1.0.6.tar.gz"
    download "$ZAPRET2_URL" "$ARCHIVE"
    verify_pinned_hash "$ARCHIVE" "$ZAPRET2_SHA256" "$ZAPRET2_LABEL"
    tar -xzf "$ARCHIVE" -C "$TMP_DIR" "$NFQWS2_ARCHIVE_PATH"
    cp "$TMP_DIR/$NFQWS2_ARCHIVE_PATH" "$TARGET"
    chmod +x "$TARGET"
    verify_pinned_hash "$TARGET" "$NFQWS2_SHA256" "nfqws2 ($ZAPRET2_LABEL)"
    echo "    [+] Installed nfqws2 into $BIN_DIR"
fi

echo "==> Linux binaries ready: $TARGET"
