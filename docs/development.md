# Development

See also: [Overview](README.md).

## Dependencies and toolchain

- **`rust-version = 1.85`:**
  - That's why ratatui is 0.29: 0.30 needs Rust 1.88.
  - ratatui 0.29 depends on crossterm 0.28 while dotter uses 0.29 directly, so two crossterm
    copies are built. TUI code must use `ratatui::crossterm::*` throughout so the two never mix.
- **watchexec** is required at ≥ 8.4 (for `Filterer::check_dir`), with globset filterer
  ≥ 8.0.6. `cargo install` without `--locked` ignores `Cargo.lock` and takes the newest
  compatible versions. That's how a watchexec-version-specific bug showed up in the user's build
  but not in tests. Tell users to `cargo install --locked --path .`.
- **Plain mode** still needs simplelog, crossterm and diff. **Fancy mode** uses anstream,
  anstyle, similar, and dialoguer (default features off).

## CI (`.github/workflows/continuous-integration.yml`)

- `cargo check --locked` for Windows, Linux (musl) and macOS (x86 and aarch64).
- **README check:** the command list in `README.md` must match `dotter --help`. Update it when
  subcommand help text changes.
- `cargo publish --dry-run`, `cargo test`, `cargo fmt --check`, and `cargo clippy -- -D warnings`.

**Local equivalents:** add the targets with `rustup target add x86_64-pc-windows-msvc
x86_64-apple-darwin`; `cargo check` for them works without a linker. Also run clippy with
`--no-default-features`, since `changes.rs` and `tui.rs` disappear and capture functions become
dead code (handled with `cfg_attr(not(feature = "watch"), allow(dead_code))`).

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
