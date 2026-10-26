# Output modes

See also: [Interactive TUI](tui.md), [Development](development.md).

There is one output format. Everything is written through `anstream`, which strips the styling
when a stream isn't a terminal and honors `NO_COLOR` and `CLICOLOR_FORCE`. There's no separate
"plain" format and no compatibility requirement for scripts parsing the output: this is the
user's own fork, and they dropped that requirement. (Before that, piped output had to match
upstream dotter's classic format byte for byte; that code was removed.)

- **Logging** (`ui::Logger`): errors and warnings go to stderr with `error:`/`warning:` labels.
  Info goes to stdout unprefixed, debug and trace dimmed with their source location. Only targets
  starting with `dottend` (the crate name) are logged.
- **Actions** (`ui::action`): `+ symlink   src → ~/target`, with `ui::path` shortening `$HOME`.
- **Diffs** (`Diff::print`, `Diff::rows`): rendered with `similar`. Lines get `+`/`-` signs, with
  word-level emphasis and a "no newline at end of file" marker.
- **Summary** (`deploy::Summary::print`): `Deployed: 2 added, 1 skipped`, plus a `hint:` about
  `--force` when files were skipped. It prints nothing when no files were added, removed or
  skipped, and nothing with `-q`.
- **Errors** (`display_error`): `error: Failed to <context>` followed by indented `caused by:`
  lines.
- **Prompt** (`ui::confirm_delete_empty_dir`): a `dialoguer` confirmation when stdin and stderr
  are terminals. Otherwise it reads a `y`/`n` line from stdin, so answers can be piped in.

**`Diff::has_changes`** drives deploy decisions, so its meaning matters: line endings (`\n` or
`\r\n`) are ignored, but a missing final newline counts as a change.

## Capture mode

While the TUI is open, `ui::start_capture()` reroutes log records into a buffer, which the TUI
drains into its message area (`ui::take_captured`).

- `hooks.rs` pipes hook stdout and stderr into the buffer instead of inheriting the terminal.
- `confirm_delete_empty_dir` answers no.
- The TUI also sets the log level to Error, because every TUI action forces, so "Forcing."
  warnings would be noise.
