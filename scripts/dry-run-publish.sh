#!/usr/bin/env bash
# Runs cargo publish --dry-run for all crates in dependency order.
# Validates the full publish pipeline without uploading to crates.io.
#
# Usage: scripts/dry-run-publish.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Guard: ensure version is publishable (no prerelease suffix)
"$SCRIPT_DIR/check-publish-version.sh"

echo ""
echo "Running dry-run publish in dependency order..."
echo ""

cargo publish --all-features --dry-run -p distant-core
echo "  distant-core ✓"

cargo publish --all-features --dry-run -p distant-docker
echo "  distant-docker ✓"

cargo publish --all-features --dry-run -p distant-host
echo "  distant-host ✓"

cargo publish --all-features --dry-run -p distant-ssh
echo "  distant-ssh ✓"

cargo publish --all-features --dry-run
echo "  distant ✓"

echo ""
echo "Dry run succeeded. All crates are publishable."
