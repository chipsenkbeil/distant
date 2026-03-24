#!/bin/bash
set -euo pipefail

BUNDLE="target/Distant.app"
BINARY="${1:-target/release/distant}"
IDENTITY="${CODESIGN_IDENTITY:--}"  # ad-hoc default, override for distribution
ENTITLEMENTS="${ENTITLEMENTS:-resources/macos/distant-dev.entitlements}"

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

# Sign extension first, then app (order matters for Apple)
codesign -s "$IDENTITY" -f --entitlements "$ENTITLEMENTS" \
    "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex"
codesign -s "$IDENTITY" -f --entitlements "$ENTITLEMENTS" \
    "$BUNDLE"

echo "Bundle created at $BUNDLE"
echo "Signed with identity: $IDENTITY"
echo "Entitlements: $ENTITLEMENTS"
