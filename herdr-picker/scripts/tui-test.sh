#!/usr/bin/env bash
# Drive the picker TUI in a background herdr tab and print what it renders.
#
#   scripts/tui-test.sh QUERY [KEY...]
#
# Opens a new tab without focusing it, runs bin/herdr-picker there, types
# QUERY, sends each KEY (herdr key syntax: alt+j, ctrl+s, down, ...), prints the
# visible screen after the query and after every key, then closes the tab.
# Popups can't be read back, which is why this runs the binary in a tab.
#
# Env: PICKER_BIN (default: bin/herdr-picker), INDEX_WAIT seconds to let the
# background index fill in before typing (default 2), STEP_WAIT (default 0.6).
set -euo pipefail

if [ $# -lt 1 ]; then
  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HERDR="${HERDR_BIN_PATH:-herdr}"
BIN="${PICKER_BIN:-$ROOT/bin/herdr-picker}"
INDEX_WAIT="${INDEX_WAIT:-2}"
STEP_WAIT="${STEP_WAIT:-0.6}"
QUERY="$1"
shift

[ -x "$BIN" ] || { echo "tui-test: $BIN not found; run herdr/install.sh first" >&2; exit 1; }
command -v python3 >/dev/null || { echo "tui-test: python3 is required to parse herdr JSON" >&2; exit 1; }

json() { python3 -c "import sys,json; print(json.load(sys.stdin)$1)"; }

created="$("$HERDR" tab create --label picker-test --no-focus)"
TAB="$(json "['result']['tab']['tab_id']" <<<"$created")"
PANE="$(json "['result']['root_pane']['pane_id']" <<<"$created")"
cleanup() {
  "$HERDR" pane send-keys "$PANE" ctrl+c >/dev/null 2>&1 || true
  "$HERDR" tab close "$TAB" >/dev/null 2>&1 || true
}
trap cleanup EXIT

show() {
  echo "=== $1"
  "$HERDR" pane read "$PANE" --source visible
}

sleep 0.5 # let the shell start
"$HERDR" pane run "$PANE" "$BIN" >/dev/null
sleep "$INDEX_WAIT"
"$HERDR" pane send-text "$PANE" "$QUERY" >/dev/null
sleep "$STEP_WAIT"
show "query: $QUERY"

for key in "$@"; do
  "$HERDR" pane send-keys "$PANE" "$key" >/dev/null
  sleep "$STEP_WAIT"
  show "key: $key"
done
