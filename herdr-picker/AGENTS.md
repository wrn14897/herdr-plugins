# herdr-picker

A popup for herdr that searches workspaces by name, path, branch, tab, pane,
agent, screen text, and agent conversation history, with a live preview. One
Rust binary, no runtime dependencies. User docs: [README.md](README.md).
Testing guide: [docs/TESTING.md](docs/TESTING.md).

Read the repo-level [../AGENTS.md](../AGENTS.md) first for conventions and the
release checklist.

## Constraints

- **Fast.** First paint must not wait on anything but one `session.snapshot`.
  Per keystroke work is bounded (see [Performance budget](#performance-budget)).
- **Zero-build install.** `herdr/install.sh` downloads a release binary; cargo is
  only a fallback. Don't add system dependencies (SQLite is bundled).
- **Read-only toward user data.** Transcripts and the OpenCode database are only
  read (SQLite opened read-only). The picker writes nothing but its own config.
- **One row per workspace.** Matches inside a workspace never become extra rows.

## Architecture

```
                 ┌──────────────── herdr socket (NDJSON, one connection per request)
                 │
 session.snapshot│                         first paint
 ────────────────┴──▶ model::build_rows ──▶ Vec<Row> ───────────────┐
                                              │                     │
                       index::tasks_for(row) ─┤ git::queue_lookup   │
                                              ▼                     │
                       pool::Pool (4 workers, urgent jobs first)    │
                         ├─ git branch/dirty ─▶ Row.branch           │
                         ├─ Job::Screen ── pane.read recent_unwrapped│
                         └─ Job::Transcript ─ claude|codex|opencode  │
                                   │ index::Update (segment)         │
                                   ▼                                 │
                         content::Store (one per workspace)         │
                                                                     │
 query ─▶ search::Searcher (nucleo, per field) ──┐                   │
       ─▶ content::Query  (regex, per store) ────┴▶ merge_content ─▶ Vec<Hit> ─▶ ui::draw
                                                                     │
 selected row ─▶ preview worker ─ pane.read visible (ANSI) ─▶ live screen / hit view
```

Everything except snapshot and focus calls happens off the UI thread. Results
arrive on channels that `App::drain_background` empties once per frame (40 ms
tick), then re-runs the search if data changed.

## Module map

| File | Responsibility | Invariant |
| --- | --- | --- |
| `main.rs` | CLI (`--list`, `--search`, picker), terminal setup | Errors before the TUI starts print plainly; inside a popup they sleep 4 s so they're readable |
| `herdr.rs` | Socket client: `snapshot`, `read_visible`, `read_recent`, `focus_workspace`, `focus_agent` | Herdr closes the connection after each response, so every request reconnects (~0.2 ms) |
| `model.rs` | Snapshot types → `Vec<Row>`; searchable `fields`; precomputed `haystack` | `build_rows` is linear (id maps, no nested scans); focused workspace sorts last |
| `search.rs` | Fuzzy name matching (nucleo), ranking, `merge_content` | Each word matches inside one field; never across fields |
| `content.rs` | `Store` (documents per workspace), `Query` (literal/regex), snippets | One contiguous string per store, so a query is one regex scan |
| `index.rs` | Background jobs that produce store segments | Picker's own pane is never screen-indexed |
| `transcript.rs` | Locate and read Claude / Codex / OpenCode sessions | Keep user prompts and assistant text only; bounded reads |
| `pool.rs` | Bounded worker pool | `submit(urgent = true)` jumps the queue |
| `git.rs` | Branch + dirty lookups via `git` subprocess | Runs on the pool, never on the UI thread |
| `preview.rs` | Live ANSI screen of the selected pane | Worker drains to the newest request, so fast scrolling never queues stale reads |
| `app.rs` | State, key handling, refresh timers, focus on Enter | No I/O in key handlers except the final focus call |
| `ui.rs` | Layout and rendering | Only the visible list window is built each frame |
| `config.rs` | Optional `config.toml` | `deny_unknown_fields`: typos are errors |
| `bench_tests.rs` | 5,000-workspace scale test | Perf gate; see below |

## Design decisions (and why not to "fix" them)

### Two matchers

Names and content use different matching on purpose.

- **Names** (label, tab/pane/agent names, agent terminal titles, path, repo,
  branch) use nucleo fuzzy matching, but stricter than fzf:
  - Each query word is matched against each field **separately**
    (`Searcher::match_row`). The joined `Row::haystack` is only a cheap
    pre-filter, never a source of hits. Matching across the joined string let
    `asdasd` hit three workspaces as a scattered subsequence.
  - A fuzzy match must score ≥ `MIN_QUALITY_PERCENT` (60) of a perfect match of
    the same word (`atom_indices`). Calibrated on real names: substrings,
    prefixes, and initials (`wrcb`, `wqlod`) score 68–100%; junk like `asd`
    inside a long name scores ≤ 53%. Change it only with
    `nonsense_matches_nothing` and `real_queries_still_find_their_workspace`
    both green.
  - Paths match only their last two segments (`path_tail`), because prefixes
    like `~/.herdr/worktrees/` contain every common letter.
  - Shell window titles (`user@host:~/dir`) are not fields (`is_shell_title`):
    they only repeat the cwd.
- **Content** (screen lines, chat messages) uses literal/regex matching
  (`content::Query`): 3+ non-space characters, every word in the same document,
  smart case, `'phrase`, `/regex/`. Fuzzy matching over prose matches anything.

### Ranking

`MatchKind::tier`: label → tab/pane/agent/title → path/repo/branch → chat →
screen. Within a tier, higher score first; ties keep workspace order (stable
sort). Content hits in the **current** workspace rank after other workspaces,
because your own session usually contains what you just typed.

### What gets indexed

- Screen: `Row::indexed_panes` = panes with a detected agent + each workspace's
  active pane, last `screen_lines` (500) rows of `recent_unwrapped`. Other panes
  are not indexed, to keep indexing proportional to agents, not to all panes.
- The pane the picker runs in (`HERDR_PANE_ID`) is skipped (`index::own_pane`);
  its scrollback echoes the query. Popups have no pane id, so nothing is skipped
  in normal use.
- Transcripts: agent panes whose agent is in `config.content.agents` and
  supported by `transcript::supported`.
- The selected workspace re-indexes every `CONTENT_REFRESH` (2 s). Transcripts
  carry a version (mtime, or OpenCode `time_updated`); `index::Versions` skips
  re-reading unchanged sessions. `ctrl-r` clears versions and re-indexes all.

### Transcript mapping

1. Use herdr's `agent_session` (`{kind: id|path, value}`) when the agent's
   integration reports one.
2. Otherwise the newest session for the pane's cwd: Claude
   `~/.claude/projects/<cwd with non-alphanumerics → '-'>/*.jsonl`
   (`CLAUDE_CONFIG_DIR`), Codex rollouts whose `session_meta.cwd` matches
   (newest 300 scanned, `CODEX_HOME`), OpenCode `session.directory`
   (`$XDG_DATA_HOME/opencode/opencode.db`, top-level sessions only).

Readers keep only user prompts and assistant text (no tool calls, tool output,
reasoning, sidechains, meta, or slash-command plumbing), cap at
`transcript_messages` / `transcript_bytes`, and read JSONL from the tail with a
bounded budget.

### Preview

`View::Live` shows the active pane's visible screen (ANSI, bottom-aligned, re-read
every 400 ms). `View::Hits` shows the selected content hit in context (the whole
chat message, or ±half a screen of neighbouring lines from the same pane), wrapped
and scrolled to the hit. Content-only rows default to `Hits`; `ctrl-s` toggles;
`alt-j/k` cycles hits. Enter in `Hits` view on an agent pane calls `agent.focus`
(falls back to `workspace.focus`).

### Launch path

The action (`herdr/open.sh`) runs server-side without a TTY, so it only calls
`plugin pane open`; the manifest's `[[panes]]` entry (`placement = "popup"`) runs
the binary. Because a keybinding has nowhere to print errors, `open.sh` turns a
missing binary, `ui_busy`, or any herdr error into a `herdr notification show`
toast.

## Performance budget

`bench_tests::five_thousand_workspaces_stay_fast` (5,000 workspaces × 3 tabs × 2
panes) asserts `build_rows` and per-keystroke search each stay under 50 ms in
release (20× that in debug). Currently ~10 ms and ~2 ms.

Rules for new code:

- Nothing on the UI thread does I/O except the snapshot (start, `ctrl-r`) and
  the final focus call.
- New background work goes through `pool::Pool`; use `urgent` only for the
  selected row.
- Per-keystroke work may not allocate per row beyond the resulting hits.
  Precompute in `build_rows` / `refresh_haystack` instead.
- Rendering builds only the visible list window (`ui::window_start`).

## Herdr surface used

| Call | Where | Notes |
| --- | --- | --- |
| `session.snapshot` | `Client::snapshot` | workspaces, tabs, panes (incl. `agent`, `agent_session`, `terminal_title_stripped`), layouts (`focused_pane_id` per tab) |
| `pane.read` `source: visible, format: ansi, strip_ansi: false` | preview | |
| `pane.read` `source: recent_unwrapped, format: text, lines: N` | screen index | One slow pane can take ~300 ms herdr-side; it's on the pool |
| `workspace.focus {workspace_id}` | Enter | |
| `agent.focus {target: pane_id}` | Enter on an agent hit | |
| `plugin pane open` / `notification show` | `herdr/open.sh` | via `$HERDR_BIN_PATH` |

Env read by the binary: `HERDR_SOCKET_PATH` (fallback
`~/.config/herdr[/sessions/$HERDR_SESSION]/herdr.sock`), `HERDR_PANE_ID`,
`HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_ENTRYPOINT_ID`, `HOME`,
`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME`. Verify shapes with
`herdr api schema --json` before adding calls.

## Recipes

- **New searchable name**: add a `FieldKind` variant, push it in
  `model::collect_fields`, add its badge in `MatchKind::badge`. Add a case to
  `real_rows()` / `real_queries_still_find_their_workspace`.
- **New content source**: add a `content::Source` variant (and `badge`), an
  `index::Job` variant produced by `tasks_for` and executed in `index::run`, and
  return `segment: None` when unchanged. Gate it in `config.rs`.
- **New transcript agent**: a module in `transcript.rs` with `find` and `read`/
  `parse`, wired into `supported`, `locate`, and `read`. Add a parse test with
  real-shaped lines, and add the agent to `Content::default().agents`.
- **New key**: `App::on_key`, then the README Keys table, the footer hint in
  `ui::draw_search` if it's important, and `USAGE` in `main.rs`.
- **New config option**: `config::Content` (+ default), the module doc in
  `config.rs`, and the README Configuration section.
- **Index all panes** (not done): would be a `[content] panes = "all"` option
  read in `Row::indexed_panes`. Keep the current default; re-check the scale test.

## Working on it

```sh
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # this Mac: cargo isn't on PATH by default
cargo test                                          # unit + scale (debug budget)
cargo test --release five_thousand -- --nocapture   # real perf numbers
cargo clippy --all-targets                          # pedantic; must print no warnings
cargo fmt
HERDR_PICKER_FROM_SOURCE=1 bash herdr/install.sh    # build into bin/ (what the plugin runs)
```

The local checkout is used only when linked (`herdr plugin link .`); a GitHub
install runs its own managed copy. See [docs/TESTING.md](docs/TESTING.md) for
headless checks, driving the TUI, install-path tests, and release verification.

## Gotchas

- **Edit by reading first.** `cargo fmt` reflows code, so text you remember (or
  wrote earlier) often no longer matches. Read the current lines, then edit.
- **Version bumps change `Cargo.lock`.** Run `cargo build` after bumping, and
  confirm `cargo build --locked` passes before tagging; CI builds `--locked`.
- **`herdr plugin link` never runs `[[build]]`.** A fresh checkout has no
  `bin/herdr-picker` until `herdr/install.sh` runs.
- **Scripts run in minimal environments.** Build steps may not get the
  interactive PATH, and macOS ships bash 3.2: no `mapfile`, guard empty arrays
  with `${a[@]+"${a[@]}"}`.
- **Pane labels contain `›`** (`name › title`); `PaneRow::short_name` takes the
  first part for badges.
- **Don't test in the user's focused pane or tab.** Use a background tab (see
  the testing guide); popups opened for testing interrupt the user.
