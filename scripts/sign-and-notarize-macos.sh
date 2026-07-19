#!/usr/bin/env bash
# sign-and-notarize-macos.sh
# Code-sign, notarize and staple the wtmux macOS .pkg installer.
#
# Takes the release binary from 'cargo build --release', signs it (Developer ID
# Application, hardened runtime), wraps it into a signed .pkg (Developer ID
# Installer), submits the .pkg for notarization and staples the ticket.
#
# Usage:
#   ./scripts/sign-and-notarize-macos.sh [options]
#
# Options:
#   --binary PATH            Binary to sign (default: target/release/wtmux)
#   --identity ID            "Developer ID Application: ..." identity
#                            (default: $WTMUX_SIGN_IDENTITY, else auto-detect)
#   --installer-identity ID  "Developer ID Installer: ..." identity
#                            (default: $WTMUX_INSTALLER_IDENTITY, else auto-detect)
#   --profile NAME           notarytool keychain profile
#                            (default: $WTMUX_NOTARY_PROFILE, else auto-detect
#                            by probing common profile names)
#   --apple-id ID            Apple ID          (default: $WTMUX_APPLE_ID)
#   --team-id ID             Team ID           (default: $WTMUX_TEAM_ID)
#   --password PW            App-specific pass (default: $WTMUX_APP_PASSWORD)
#   --skip-notarize          Sign only, skip notarization and stapling
#
# Credentials: either store a keychain profile once:
#   xcrun notarytool store-credentials notarytool --apple-id <id> --team-id <team> --password <app-pw>
# (profiles named 'notarytool', 'wtmux' or 'notary' are auto-detected; other
# names need --profile), or pass --apple-id/--team-id/--password directly.
#
# Output:
#   installer/output/wtmux-<version>-macos-<arch>.pkg  (signed, notarized, stapled)

set -euo pipefail
# Run from the repository root regardless of where the script lives
cd "$(dirname "$0")/.."

BINARY=target/release/wtmux
IDENTITY="${WTMUX_SIGN_IDENTITY:-}"
INSTALLER_IDENTITY="${WTMUX_INSTALLER_IDENTITY:-}"
PROFILE="${WTMUX_NOTARY_PROFILE:-}"
APPLE_ID="${WTMUX_APPLE_ID:-}"
TEAM_ID="${WTMUX_TEAM_ID:-}"
APP_PASSWORD="${WTMUX_APP_PASSWORD:-}"
SKIP_NOTARIZE=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary)             BINARY="$2"; shift 2 ;;
        --identity)           IDENTITY="$2"; shift 2 ;;
        --installer-identity) INSTALLER_IDENTITY="$2"; shift 2 ;;
        --profile)            PROFILE="$2"; shift 2 ;;
        --apple-id)           APPLE_ID="$2"; shift 2 ;;
        --team-id)            TEAM_ID="$2"; shift 2 ;;
        --password)           APP_PASSWORD="$2"; shift 2 ;;
        --skip-notarize)      SKIP_NOTARIZE=1; shift ;;
        -h|--help)            grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Error: unknown option: $1" >&2; exit 1 ;;
    esac
done

echo "=== wtmux macOS Sign & Notarize ==="

if [[ ! -f "$BINARY" ]]; then
    echo "Error: binary not found: $BINARY" >&2
    echo "Run 'cargo build --release' first." >&2
    exit 1
fi

VERSION=$(sed -n 's/^version = "\([0-9.]*\)"/\1/p' Cargo.toml | head -1)
ARCHS=$(lipo -archs "$BINARY")
case "$ARCHS" in
    *arm64*x86_64*|*x86_64*arm64*) ARCH=universal ;;
    arm64)  ARCH=arm64 ;;
    x86_64) ARCH=x86_64 ;;
    *)      ARCH=unknown ;;
esac
echo "Binary:  $BINARY ($ARCHS)"
echo "Version: $VERSION"

# ── Resolve signing identities ───────────────────────────────────────────
if [[ -z "$IDENTITY" ]]; then
    IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
        | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
    if [[ -z "$IDENTITY" ]]; then
        echo "Error: no 'Developer ID Application' identity found in keychain." >&2
        echo "Specify one with --identity or \$WTMUX_SIGN_IDENTITY." >&2
        exit 1
    fi
    echo "Auto-detected identity: $IDENTITY"
fi

