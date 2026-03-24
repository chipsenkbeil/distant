#!/bin/bash
set -euo pipefail

BUNDLE="target/Distant.app"
BINARY="${1:-target/release/distant}"
IDENTITY="${CODESIGN_IDENTITY:--}"  # ad-hoc default, override for distribution

# Entitlements for the main app and the appex extension.
# For ad-hoc (dev) builds, default to dev entitlements so the
# FileProvider extension can load via testing-mode.
# For distribution, set ENTITLEMENTS / APPEX_ENTITLEMENTS to the
# production entitlements files.
ENTITLEMENTS="${ENTITLEMENTS:-resources/macos/distant-dev.entitlements}"
APPEX_ENTITLEMENTS="${APPEX_ENTITLEMENTS:-resources/macos/distant-appex-dev.entitlements}"
APP_PROFILE="${APP_PROFILE:-}"
APPEX_PROFILE="${APPEX_PROFILE:-}"

rm -rf "$BUNDLE"

mkdir -p "$BUNDLE/Contents/MacOS"
mkdir -p "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/MacOS"

cp "$BINARY" "$BUNDLE/Contents/MacOS/distant"

# Hardlink — appex uses same binary without duplicating disk space.
# (Symlinks are rejected by codesign for the main executable.)
ln "$BUNDLE/Contents/MacOS/distant" \
    "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/MacOS/distant"

cp resources/macos/Info.plist "$BUNDLE/Contents/"
cp resources/macos/Extension-Info.plist \
   "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/Info.plist"

# Embed provisioning profiles (required for restricted entitlements)
if [ -n "$APP_PROFILE" ]; then
    cp "$APP_PROFILE" "$BUNDLE/Contents/embedded.provisionprofile"
fi
if [ -n "$APPEX_PROFILE" ]; then
    cp "$APPEX_PROFILE" \
        "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex/Contents/embedded.provisionprofile"
fi

# Build codesign flags for appex
APPEX_SIGN_FLAGS=(-s "$IDENTITY" -f)
if [ -n "$APPEX_ENTITLEMENTS" ]; then
    APPEX_SIGN_FLAGS+=(--entitlements "$APPEX_ENTITLEMENTS")
fi

# Build codesign flags for app
APP_SIGN_FLAGS=(-s "$IDENTITY" -f)
if [ -n "$ENTITLEMENTS" ]; then
    APP_SIGN_FLAGS+=(--entitlements "$ENTITLEMENTS")
fi

# Sign extension first, then app (order matters for Apple).
# The appex contains a symlink to the main binary, so we sign the
# appex bundle (not the symlink itself) — codesign handles this.
codesign "${APPEX_SIGN_FLAGS[@]}" \
    "$BUNDLE/Contents/PlugIns/DistantFileProvider.appex"
codesign "${APP_SIGN_FLAGS[@]}" \
    "$BUNDLE"

echo "Bundle created at $BUNDLE"
echo "Signed with identity: $IDENTITY"
echo "App entitlements: ${ENTITLEMENTS:-none}"
echo "Appex entitlements: ${APPEX_ENTITLEMENTS:-none}"
