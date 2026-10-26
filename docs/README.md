# Dottend design notes

Notes for working on this codebase: how it fits together, the rules it must keep, and the traps
already found. Function and type names are given instead of line numbers, which go stale; use
grep.

## Contents

- [Configuration](configuration.md): loading and merging `global.toml` and `local.toml`.
- [Deploying](deploy.md): the cache, the deploy algorithm, filesystem access and sudo.
- [Output](output.md): the output format, and capture mode.
- [Watch mode](watch.md): `dottend watch` and how the watcher is filtered.
- [Interactive TUI](tui.md): change detection, copy-back, and the interface.
- [Development](development.md): dependencies and toolchain, CI, testing.

## What it does

Dottend deploys files from a dotfiles repository (the current directory) to their target
locations, either as **symlinks** or as rendered **templates** (Handlebars). It records what it
deployed in a cache, so later runs can update or remove files and can tell when someone edited an
installed file by hand.

Subcommands (`src/args.rs`): `deploy` (default), `undeploy`, `init`, `watch` (feature `watch`),
`gen-completions`.

## Module map

| File | Role |
|---|---|
| `main.rs` | Parses options, sets up logging (`ui::init_logger`), dispatches subcommands, `display_error` formats an `anyhow` chain. |
| `args.rs` | clap options. `get_options` makes `--dry-run` imply `-v` and `--patch` imply `--noconfirm`. |
| `config.rs` | Loads and merges `global.toml` and `local.toml` into a `Configuration`, plus the `Cache` type. |
| `handlebars_helpers.rs` | Builds the Handlebars instance, registers helpers, adds the `dotter.*` variables, filters files by their `if` field. |
| `deploy.rs` | `deploy` / `undeploy`: the set arithmetic between desired files and the cache, hooks, the `Summary` of a run, and `desired_files`, which splits files into symlinks and templates. |
| `actions.rs` | One function per action (create, update or delete × symlink or template), each matching on the on-disk state. The `ActionRunner` trait exists so `deploy.rs` tests can mock it (mockall). |
| `filesystem.rs` | The `Filesystem` trait: `RealFilesystem` (unix and windows versions) and `DryRunFilesystem`. State comparisons (`SymlinkComparison`, `TemplateComparison`), sudo elevation, file loading and saving. |
| `difference.rs` | `Diff` (old and new text): change detection and rendering with `similar`, either printed or as rows for the TUI. |
| `hooks.rs` | Renders a hook script as a template into the cache directory and runs it. |
| `init.rs` | Writes a starter config listing every file in the directory. |
| `ui.rs` | Styles, the logger, capture mode, prompts. |
| `watch.rs` | `dottend watch`: auto-deploy mode and interactive mode (watchexec setup and filtering). |
| `changes.rs` | For the TUI: finds differences between the repo and the installed files (`Plan`, `Change`) and resolves them (`Plan::apply`). |
| `tui.rs` | The ratatui interface, opened by `dottend watch` and by `dottend deploy` when files couldn't be deployed. |

Only `watch.rs` (and watchexec) depends on feature `watch`; `changes.rs` and `tui.rs` are always compiled, since `deploy` uses them too.

## History

This is the user's own fork of Dotter, renamed Dottend (`origin` = johnw42/dottend, `upstream` =
SuperCuber/dotter). It no
longer needs to stay compatible with upstream's output or toolchain: scripts parsing the old output
format aren't supported, and it targets the latest stable Rust.

- The watch redeploy-loop fix was merged upstream as #234.
- The terminal-output modernization (#235) was rejected upstream.
- A draft PR (#236) offers upstream a version-agnostic fix for the watcher's vanished-file
  errors, without the `check_dir` forwarding, which would need Rust 1.87 there.
- The terminal output, TUI, sudo support, and later watcher fixes live only in the fork.
- The program was renamed from `dotter` to `dottend` to tell it apart from upstream, and its
  configuration directory from `.dotter/` to `.dottend/`. `main::check_old_config_dir` explains
  the rename when a repository only has `.dotter/`. Templates still use the `dotter.*` variables,
  so existing templates and the upstream wiki's examples keep working.
- All development happens directly on `main`, the fork's default branch. The old `master` (a
  copy of upstream's) was deleted. Branches for upstream PRs are based on `upstream/master`
  instead.
