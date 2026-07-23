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
#   installer/output/wtmux_<version>-1_<arch>.deb
#   installer/output/wtmux-<version>-1.<arch>.rpm

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

if [[ $DO_DEB -eq 1 ]]; then
    echo ""
    if command -v cargo-deb >/dev/null 2>&1; then
        echo "Building .deb ..."
        cargo deb --no-build --output "$OUTPUT_DIR"
        DEB_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name "wtmux*.deb" -newer Cargo.toml -print -quit)
        [[ -z "$DEB_FILE" ]] && DEB_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name "wtmux*.deb" | sort | tail -1)
        echo "Package: $DEB_FILE"
    else
        echo "Warning: cargo-deb not installed, skipping .deb (cargo install cargo-deb)" >&2
    fi
fi

if [[ $DO_RPM -eq 1 ]]; then
    echo ""
    if command -v cargo-generate-rpm >/dev/null 2>&1; then
        echo "Building .rpm ..."
        cargo generate-rpm --output "$OUTPUT_DIR"
        RPM_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name "wtmux*.rpm" -newer Cargo.toml -print -quit)
        [[ -z "$RPM_FILE" ]] && RPM_FILE=$(find "$OUTPUT_DIR" -maxdepth 1 -name "wtmux*.rpm" | sort | tail -1)
        echo "Package: $RPM_FILE"
    else
        echo "Warning: cargo-generate-rpm not installed, skipping .rpm (cargo install cargo-generate-rpm)" >&2
    fi
fi

echo ""
echo "=== Done ==="
ls -lh "$OUTPUT_DIR"/wtmux* 2>/dev/null || true
