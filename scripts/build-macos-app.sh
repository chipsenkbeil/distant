#!/bin/bash
# Build, sign, and install Distant.app with the FileProvider extension.
#
# Usage:
#   scripts/make-app.sh          # full pipeline: build + bundle + install
#   scripts/make-app.sh --skip-build   # skip cargo build (reuse existing binary)
#
# Environment overrides (all have sensible defaults for dev):
#   CODESIGN_IDENTITY   — signing identity (default: auto-detect Apple Development)
#   APP_PROFILE         — app provisioning profile path
#   APPEX_PROFILE       — appex provisioning profile path
#   CARGO_FEATURES      — features to build with
#   INSTALL_DIR         — where to install (default: /Applications)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# ── Configuration ────────────────────────────────────────────────────
CARGO_FEATURES="${CARGO_FEATURES:-docker,host,ssh,pty,mount-nfs,mount-macos-file-provider}"
INSTALL_DIR="${INSTALL_DIR:-/Applications}"
APP_NAME="Distant.app"
APPEX_REL="Contents/PlugIns/DistantFileProvider.appex"
APPEX_BUNDLE_ID="dev.distant.file-provider"

# Auto-detect signing identity if not set
if [ -z "${CODESIGN_IDENTITY:-}" ]; then
    CODESIGN_IDENTITY=$(security find-identity -v -p codesigning \
        | grep "Apple Development" \
        | head -1 \
        | sed 's/.*"\(.*\)"/\1/')
    if [ -z "$CODESIGN_IDENTITY" ]; then
        echo "error: no Apple Development signing identity found" >&2
        echo "  Set CODESIGN_IDENTITY or install a development certificate" >&2
        exit 1
    fi
fi

# Default provisioning profiles (checked into the repo)
APP_PROFILE="${APP_PROFILE:-$PROJECT_DIR/resources/macos/profiles/Distant_Dev.provisionprofile}"
APPEX_PROFILE="${APPEX_PROFILE:-$PROJECT_DIR/resources/macos/profiles/Distant_FileProvider_Dev.provisionprofile}"

# Validate profiles exist
for profile in "$APP_PROFILE" "$APPEX_PROFILE"; do
    if [ ! -f "$profile" ]; then
        echo "error: provisioning profile not found: $profile" >&2
        exit 1
    fi
done

# ── Parse arguments ──────────────────────────────────────────────────
SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
        *) echo "unknown argument: $arg" >&2; exit 1 ;;
    esac
done

# ── Profile (debug or release) ──────────────────────────────────────
PROFILE="${CARGO_PROFILE:-release}"
BINARY="target/$PROFILE/distant"

# ── Step 1: Build ────────────────────────────────────────────────────
if [ "$SKIP_BUILD" = false ]; then
    BUILD_FLAGS="--features $CARGO_FEATURES"
    if [ "$PROFILE" = "release" ]; then
        BUILD_FLAGS="--release $BUILD_FLAGS"
    fi
    echo "==> Building ($PROFILE, features: $CARGO_FEATURES)"
    cargo build $BUILD_FLAGS
else
    echo "==> Skipping build (--skip-build, profile: $PROFILE)"
    if [ ! -f "$BINARY" ]; then
        echo "error: $BINARY not found — run without --skip-build first" >&2
        exit 1
    fi
fi

# ── Step 2: Bundle + sign ────────────────────────────────────────────
echo "==> Bundling and signing"
CODESIGN_IDENTITY="$CODESIGN_IDENTITY" \
    APP_PROFILE="$APP_PROFILE" \
    APPEX_PROFILE="$APPEX_PROFILE" \
    bash scripts/build-macos-bundle.sh "$BINARY"

# ── Step 3: Install to /Applications ─────────────────────────────────
echo "==> Installing to $INSTALL_DIR/$APP_NAME"
rm -rf "$INSTALL_DIR/$APP_NAME"
cp -R "target/$APP_NAME" "$INSTALL_DIR/"

# ── Step 4: Restart fileproviderd + register plugin ──────────────────
echo "==> Restarting fileproviderd and registering plugin"
killall fileproviderd 2>/dev/null || true
sleep 2
pluginkit -a "$INSTALL_DIR/$APP_NAME/$APPEX_REL"
pluginkit -e use -i "$APPEX_BUNDLE_ID"

# ── Done ─────────────────────────────────────────────────────────────
echo ""
echo "Done! Distant.app installed and FileProvider extension registered."
echo "  Identity: $CODESIGN_IDENTITY"
echo "  Install:  $INSTALL_DIR/$APP_NAME"
