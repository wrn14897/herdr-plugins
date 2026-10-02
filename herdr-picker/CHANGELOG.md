# Changelog

All notable changes to herdr-picker. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/). Each version is released as the
git tag `herdr-picker-v<version>`; the release workflow uses this file's
section for that version as the release notes.

## [Unreleased]

### Fixed

- `!word` now excludes in text search too: messages and screen lines
  containing `word` don't match, and a workspace excluded by name is not
  brought back by a text match. Previously `!word` was searched for literally.

### Added

- `AGENTS.md` (architecture for contributors and coding agents),
  `docs/TESTING.md` (local testing guide), and `scripts/tui-test.sh`, which
  drives the TUI in a background tab.
- README sections for requirements, query syntax, troubleshooting, and the
  `transcript_bytes` setting.

## [0.2.1] - 2026-10-02

### Fixed

- Random or misspelled queries no longer match unrelated workspaces. Each word
  now has to match inside a single name with a well-formed match (substring,
  prefix, or initials), instead of anywhere across a long joined string of
  label, full path, and branch. Paths match on their last two segments.
- Text typed into the picker no longer matches its own pane: the pane the
  picker runs in is not screen-indexed.
- `install.sh` explains why the release download failed before falling back to
  a source build.
- Source builds find a cargo that supports Rust 1.85 (trying rustup's stable
  toolchain when the default is older), and say to run `rustup update stable`
  otherwise.
- The open action shows a toast when the binary is missing, another popup is
  open, or herdr refuses, instead of failing silently from a keybinding.

### Changed

- `--search` output includes each match's score.
- `Cargo.toml` declares `rust-version = "1.85"`.

## [0.2.0] - 2026-10-02

### Added

- Search inside workspaces, not just their labels: tab, pane, and agent names
  and agent terminal titles. A second line under the row shows what matched.
- Screen text search over the last 500 rows of every agent pane and each
  workspace's active pane.
- Conversation search over Claude Code, Codex, and OpenCode sessions (prompts
  and replies only), mapped through herdr's `agent_session` or, failing that,
  the newest session in the pane's directory.
- Text queries: all words on the same line or message, smart case,
  `'exact phrase`, and `/regex/`.
- A preview of each text match in context, with `alt-j`/`alt-k` to step
  through matches and `ctrl-s` to switch to the live screen. Enter on a match
  from an agent pane focuses that agent.
- `indexing n/m` progress in the header; the selected workspace re-indexes
  every 2 seconds.
- Optional `config.toml` (`[content] enabled, screen_lines,
  transcript_messages, transcript_bytes, agents`).
- `herdr-picker --search QUERY` prints ranked results without the UI.

### Changed

- Faster with large sessions: linear row building, a 4-thread worker pool
  instead of a thread per directory, and only the visible rows rendered. Tested
  at 5,000 workspaces (about 10 ms to build, 2–3 ms per keystroke).
- Binary grows to about 4 MB, since SQLite is bundled.

### Fixed

- `install.sh` finds cargo installed through Homebrew's rustup when it isn't
  on PATH.
- Text matches late in a long line stay visible in the list.

## [0.1.0] - 2026-10-02

### Added

- Popup picker over open workspaces: fuzzy search bar on top, workspaces on
  the left, preview on the right.
- Preview with the workspace's path, git branch and dirty state, agent status,
  tab/pane tree, and the live screen of its active pane.
- Enter focuses the workspace; the current workspace is listed last.
- Prebuilt binaries for macOS and Linux, downloaded and sha256-verified at
  install, with a cargo fallback.

[Unreleased]: https://github.com/wrn14897/herdr-plugins/compare/herdr-picker-v0.2.1...HEAD
[0.2.1]: https://github.com/wrn14897/herdr-plugins/releases/tag/herdr-picker-v0.2.1
[0.2.0]: https://github.com/wrn14897/herdr-plugins/releases/tag/herdr-picker-v0.2.0
[0.1.0]: https://github.com/wrn14897/herdr-plugins/releases/tag/herdr-picker-v0.1.0
