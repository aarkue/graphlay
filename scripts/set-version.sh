#!/usr/bin/env bash
# Sets the release version everywhere it is written down and adds its changelog heading.
# Usage: scripts/set-version.sh 0.2.2
set -euo pipefail

version="${1:?usage: scripts/set-version.sh X.Y.Z}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $version" >&2; exit 1; }
cd "$(dirname "$0")/.."

# The first `version = ` in Cargo.toml is [workspace.package]. The Python package reads it too.
perl -0pi -e "s/^version = \"[^\"]*\"/version = \"$version\"/m" Cargo.toml
# graphlay-cli is published, so its dependency on graphlay needs the matching version.
perl -pi -e "s/^(graphlay = \{ path = \"\.\.\", version = \")[^\"]*/\${1}$version/" cli/Cargo.toml
(cd bindings/wasm/js && npm pkg set version="$version")
cargo update --workspace --quiet

if ! grep -q "^## $version " CHANGELOG.md; then
  perl -0pi -e "s/^## /## $version ($(date +%F))\n\n- \n\n## /m" CHANGELOG.md
  echo "Added a CHANGELOG.md heading for $version. Fill in its entries."
fi
git --no-pager diff --stat
