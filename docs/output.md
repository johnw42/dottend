# Output modes

See also: [Interactive TUI](tui.md), [Development](development.md).

`ui::fancy()` is true when stdout is a terminal (checked once).

- **Not a terminal (plain mode):** the output must stay **byte-for-byte identical** to dotter's
  classic output: simplelog's `[ INFO]`-style prefixes, crossterm-colored `[+]`/`[-]`/`[~]`
  markers with Debug-quoted paths, the `diff`-crate renderer, the classic prompts and error
  format. This includes the ANSI codes crossterm emits even into pipes. It is a hard
  requirement from the user. Don't change plain-mode strings. The only accepted differences are
  source line numbers in `-vv` debug tags.
- **Terminal (fancy mode):** `ui::Logger` (writes through `anstream`, honors `NO_COLOR` and
  `CLICOLOR_FORCE`) prints `error:`/`warning:` labels. Actions look like
  `+ symlink   src → ~/target` (`ui::action`, `ui::path` shortens `$HOME`). Diffs use
  `Diff::rows` with `similar`. `deploy` prints a closing `Summary`. The prompt is `dialoguer`.

Every user-visible output site branches on `fancy()`: `ui::action`, `Diff::print`,
`display_error`, the "Refusing because…" line in `actions.rs`, the skipped-files error and
summary in `deploy.rs`, watch's "Deploying..." line, and the prompt.

**How to verify plain mode:** build the old binary (a `git worktree` at the base commit, with
its own `--target-dir`) and the new one. Run the same scenarios in a fresh repo at a fixed path
with a fake `$HOME`, all output piped. Then `diff -r` the captured stdout, stderr and exit codes.

**`Diff::has_changes`** uses `diff::lines` in both modes, because it drives deploy decisions.
A missing final newline *does* count as a change.

## Capture mode

While the TUI is open, `ui::start_capture()` reroutes log records into a buffer, which the TUI
drains into its message area (`ui::take_captured`).

- `hooks.rs` pipes hook stdout and stderr into the buffer instead of inheriting the terminal.
- `confirm_delete_empty_dir` answers no.
- The TUI also sets the log level to Error, because every TUI action forces, so "Forcing."
  warnings would be noise.
