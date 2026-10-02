#!/usr/bin/env bash
# Print the CHANGELOG.md section for VERSION (without its heading), for use as
# release notes. Exits 1 if the version has no entry.
#
#   scripts/changelog-section.sh 0.2.1
set -euo pipefail

version="${1:?usage: changelog-section.sh VERSION}"
changelog="$(cd "$(dirname "$0")/.." && pwd)/CHANGELOG.md"

awk -v v="$version" '
  /^## \[/ { if (found) exit; if (index($0, "## [" v "]") == 1) { found = 1; next } }
  /^\[[^]]+\]: / { if (found) exit }
  found { print }
  END { if (!found) exit 1 }
' "$changelog" | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}' || {
  echo "changelog-section: no entry for $version in $changelog" >&2
  exit 1
}
