# herdr picker

A fast popup for [herdr](https://herdr.dev) that fuzzy-searches your open
workspaces and previews the live screen of each workspace's active pane.

```
╭ herdr picker ─────────────────────────────────────────────── 1/10 ╮
│❯ revis                                                            │
╰──────────────────────────── enter focus · esc clear/close · ctrl-r ╯
╭ workspaces ───────────────╮╭ warren-revisit-claude-bot ────────────╮
│▌ 9 ○ warren-revisit-claude││~/.herdr/worktrees/hyperdx-ee/…  ⎇ main│
│                           ││○ idle  · 1 tab                        │
│                           ││▸ 1 · opencode-gh                      │
│                           ││    ● OC | PR #3513 …  opencode idle   │
│                           ││── screen w1A:p1 ──────────────────────│
│                           ││  (live pane output)                   │
╰───────────────────────────╯╰───────────────────────────────────────╯
```

- Single Rust binary, ~1 MB. Talks to the herdr socket directly (sub-millisecond
  round trips) instead of spawning the CLI.
- fzf-style matching via [nucleo](https://github.com/helix-editor/nucleo), with
  label matches ranked first. Also matches cwd, repo name, and git branch.
- Preview header shows cwd, git branch and dirty state, agent status, and the
  tab/pane tree, followed by the active pane's screen with colors. The screen
  refreshes while you look at it.
- The current workspace is listed last, so the initial selection is your most
  likely switch target.

## Requirements

- herdr 0.7.5 or newer, macOS or Linux
- Rust (only if no prebuilt binary exists for your platform)

## Install

```sh
herdr plugin install wrn14897/herdr-plugins/herdr-picker
```

The build step downloads the prebuilt binary from the matching GitHub release
(sha256-verified), or builds it with `cargo` when no release asset exists.

Bind a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "warren.herdr-picker.open"
description = "open herdr picker"
```

Then run `herdr server reload-config`. You can also open it without a binding:

```sh
herdr plugin action invoke warren.herdr-picker.open
```

## Keys

| Key | Action |
| --- | --- |
| type | Filter workspaces |
| `↑`/`↓`, `ctrl-p`/`ctrl-n`, `ctrl-k`/`ctrl-j`, `tab`/`shift-tab` | Move selection |
| `pgup`/`pgdn` | Move 10 rows |
| `enter` | Focus workspace and close |
| `esc` | Clear the query, or close when empty |
| `ctrl-u` / `ctrl-w` | Clear query / delete word |
| `ctrl-r` | Refresh workspaces |
| `ctrl-c` | Close |

## Development

```sh
cargo test
HERDR_PICKER_FROM_SOURCE=1 bash herdr/install.sh   # build into bin/
herdr plugin link .
```

`herdr plugin link` does not run build steps, so rerun `herdr/install.sh` after
code changes. `herdr-picker --list` prints the workspace rows as TSV, which is
handy for checking socket access outside the popup.

## Releasing

Bump `version` in both `herdr-plugin.toml` and `Cargo.toml`, then push a tag
`herdr-picker-v<version>`. The release workflow builds macOS and Linux (musl)
binaries and attaches `herdr-picker-<target>.tar.gz` plus `.sha256` files.
