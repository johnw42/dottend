# Development

See also: [Overview](README.md).

## Dependencies and toolchain

- **`rust-version` tracks the latest stable Rust** (currently 1.99). The user wants the fork on
  the newest compiler, so raise it freely, and use new language features (clippy pushes let
  chains, for example).
- **Dependencies:** keep them up to date within their major versions with `cargo update`. Major
  upgrades still need checking for breaking changes; `hostname` 0.4, `mockall` 0.15 and
  `shellexpand` 3 are available but not yet adopted.
- **ratatui** is 0.30 and uses the same crossterm (0.29) that dotter uses directly, so only one
  crossterm is built. TUI code uses it through `ratatui::crossterm`.
- **watchexec** is required at ≥ 8.4 (for `Filterer::check_dir`), with globset filterer
  ≥ 8.0.6. watchexec 8.4's dependencies need Rust 1.87. `cargo install` without `--locked`
  ignores `Cargo.lock` and takes the newest compatible versions, which is how a
  watchexec-version-specific bug once showed up in the user's build but not in tests. Install
  with `cargo install --locked --path .`.
- **Plain mode** still needs simplelog, crossterm and diff. **Fancy mode** uses anstream,
  anstyle, similar, and dialoguer (default features off).

## CI (`.github/workflows/continuous-integration.yml`)

- `cargo check --locked` for Windows, Linux (musl) and macOS (x86 and aarch64).
- **README check:** the command list in `README.md` must match `dotter --help`. Update it when
  subcommand help text changes.
- `cargo publish --dry-run`, `cargo test`, `cargo fmt --check`, and `cargo clippy -- -D warnings`.

**Local equivalents:** run clippy both normally and with `--no-default-features`, since
`changes.rs` and `tui.rs` disappear and capture functions become dead code (handled with
`cfg_attr(not(feature = "watch"), allow(dead_code))`). Checking the Windows and macOS targets
locally no longer works out of the box: some dependencies' build scripts (`psm`, `stacker`) need
a C compiler for the target. Leave those to CI unless the user asks.

## Testing

- **Unit tests:**
  - `deploy.rs`: the deploy algorithm, with `MockActionRunner`.
  - `config.rs`: merging.
  - `filesystem.rs`, `handlebars_helpers.rs`: conditions.
  - `difference.rs`: change detection.
  - `tui.rs`: `wrap` and `truncate_start`.
- **No automated tests** cover `changes.rs` or TUI behavior. Those are tested by hand. Methods
  that work:
  - **Drive the TUI in tmux on a private socket:**
    `tmux -L dottertest -f /dev/null new-session -d -x 120 -y 30 …`, then `send-keys` and
    `capture-pane -p`.
  - **Test sudo flows with a fake `sudo`** first on `PATH`: `-v` checks a password and writes a
    stamp, `-n` succeeds only if the stamp exists, other calls strip `-u <user>` and exec the
    command.
  - **Measure idle CPU** with `/proc/<pid>/stat` fields 14+15 over a few seconds. Get the PID
    with `pgrep -n` (newest only).
- **Hard-won rules:**
  - Test scripts must `set -euo pipefail` and check that `pwd` is inside the scratch directory
    before any git or file operation. A failed `cd` once ran test git commits in this repo.
  - `tmux kill-server` does **not** reliably end the program inside. Check for leftover
    `dotter watch` processes afterwards and kill them by exact PID. Never use a broad
    `pkill -f`: it can match the user's own processes, or the shell running the command.
  - Debug builds aren't rebuilt by `cargo clippy` or `cargo test`. Run `cargo build` before
    testing `target/debug/dotter`.
