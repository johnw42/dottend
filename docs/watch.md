# Watch mode

See also: [Interactive TUI](tui.md), [Development](development.md).

`watch(opt, no_tui)` picks one of two modes.

## Auto-deploy mode

Used with `--no-tui`, when stdout isn't a terminal, or with `--patch` (stdin can't hold both a
patch and keypresses). `watch_and_deploy` deploys at startup and after every change, as dotter
always did.

## Interactive mode

`watch_interactive` runs these steps:

1. **Deploy normally** before the TUI opens, so output stays in the terminal scrollback and sudo
   can prompt. The result becomes the TUI's first message.
2. **Set up watchexec:** the repository is watched recursively. The parent directory of every
   target is watched non-recursively (via `watch_config.pathset`, updated whenever the target
   set changes). Parents inside the repo are skipped.
3. **Run the TUI** in `spawn_blocking`. watchexec's `on_action` sends `tui::Message::Refresh`
   (or `Quit` on a signal) over an `std::sync::mpsc` channel.

## `ModificationFilterer` (used by both modes)

- **Access events are dropped.** Deploying reads repository files, and treating reads as changes
  caused an endless redeploy loop (fixed upstream in #234).
- **`check_dir` is forwarded to the globset filterer,** so ignored directories (`.git`, the
  cache) aren't watched at all. watchexec ≥ 8.4 calls this hook. Without forwarding, `.git` is
  watched and git's short-lived lock files produce `PathScan` errors.
- **Interactive mode:** events for exact target paths pass. Other paths outside the repo are
  dropped, since they're other files in `$HOME` and similar directories.
- **Ignore patterns** are passed as *ignores* (the third argument of `GlobsetFilterer::new`),
  never as `!`-prefixed *filters*. With no positive filters, the filter list is ignored entirely.
  Each excluded directory needs both `dir` and `dir/**`, because a `dir/` pattern alone doesn't
  exclude the files inside it.

**`report_watcher_error`** drops errors caused by a vanished file (any `io::ErrorKind::NotFound`
in the source chain; logged at debug only) and logs the rest as one line.
