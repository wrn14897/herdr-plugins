# herdr picker

A fast popup for [herdr](https://herdr.dev) that searches everything in your
workspaces: names, paths, branches, tabs, panes, agents, what's on screen, and
your agents' conversation history. The preview shows the live screen, or the
match in context.

```
╭ herdr picker ───────────────────────────────────────────────── 2/10 ╮
│❯ automerge                                                          │
╰─────────────── enter focus · alt-j/k next/prev hit · ctrl-s · esc ──╯
╭ workspaces ─────────────────╮╭ warren-revisit-claude-bot ───────────╮
│▌ 9 ○ warren-revisit-claude… ││~/.herdr/worktrees/…  ⎇ main          │
│      ↳ chat · opencode  +8 …││○ idle  · 1 tab                       │
│ 10 ● herdr-plugins (current)││▸ 1 · opencode-gh                     │
│      ↳ screen · opencode  …  ││── chat · opencode 1/9 ───────────────│
│                             ││**1. Nothing ever summons it for      │
│                             ││conflicts.** `pull-upstream.yml` and  │
│                             ││`automerge-conflict-watch.yml` post … │
╰─────────────────────────────╯╰──────────────────────────────────────╯
```

## What it searches

| Kind | Matching | Ranked |
| --- | --- | --- |
| Workspace label | fuzzy | 1st |
| Tab, pane, agent names; agent terminal titles | fuzzy | 2nd |
| Path, repo, git branch | fuzzy | 3rd |
| Agent conversation history (Claude Code, Codex, OpenCode) | text | 4th |
| Pane scrollback (last 500 rows of agent panes and the active pane) | text | 5th |

Fuzzy matching is fzf-style ([nucleo](https://github.com/helix-editor/nucleo)).
Content matching starts at 3 characters and finds lines (or messages) that
contain every word, in any order. Lowercase queries ignore case; any uppercase
letter makes the query case-sensitive. `'exact phrase` matches literally and
`/regex/` is a regular expression.

Each workspace appears once. When it matched on something other than its
label, a second line shows what matched, where (`chat · claude`,
`screen · zsh`, `tab`, `branch`, ...), and how many more content hits there are.

## Install

```sh
herdr plugin install wrn14897/herdr-plugins/herdr-picker
```

The build step downloads the prebuilt binary from the matching GitHub release
(sha256-verified), or builds it with `cargo` when no release asset exists.
Requires herdr 0.7.5 or newer on macOS or Linux.

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

For conversation history, install herdr's integration for each agent you use
(`herdr integration install claude`, `codex`, `opencode`). It reports which
session a pane is running. Without it, the picker uses the newest session in
the pane's directory.

## Keys

| Key | Action |
| --- | --- |
| type | Search |
| `↑`/`↓`, `ctrl-p`/`ctrl-n`, `ctrl-k`/`ctrl-j`, `tab`/`shift-tab` | Move selection |
| `pgup`/`pgdn` | Move 10 rows |
| `alt-j`/`alt-k` (or `alt-↓`/`alt-↑`) | Next/previous content hit in the selected workspace |
| `ctrl-s` | Switch the preview between the hit and the live screen |
| `enter` | Focus the workspace and close. On a hit in an agent pane, focus that agent |
| `esc` | Clear the query, or close when empty |
| `ctrl-u` / `ctrl-w` | Clear query / delete word |
| `ctrl-r` | Refresh workspaces and re-index everything |
| `ctrl-c` | Close |

## How it stays fast

- One Rust binary (~4 MB, SQLite included) that talks to the herdr socket directly.
- Workspaces appear instantly; screens, transcripts, and git info are indexed in
  the background on 4 worker threads, with progress in the header
  (`indexing 12/30`). Search results update as data arrives.
- The selected workspace re-indexes every 2 seconds, so a running agent's newest
  messages are searchable without refreshing. Unchanged transcripts are not
  re-read.
- Tested with 5,000 workspaces: about 10 ms to build the list and 2–3 ms per
  keystroke.

## Configuration

Optional. Create `config.toml` in the plugin's config directory
(`herdr plugin config-dir warren.herdr-picker`):

```toml
[content]
enabled = true                           # false: search names and paths only
screen_lines = 500                       # scrollback rows indexed per pane
transcript_messages = 200                # newest messages kept per session
agents = ["claude", "codex", "opencode"] # whose history to search
```

Unknown keys are rejected, so a typo shows an error instead of being ignored.

## Privacy

Conversation history is read from your own machine (`~/.claude/projects`,
`~/.codex/sessions`, and OpenCode's local database, which is opened read-only)
and is kept only in the picker's memory while it is open. Nothing is written or
sent anywhere. Set `enabled = false` or trim `agents` to opt out.

## Development

```sh
cargo test
HERDR_PICKER_FROM_SOURCE=1 bash herdr/install.sh   # build into bin/
herdr plugin link .
```

`herdr plugin link` does not run build steps, so rerun `herdr/install.sh` after
code changes. Two commands help outside the popup:

- `herdr-picker --list` prints the workspace rows as TSV.
- `herdr-picker --search QUERY` indexes everything and prints ranked results.

## Releasing

Bump `version` in both `herdr-plugin.toml` and `Cargo.toml`, then push a tag
`herdr-picker-v<version>`. The release workflow builds macOS and Linux (musl)
binaries and attaches `herdr-picker-<target>.tar.gz` plus `.sha256` files.
