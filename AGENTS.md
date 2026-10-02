# herdr-plugins

Plugins for [herdr](https://herdr.dev), a terminal runtime for coding agents.
Each top-level directory is one separately installable plugin. Read the
plugin's own `AGENTS.md` before changing it.

| Plugin | Language | Guide |
| --- | --- | --- |
| `herdr-picker/` | Rust | [herdr-picker/AGENTS.md](herdr-picker/AGENTS.md) |

## How herdr plugins work

- A plugin is a directory with a `herdr-plugin.toml` manifest. Herdr runs the
  argv commands it declares (actions, panes, event hooks, build steps). There is
  no SDK: plugins call back into herdr through the CLI (`$HERDR_BIN_PATH`) or
  the socket (`$HERDR_SOCKET_PATH`, newline-delimited JSON).
- Users install one plugin by path: `herdr plugin install wrn14897/herdr-plugins/<dir>`.
  Herdr clones the repo, runs that plugin's `[[build]]` commands, and registers it.
- `herdr plugin link <dir>` registers a local checkout **without running build
  steps**. After editing a manifest, `herdr plugin unlink <id>` and link again.
- Keybindings live in the user's `~/.config/herdr/config.toml`
  (`[[keys.command]] type = "plugin_action"`), never in the manifest, and need
  `herdr server reload-config`.
- Authoritative references: `herdr --help`, `herdr api schema --json`, and the
  docs at https://herdr.dev/docs/plugins/ and https://herdr.dev/docs/socket-api/.
  Verify request and response shapes against the schema before relying on them.

## Repo conventions

- **Plugin ids** are namespaced `warren.<name>`, so action ids such as
  `warren.herdr-picker.open` never collide with other plugins.
- **Release tags are per plugin**: `<name>-v<version>` (e.g.
  `herdr-picker-v0.2.1`). Each plugin has its own workflow under
  `.github/workflows/`, triggered only by its tag pattern, and its install
  script downloads assets from its own tag.
- **Versions** live in both `herdr-plugin.toml` and the language manifest
  (`Cargo.toml`); keep them equal. CI fails the release if the tag does not
  match the manifest.
- **Marketplace**: the repo has the `herdr-plugin` GitHub topic; the herdr
  marketplace indexes every `herdr-plugin.toml` on `main` every 30 minutes.

## Git

- Branches: `warren/<topic>`.
- Conventional commits scoped by plugin: `feat(herdr-picker): …`,
  `fix(herdr-picker): …`, `docs: …`, `ci: …`.
- Keep commits reviewable: one concern each (perf, feature, fix, release).

## Releasing a plugin

1. Bump `version` in `herdr-plugin.toml` and `Cargo.toml`.
2. Regenerate the lockfile: `cargo build --release` then confirm
   `cargo build --release --locked` passes. A stale `Cargo.lock` fails every CI
   job, because the workflow builds with `--locked`.
3. Add the version's entry to the plugin's `CHANGELOG.md`.
4. Commit `chore(<plugin>): release <version>`, push `main`, then push the tag
   `<plugin>-v<version>`.
5. `gh run watch` until all targets pass, then check the release has a
   `.tar.gz` and `.sha256` per target.
6. Verify the published artifact, not your local build:
   `herdr plugin uninstall <id>` →
   `herdr plugin install wrn14897/herdr-plugins/<plugin> --yes` → run the
   plugin's headless checks against the installed root (see its testing guide).
