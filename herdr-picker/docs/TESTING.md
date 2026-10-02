# Testing herdr-picker locally

A ladder from cheapest to most realistic. Run the lower levels on every change;
climb as far as the change needs. All commands run from `herdr-picker/`.

On the maintainer's Mac, cargo is not on PATH by default:

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
```

| Level | What | Needs herdr | Touches the user's screen |
| --- | --- | --- | --- |
| 0 | Unit, regression, and scale tests | no | no |
| 1 | Headless `--list` / `--search` against the live session | yes | no |
| 2 | Drive the TUI in a background tab (`scripts/tui-test.sh`) | yes | no |
| 3 | The real popup via the plugin action | yes | **yes** |
| 4 | `install.sh` and `open.sh` failure paths | no | no |
| 5 | Release verification from GitHub | yes | no |

## Level 0: unit, regression, scale

```sh
cargo fmt --check
cargo clippy --all-targets        # pedantic lints; must print no warnings
cargo test                        # all unit tests (scale test uses a 20x debug budget)
cargo test --release five_thousand -- --nocapture
```

The last command prints the real numbers, for example
`5k workspaces: build_rows 10ms, search 2.3ms/query`; the budget is 50 ms each.

Fixtures worth knowing:

- `search::tests::real_rows()` mirrors real workspace names, worktree paths,
  and titles. Add regressions here. `nonsense_matches_nothing` and
  `real_queries_still_find_their_workspace` guard the fuzzy matching quality.
- `model::tests::FIXTURE` is a small `session.snapshot`.
- `transcript::tests` build temporary JSONL files and an OpenCode-shaped SQLite
  database; they never read your real history.
- `bench_tests::synthetic_snapshot(n)` generates large sessions.

## Level 1: headless against the live session

Uses the running herdr server through the socket; no UI.

```sh
cargo build --release
target/release/herdr-picker --list
target/release/herdr-picker --search 'some query'
```

`--list` prints `workspace_id, number, label, status, cwd, active_pane_id`, with
the focused workspace last. `--search` indexes everything synchronously
(screens and transcripts) and prints
`workspace_id, kind, score, label, content hits, detail`, best first.

Spot checks:

| Query | Expect |
| --- | --- |
| a fresh random string, `"$(openssl rand -hex 6)"` | no rows |
| a substring or initials of a workspace label | that workspace first, kind `label` |
| a word from a branch name | kind `branch` |
| a phrase from an agent conversation | kind `chat`, with the matching line |
| a phrase visible in an agent pane | kind `screen` |

Use a freshly generated string for the "no rows" check. A fixed nonsense word
stops being nonsense once it appears in an agent conversation (including the one
you're testing from), and content search will then correctly find it.

The pane you run `--search` from is excluded from screen indexing (it would
contain the query), so typing a word and immediately searching for it from the
same shell won't match itself. Its agent transcripts are still searched.

## Level 2: the TUI in a background tab

Popups cannot be read back, so test the TUI by running the binary in a tab that
is never focused:

```sh
HERDR_PICKER_FROM_SOURCE=1 bash herdr/install.sh   # bin/herdr-picker = your build
scripts/tui-test.sh QUERY [KEY...]
```

It opens a background tab, starts `bin/herdr-picker`, waits for indexing, types
`QUERY`, sends each key, prints the rendered screen after the query and after
each key, then closes the tab (also on failure or Ctrl-C). Keys use herdr key
syntax: `down`, `up`, `alt+j`, `alt+k`, `ctrl+s`, `ctrl+u`, `esc`, `backspace`.

```sh
scripts/tui-test.sh automerge alt+j ctrl+s
```

prints three screens; check that the preview rule changes from
`chat · opencode 1/9` to `2/9`, then to `screen <pane> · 9 hits (ctrl-s)`.
Assert on text, not on ANSI styling or exact widths (the tab's size follows
the user's terminal).

Tunables: `PICKER_BIN` to test another binary (for example
`target/release/herdr-picker`), `INDEX_WAIT` (default 2 s) for big sessions,
`STEP_WAIT` (default 0.6 s).

Avoid `enter` here: it focuses a workspace and switches the user's view.

Doing it by hand, for reference:

```sh
out=$(herdr tab create --label picker-test --no-focus)
T=$(echo "$out" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['tab']['tab_id'])")
P=$(echo "$out" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['root_pane']['pane_id'])")
herdr pane run "$P" "$PWD/bin/herdr-picker"
herdr pane send-text "$P" "query"; herdr pane send-keys "$P" alt+j
herdr pane read "$P" --source visible
herdr tab close "$T"
```

Use a tab, not a split of the user's pane: splits in the focused tab get closed
when the user switches or tidies up, and they steal space from their work.

## Level 3: the real popup

Only when the change is about the launch path (manifest, `herdr/open.sh`,
popup sizing). This opens a popup over whatever the user is doing.

```sh
herdr plugin action invoke warren.herdr-picker.open
herdr plugin log list --plugin warren.herdr-picker --limit 3
```

The log entry for the action should have `status: succeeded`. On failure,
`stderr` has the reason, which `open.sh` also shows as a toast:

| stderr | Meaning |
| --- | --- |
| `Not built yet. Run: bash …/install.sh` | `bin/herdr-picker` missing (linked checkout, never built) |
| `ui_busy` / `Close the open popup or menu first.` | another popup, Settings, or copy mode is open |
| `Could not open: …` | herdr refused; the message is herdr's |

Close a popup without touching the keyboard (there is no CLI command for this):

```sh
python3 -c "import socket,os; s=socket.socket(socket.AF_UNIX); s.connect(os.path.expanduser('~/.config/herdr/herdr.sock')); s.sendall(b'{\"id\":\"c\",\"method\":\"popup.close\",\"params\":{}}\n'); print(s.recv(4096).decode())"
```

Keybinding problems: the key must be a `[[keys.command]]` in the user's
config, loaded with `herdr server reload-config` (non-empty `diagnostics` means
the config was rejected), and should be a `prefix+` key. If pressing the key
adds no entry to `plugin log list`, the key never reached herdr.

## Level 4: install and launch scripts

Test `herdr/install.sh` in a scratch copy so `bin/` and `target/` stay intact:

```sh
D=$(mktemp -d); cp -R herdr herdr-plugin.toml Cargo.toml Cargo.lock src "$D"/; cd "$D"
```

| Scenario | Command | Expect |
| --- | --- | --- |
| release download | `bash herdr/install.sh` | `downloading …`, `installed …/bin/herdr-picker` |
| no release for this version | set `version = "9.9.9"` in `herdr-plugin.toml`, run again | `could not download …`, `falling back to building from source`, then a cargo build |
| cargo too old | put a fake `cargo` that prints `cargo 1.82.0` first on PATH, `HOME=/nonexistent`, and remove the Homebrew paths from `find_cargo` in the copy | `found cargo 1.82, but building needs Rust 1.85` and `rustup update stable`, exit 1 |
| no cargo | `env PATH=/usr/bin:/bin HOME=/nonexistent` (same edit) | `cargo is not installed; install Rust 1.85+`, exit 1 |
| stock macOS bash | run any of the above with `/bin/bash` (3.2) | same results |

Test `herdr/open.sh` with a stub `herdr` so no popup or toast appears:

```sh
D=$(mktemp -d); mkdir -p "$D/root/bin"
cat > "$D/herdr" <<'EOF'
#!/bin/sh
case "$1" in
  notification) echo "TOAST: $*" ;;
  plugin) [ "$FAKE" = busy ] && { echo '{"error":{"code":"ui_busy","message":"busy"}}'; exit 1; }; echo ok ;;
