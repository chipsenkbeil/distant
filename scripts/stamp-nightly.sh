#!/usr/bin/env bash
set -euo pipefail
DATE=$(date -u +%Y%m%d)
find . -name 'Cargo.toml' -not -path './target/*' -exec \
  perl -i -pe "s/-dev\"/-nightly.$DATE\"/g" {} +
echo "Stamped nightly version: $DATE"