if [[ -z "$INSTALLER_IDENTITY" ]]; then
    INSTALLER_IDENTITY=$(security find-identity -v 2>/dev/null \
        | sed -n 's/.*"\(Developer ID Installer: [^"]*\)".*/\1/p' | head -1)
    if [[ -z "$INSTALLER_IDENTITY" ]]; then
        echo "Error: no 'Developer ID Installer' identity found in keychain." >&2
        echo "Specify one with --installer-identity or \$WTMUX_INSTALLER_IDENTITY." >&2
        exit 1
    fi
    echo "Auto-detected installer identity: $INSTALLER_IDENTITY"
fi

# ── Notary credential check (before doing any work) ──────────────────────
notary_args=()
if [[ $SKIP_NOTARIZE -eq 0 ]]; then
    if [[ -z "$PROFILE" && -z "$APPLE_ID" && -z "$TEAM_ID" && -z "$APP_PASSWORD" ]]; then
        # notarytool profiles live in the data-protection keychain, which the
        # `security` CLI cannot enumerate — probe common profile names instead.
        # A missing profile fails instantly, so this only costs one network
        # round-trip for the profile that exists.
        for candidate in notarytool wtmux notary; do
            if ! xcrun notarytool history --keychain-profile "$candidate" 2>&1 \
                | grep -q "No Keychain password item found"; then
                PROFILE="$candidate"
                echo "Auto-detected notary profile: $PROFILE"
                break
            fi
        done
    fi
    if [[ -n "$PROFILE" ]]; then
        notary_args=(--keychain-profile "$PROFILE")
    elif [[ -n "$APPLE_ID" && -n "$TEAM_ID" && -n "$APP_PASSWORD" ]]; then
        notary_args=(--apple-id "$APPLE_ID" --team-id "$TEAM_ID" --password "$APP_PASSWORD")
    else
        echo "Error: no notarization credentials." >&2
        echo "Use --profile NAME (see 'xcrun notarytool store-credentials')" >&2
        echo "or --apple-id/--team-id/--password." >&2
        exit 1
    fi
fi

# ── 1. Code-sign the binary (hardened runtime) ───────────────────────────
echo ""
echo "Signing binary ..."
codesign --force --timestamp --options runtime --sign "$IDENTITY" "$BINARY"
codesign --verify --strict --verbose=2 "$BINARY"
echo "Signature OK"

# ── 2. Build the signed .pkg (installs to /usr/local/bin/wtmux) ──────────
echo ""
echo "Building signed .pkg ..."
OUTPUT_DIR=installer/output
PKG_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/wtmux-pkgroot.XXXXXX")
mkdir -p "$PKG_ROOT/usr/local/bin"
cp -X "$BINARY" "$PKG_ROOT/usr/local/bin/wtmux"
# Strip extended attributes so pkgbuild does not emit AppleDouble (._*) files
xattr -cr "$PKG_ROOT"

PKG_FILE="$OUTPUT_DIR/wtmux-$VERSION-macos-$ARCH.pkg"
rm -f "$PKG_FILE"
pkgbuild \
    --root "$PKG_ROOT" \
    --identifier org.spumoni.wtmux \
    --version "$VERSION" \
    --install-location / \
    --sign "$INSTALLER_IDENTITY" \
    "$PKG_FILE"
rm -rf "$PKG_ROOT"
echo "Package: $PKG_FILE"

# ── 3. Notarize and staple ───────────────────────────────────────────────
if [[ $SKIP_NOTARIZE -eq 1 ]]; then
    echo ""
    echo "Skipping notarization (--skip-notarize)"
else
    echo ""
    echo "Submitting for notarization (this may take a few minutes) ..."
    result=$(xcrun notarytool submit "$PKG_FILE" "${notary_args[@]}" --wait --output-format json)
    status=$(echo "$result" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))')
    submission_id=$(echo "$result" | /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin).get("id",""))')
    echo "Submission $submission_id: $status"
    if [[ "$status" != "Accepted" ]]; then
        echo "Notarization failed — fetching log:" >&2
        xcrun notarytool log "$submission_id" "${notary_args[@]}" >&2 || true
        exit 1
    fi

    echo "Stapling ticket ..."
    xcrun stapler staple "$PKG_FILE"

    echo "Verifying with Gatekeeper ..."
    spctl --assess --type install -vv "$PKG_FILE"
fi

SIZE=$(du -h "$PKG_FILE" | cut -f1)
echo ""
echo "=== Done ==="
echo "Distribution package: $PKG_FILE ($SIZE)"
