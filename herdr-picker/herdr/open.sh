#!/bin/sh
# Plugin actions run without a TTY, so the action only asks herdr to open the
# picker pane; placement and size come from the manifest.
#
# Keybindings have nowhere to print errors, so failures become a toast.
set -u
herdr="${HERDR_BIN_PATH:-herdr}"
root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"

notify() {
  "$herdr" notification show "herdr picker" --body "$1" >/dev/null 2>&1 || true
  echo "herdr-picker: $1" >&2
  exit 1
}

[ -x "$root/bin/herdr-picker" ] ||
  notify "Not built yet. Run: bash $root/herdr/install.sh"

if ! out="$("$herdr" plugin pane open --plugin warren.herdr-picker --entrypoint picker --focus 2>&1)"; then
  case "$out" in
    *ui_busy*) notify "Close the open popup or menu first." ;;
    *) notify "Could not open: $(printf '%s' "$out" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p' | head -c 200)" ;;
  esac
fi