esac
EOF
chmod +x "$D/herdr"
HERDR_BIN_PATH="$D/herdr" HERDR_PLUGIN_ROOT="$D/root" sh herdr/open.sh   # missing binary -> toast, exit 1
touch "$D/root/bin/herdr-picker"; chmod +x "$D/root/bin/herdr-picker"
FAKE=busy HERDR_BIN_PATH="$D/herdr" HERDR_PLUGIN_ROOT="$D/root" sh herdr/open.sh   # ui_busy -> toast
HERDR_BIN_PATH="$D/herdr" HERDR_PLUGIN_ROOT="$D/root" sh herdr/open.sh             # exit 0
```

## Level 5: release verification

After pushing a `herdr-picker-v<version>` tag (see the release checklist in
[../../AGENTS.md](../../AGENTS.md)):

```sh
gh run watch "$(gh run list --limit 1 --json databaseId -q '.[0].databaseId')" --exit-status
gh release view herdr-picker-v<version> --json assets -q '.assets[].name'   # 4 .tar.gz + 4 .sha256
gh release view herdr-picker-v<version> --json body -q .body              # the CHANGELOG section

herdr plugin uninstall warren.herdr-picker
herdr plugin install wrn14897/herdr-plugins/herdr-picker --yes
ROOT=$(herdr plugin list --plugin warren.herdr-picker --json | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['plugins'][0]['plugin_root'])")
"$ROOT/bin/herdr-picker" --search 'some query'        # Level 1 against the published binary
PICKER_BIN="$ROOT/bin/herdr-picker" scripts/tui-test.sh 'some query'
```

The installed root should contain no `target/` directory; that confirms the
binary came from the release, not a cargo fallback.

Switch back to the local checkout for development:

```sh
herdr plugin uninstall warren.herdr-picker
herdr plugin link "$PWD"
bash herdr/install.sh
```
