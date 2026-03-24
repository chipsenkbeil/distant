#!/bin/bash
set -euo pipefail

BUNDLE="target/Distant.app"
BINARY="${1:-target/release/distant}"
IDENTITY="${CODESIGN_IDENTITY:--}"  # ad-hoc default, override for distribution
ENTITLEMENTS="${ENTITLEMENTS:-}"    # empty = no entitlements (safe for ad-hoc)

rm -rf "$BUNDLE"

mkdir -p "$BUNDLE/Contents/MacOS"
mkdir -p "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/MacOS"

cp "$BINARY" "$BUNDLE/Contents/MacOS/distant"

# Symlink — appex uses same binary, avoids duplication
ln -s ../../../MacOS/distant \
    "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/MacOS/distant"

cp resources/macos/Info.plist "$BUNDLE/Contents/"
cp resources/macos/Extension-Info.plist \
   "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/Info.plist"

# Build codesign flags
SIGN_FLAGS=(-s "$IDENTITY" -f)
if [ -n "$ENTITLEMENTS" ]; then
    SIGN_FLAGS+=(--entitlements "$ENTITLEMENTS")
fi

# Sign extension first, then app (order matters for Apple).
# The appex contains a symlink to the main binary, so we sign the
# appex bundle (not the symlink itself) — codesign handles this.
codesign "${SIGN_FLAGS[@]}" \
    "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex"
codesign "${SIGN_FLAGS[@]}" \
    "$BUNDLE"

echo "Bundle created at $BUNDLE"
echo "Signed with identity: $IDENTITY"
if [ -n "$ENTITLEMENTS" ]; then
    echo "Entitlements: $ENTITLEMENTS"
else
    echo "Entitlements: none (ad-hoc)"
fi
