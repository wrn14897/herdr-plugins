# herdr picker

A fast popup for [herdr](https://herdr.dev) that searches everything in your
workspaces: names, paths, branches, tabs, panes, agents, what's on screen, and
your agents' conversation history. The preview shows the live screen, or the
match in context.

```
╭ herdr picker ───────────────────────────────────────────────── 2/10 ╮
│❯ automerge                                                          │
╰──────── enter focus · alt-j/k next/prev hit · ctrl-s hits/screen ───╯
╭ workspaces ─────────────────╮╭ warren-revisit-claude-bot ───────────╮
│▌ 9 ○ warren-revisit-claude… ││~/.herdr/worktrees/…  ⎇ warren/oc-gh  │
│      ↳ chat · opencode  +8 …││○ idle  · 1 tab                       │
│ 10 ● herdr-plugins (current)││▸ 1 · opencode-gh  ○                  │
│      ↳ chat · opencode  +1 …││── chat · opencode 1/9 ───────────────│
│                             ││**1. Nothing ever summons it for      │
│                             ││conflicts.** `pull-upstream.yml` and  │
│                             ││`automerge-conflict-watch.yml` post … │
╰─────────────────────────────╯╰──────────────────────────────────────╯
```

## Requirements

- herdr 0.7.5 or newer, on macOS or Linux.
- Prebuilt binaries for Apple Silicon and Intel macOS, and x86_64 and arm64
  Linux. Other platforms build from source, which needs Rust 1.85 or newer.
- For conversation search: herdr's integration for each agent you use
  (`herdr integration install claude`, `codex`, `opencode`). It tells herdr
  which session a pane is running. Without it, the picker uses the newest
  session in the pane's directory.

## Install

```sh
herdr plugin install wrn14897/herdr-plugins/herdr-picker
```

The build step downloads the binary from the matching GitHub release
(sha256-verified), or builds it with cargo when no release asset exists.

Bind a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "warren.herdr-picker.open"
description = "open herdr picker"
```

Then run `herdr server reload-config` and check it appears in `prefix+?`. Use a
`prefix+` key: bare chords like `ctrl+f` are passed to the focused program
(shells and editors use them), so herdr may never see them. You can also open
it without a binding:

```sh
herdr plugin action invoke warren.herdr-picker.open
```

## What it searches

| Kind | Matching | Ranked |
| --- | --- | --- |
| Workspace label | fuzzy | 1st |
| Tab, pane, and agent names; agent terminal titles | fuzzy | 2nd |
| Path (last two segments), repo, git branch | fuzzy | 3rd |
| Agent conversation history (Claude Code, Codex, OpenCode): your prompts and the agent's replies | text | 4th |
| Screen text: last 500 rows of every agent pane and of each workspace's active pane | text | 5th |

Each workspace appears once, ranked by its best match. When that match isn't
its label, a second line shows what matched (`tab`, `branch`, `chat · claude`,
`screen · zsh`, ...) and how many more content hits it has.

Screen text is only indexed for agent panes and the pane you'd land on. Text
in other panes (a server log in a background tab, say) is not searched.
Content hits in the workspace you're currently in rank after other
workspaces, since your own session usually echoes what you're typing.

### Query syntax

**Names** use fzf-style fuzzy matching ([nucleo](https://github.com/helix-editor/nucleo)),
but stricter:

- Each word must match inside a single name, not scattered across several.
- Only well-formed matches count: substrings, prefixes, and initials
  (`wrcb` → `warren-revisit-claude-bot`). A typo or random string shows
  nothing rather than noise.
- Different words can match different names: `api main` finds workspace `api`
  on branch `main`.
- `!word` excludes workspaces whose names contain `word`; text matches don't
  bring them back.

**Text** (conversations and screens) is matched literally, starting at 3
characters:

- Every word must appear in the same message or screen line, in any order.
- `'exact phrase` matches the phrase as written.
- `/regex/` is a regular expression.
- `!word` skips messages and lines that contain `word`.

Both are smart-case: lowercase ignores case, and any uppercase letter makes the
query case-sensitive.

## Preview and keys

The preview shows the workspace's path, git branch (`✱` when dirty), agent
status, and tab/pane tree. Below that it shows either the live screen of the
active pane, or, when the row matched on text, the match in context: the whole
message or the surrounding screen lines, with matches highlighted.

| Key | Action |
| --- | --- |
| type | Search |
| `↑`/`↓`, `ctrl-p`/`ctrl-n`, `ctrl-k`/`ctrl-j`, `tab`/`shift-tab` | Move selection |
| `pgup`/`pgdn` | Move 10 rows |
| `alt-j`/`alt-k` (or `alt-↓`/`alt-↑`) | Next/previous text match in the selected workspace |
| `ctrl-s` | Switch the preview between the match and the live screen |
| `enter` | Focus the workspace and close. While a match from an agent pane is shown, focus that agent instead |
| `esc` | Clear the query, or close when empty |
| `ctrl-u` / `ctrl-w` | Clear query / delete word |
| `ctrl-r` | Refresh workspaces and re-index everything |
| `ctrl-c` | Close |

## Configuration

Optional. Create `config.toml` in the plugin's config directory
(`herdr plugin config-dir warren.herdr-picker`):

```toml
[content]
enabled = true                           # false: search names and paths only
screen_lines = 500                       # screen rows indexed per pane
transcript_messages = 200                # newest messages kept per session
transcript_bytes = 1048576               # max text kept per session (1 MB)
agents = ["claude", "codex", "opencode"] # whose conversations to search
```

All keys are optional. Unknown keys are rejected, so a typo shows an error
instead of being ignored.

Install-time environment variables for `herdr/install.sh`:

| Variable | Effect |
| --- | --- |
| `HERDR_PICKER_FROM_SOURCE=1` | Skip the download and build with cargo |
| `HERDR_PICKER_REPO=owner/repo` | Download releases from a fork |

## How it stays fast

- One Rust binary (~4 MB, SQLite included) that talks to the herdr socket directly.
- Workspaces appear instantly; screens, conversations, and git info are
  indexed in the background on 4 worker threads, with progress in the header
  (`indexing 12/30`). Results update as data arrives.
- The selected workspace re-indexes every 2 seconds, so a running agent's
  newest messages are searchable without refreshing. Unchanged conversations
  are not re-read.
- Tested with 5,000 workspaces: about 10 ms to build the list and 2–3 ms per
  keystroke.

## Privacy

Conversation history is read from your own machine (`~/.claude/projects`,
`~/.codex/sessions`, and OpenCode's local database, which is opened read-only)
and is kept only in the picker's memory while it is open. Nothing is written or
sent anywhere. Set `enabled = false` or trim `agents` to opt out.

## Troubleshooting

Start with `herdr plugin log list --plugin warren.herdr-picker --limit 3`: each
time the open action runs it leaves an entry, with the error in `stderr`.

| Symptom | Cause | Fix |
| --- | --- | --- |
| Key does nothing and no new log entry | The binding isn't loaded, or the key never reaches herdr | `herdr server reload-config` (its `diagnostics` must be empty), check `prefix+?`, use a `prefix+` key |
| Toast: "Not built yet" | Plugin was linked from a checkout, which skips the build step | `bash herdr/install.sh` in the plugin directory |
| Toast: "Close the open popup or menu first." | Another popup, Settings, or copy mode is open | Close it and press the key again |
| `install.sh`: `could not download …` | Offline, blocked, or no release for this version | It falls back to cargo; or retry with network |
| `install.sh`: `building needs Rust 1.85 or newer` | Old Rust toolchain | `rustup update stable` |
| `install.sh`: `cargo is not installed` | No release asset and no Rust | Install Rust from https://rustup.rs |
| No conversation results for an agent | Unsupported agent, agent not in `agents`, or the session lives elsewhere | Install herdr's integration for that agent; check `config.toml` |
| Text visible in a pane isn't found | The pane isn't an agent pane or its workspace's active pane | Expected: see "What it searches" |
| A nonsense query finds a workspace | The words literally appear in that workspace's conversation or screen (the detail line shows where) | Expected: text search is literal |

To see exactly what matches without the UI:

```sh
"$(herdr plugin list --plugin warren.herdr-picker --json | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['plugins'][0]['plugin_root'])")/bin/herdr-picker" --search 'your query'
```

It prints each matching workspace with its match kind, score, and detail.

## Development

```sh
herdr plugin link .
bash herdr/install.sh                              # required once: link skips build steps
HERDR_PICKER_FROM_SOURCE=1 bash herdr/install.sh   # rebuild after code changes
cargo test
```

`herdr plugin link` registers the plugin but does not run its build step, so
`bin/herdr-picker` must come from `herdr/install.sh`. After editing
`herdr-plugin.toml`, run `herdr plugin unlink warren.herdr-picker` and link
again.

- [docs/TESTING.md](docs/TESTING.md): testing locally, from unit tests to
  driving the TUI (`scripts/tui-test.sh`) and verifying a release.
- [AGENTS.md](AGENTS.md): architecture and conventions for contributors and
  coding agents.
- [CHANGELOG.md](CHANGELOG.md): release history.

## Releasing

Bump `version` in both `herdr-plugin.toml` and `Cargo.toml`, run `cargo build`
so `Cargo.lock` updates, add a `CHANGELOG.md` entry, then push a tag
`herdr-picker-v<version>`. The release workflow builds the four binaries and
attaches `herdr-picker-<target>.tar.gz` plus `.sha256` files, with the
changelog entry as release notes. Full checklist: [../AGENTS.md](../AGENTS.md).
