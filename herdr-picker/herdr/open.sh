#!/bin/sh
# Plugin actions run without a TTY, so the action only asks herdr to open the
# picker pane; placement and size come from the manifest.
set -eu
exec "${HERDR_BIN_PATH:-herdr}" plugin pane open --plugin warren.herdr-picker --entrypoint picker --focus
