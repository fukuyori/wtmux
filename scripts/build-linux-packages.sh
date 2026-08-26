#!/usr/bin/env bash
# build-linux-packages.sh
# Build .deb and/or .rpm packages for wtmux from the release binary.
#
# Wraps 'cargo deb' and 'cargo generate-rpm', using the packaging metadata
# in Cargo.toml ([package.metadata.deb] / [package.metadata.generate-rpm]).
# Requires cargo-deb and/or cargo-generate-rpm:
#   cargo install cargo-deb cargo-generate-rpm
#
# Usage:
#   ./scripts/build-linux-packages.sh [options]
#
# Options:
#   --deb          Build only the .deb package
#   --rpm          Build only the .rpm package
#   --skip-build   Skip 'cargo build --release' (reuse existing binary)
#   -h, --help     Show this help
#
# With no --deb/--rpm flag, both packages are built (whichever tool is
# installed; a missing tool is skipped with a warning).
#
# Output:
#   installer/output/wtmux-<version>-linux-<arch>.deb
#   installer/output/wtmux-<version>-linux-<arch>.rpm

set -euo pipefail
# Run from the repository root regardless of where the script lives
cd "$(dirname "$0")/.."

DO_DEB=0
DO_RPM=0
SKIP_BUILD=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --deb)        DO_DEB=1; shift ;;
        --rpm)        DO_RPM=1; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        -h|--help)    grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Error: unknown option: $1" >&2; exit 1 ;;
    esac
done

# Default: build both when neither flag was given
if [[ $DO_DEB -eq 0 && $DO_RPM -eq 0 ]]; then
    DO_DEB=1
    DO_RPM=1
fi

VERSION=$(sed -n 's/^version = "\([0-9.]*\)"/\1/p' Cargo.toml | head -1)
echo "=== wtmux Linux Package Build ==="
echo "Version: $VERSION"

if [[ $SKIP_BUILD -eq 0 ]]; then
    echo ""
    echo "Building release binary ..."
    cargo build --release
else
    if [[ ! -f target/release/wtmux ]]; then
        echo "Error: target/release/wtmux not found (need a build; omit --skip-build)." >&2
        exit 1
    fi
    echo "Skipping build (--skip-build), using existing target/release/wtmux"
fi

OUTPUT_DIR=installer/output
mkdir -p "$OUTPUT_DIR"

# Each package is built into an empty staging directory, then moved to its
# release name. Globbing $OUTPUT_DIR directly would match packages left by
# earlier runs and rename an already-renamed file a second time.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

if [[ $DO_DEB -eq 1 ]]; then
    echo ""
    if command -v cargo-deb >/dev/null 2>&1; then
        echo "Building .deb ..."
        mkdir -p "$STAGE/deb"
        cargo deb --no-build --output "$STAGE/deb"
        DEB_FILE=$(find "$STAGE/deb" -maxdepth 1 -name "*.deb" -print -quit)
        if [[ -z "$DEB_FILE" ]]; then
            echo "Error: cargo deb produced no .deb package." >&2
            exit 1
        fi
        # cargo-deb names its output wtmux_<version>-1_<arch>.deb; move it to
        # the common wtmux-<version>-<os>-<arch> release naming
        DEB_ARCH=$(basename "$DEB_FILE" .deb)
        DEB_ARCH=${DEB_ARCH##*_}
        DEB_OUT="$OUTPUT_DIR/wtmux-$VERSION-linux-$DEB_ARCH.deb"
        mv -f "$DEB_FILE" "$DEB_OUT"
        echo "Package: $DEB_OUT"
    else
        echo "Warning: cargo-deb not installed, skipping .deb (cargo install cargo-deb)" >&2
    fi
fi

if [[ $DO_RPM -eq 1 ]]; then
    echo ""
    if command -v cargo-generate-rpm >/dev/null 2>&1; then
        echo "Building .rpm ..."
        mkdir -p "$STAGE/rpm"
        cargo generate-rpm --output "$STAGE/rpm"
        RPM_FILE=$(find "$STAGE/rpm" -maxdepth 1 -name "*.rpm" -print -quit)
        if [[ -z "$RPM_FILE" ]]; then
            echo "Error: cargo generate-rpm produced no .rpm package." >&2
            exit 1
        fi
        # cargo-generate-rpm names its output wtmux-<version>-1.<arch>.rpm; move
        # it to the common wtmux-<version>-<os>-<arch> release naming
        RPM_ARCH=$(basename "$RPM_FILE" .rpm)
        RPM_ARCH=${RPM_ARCH##*.}
        RPM_OUT="$OUTPUT_DIR/wtmux-$VERSION-linux-$RPM_ARCH.rpm"
        mv -f "$RPM_FILE" "$RPM_OUT"
        echo "Package: $RPM_OUT"
    else
        echo "Warning: cargo-generate-rpm not installed, skipping .rpm (cargo install cargo-generate-rpm)" >&2
    fi
fi

echo ""
echo "=== Done ==="
ls -lh "$OUTPUT_DIR"/wtmux* 2>/dev/null || true
