# Dotter design notes

Notes for working on this codebase: how it fits together, the rules it must keep, and the traps
already found. Function and type names are given instead of line numbers, which go stale; use
grep.

## Contents

- [Configuration](configuration.md): loading and merging `global.toml` and `local.toml`.
- [Deploying](deploy.md): the cache, the deploy algorithm, filesystem access and sudo.
- [Output modes](output.md): plain versus terminal output, and capture mode.
- [Watch mode](watch.md): `dotter watch` and how the watcher is filtered.
- [Interactive TUI](tui.md): change detection, copy-back, and the interface.
- [Development](development.md): dependencies and toolchain, CI, testing.

## What it does

Dotter deploys files from a dotfiles repository (the current directory) to their target
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
| `difference.rs` | `Diff` (old and new text): change detection plus two renderers, the classic one (`diff` crate) and the terminal one (`similar`). |
| `hooks.rs` | Renders a hook script as a template into the cache directory and runs it. |
| `init.rs` | Writes a starter config listing every file in the directory. |
| `ui.rs` | The output mode switch (`fancy()`), styles, the terminal logger, capture mode, prompts. |
| `watch.rs` | `dotter watch`: auto-deploy mode and interactive mode (watchexec setup and filtering). |
| `changes.rs` | Interactive mode only: finds differences between the repo and the installed files (`Plan`, `Change`) and resolves them (`Plan::apply`). |
| `tui.rs` | Interactive mode only: the ratatui interface. |

`changes.rs` and `tui.rs` are compiled only with feature `watch`.

## History

This is a fork (`origin` = johnw42/dotter, `upstream` = SuperCuber/dotter).

- The watch redeploy-loop fix was merged upstream as #234.
- The terminal-output modernization (#235) was rejected upstream.
- The terminal output, TUI, sudo support, and later watcher fixes live only on the fork's
  `modernize-cli-output` branch.
