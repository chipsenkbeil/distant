#!/usr/bin/env bash
# Pre-publish guard: aborts if the workspace version contains a prerelease suffix.
#
# Usage: scripts/check-publish-version.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Extract version from [workspace.package] in root Cargo.toml
VERSION=$(grep -A1 '\[workspace\.package\]' "$REPO_ROOT/Cargo.toml" | grep 'version' | sed 's/.*"\(.*\)".*/\1/')

if [[ -z "$VERSION" ]]; then
    echo "Error: Could not read workspace version from Cargo.toml" >&2
    exit 1
fi

echo "Workspace version: $VERSION"

# Check for prerelease suffix (anything after major.minor.patch)
if echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+-.+$'; then
    echo "Error: Cannot publish a prerelease version ($VERSION)" >&2
    echo "Run 'scripts/set-version.sh X.Y.Z' to set a release version first." >&2
    exit 1
fi

echo "Version $VERSION is publishable."
