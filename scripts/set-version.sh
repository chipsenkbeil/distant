#!/usr/bin/env bash
# Sets the workspace version across all Cargo.toml files.
#
# Usage: scripts/set-version.sh <version>
#
# Examples:
#   scripts/set-version.sh 0.21.0        # release version
#   scripts/set-version.sh 0.22.0-dev    # post-release dev version
#   scripts/set-version.sh 0.21.0-rc.1   # pre-release
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <version>" >&2
    echo "Example: $0 0.21.0" >&2
    exit 1
fi

NEW_VERSION="$1"

# Validate version format (semver with optional prerelease)
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "Error: '$NEW_VERSION' is not a valid semver version" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_TOML="$REPO_ROOT/Cargo.toml"

echo "Setting version to $NEW_VERSION ..."

# 1. Update [workspace.package] version in root Cargo.toml
# Matches the version line inside the [workspace.package] section
perl -i -0777 -pe "s/(\\[workspace\\.package\\]\nversion = \")[^\"]+/\${1}${NEW_VERSION}/s" "$ROOT_TOML"

# 2. Update [workspace.dependencies] version pins for internal crates
# Matches version = "=X.Y.Z..." inside distant-core/docker/host/ssh dependency lines
perl -i -pe "s/(distant-(?:core|docker|host|ssh) = \\{ version = \"=)[^\"]+/\${1}${NEW_VERSION}/g" "$ROOT_TOML"

echo "Updated $ROOT_TOML"

# 3. Verify
echo ""
echo "Verifying workspace resolves..."
cd "$REPO_ROOT"
cargo check --workspace 2>&1 | tail -3

echo ""
echo "Version set to $NEW_VERSION"
