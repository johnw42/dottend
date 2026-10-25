# Interactive TUI

See also: [Watch mode](watch.md), [Output](output.md), [Deploying](deploy.md).

## `changes::Plan`

`Plan` is the loaded config plus cache. `Plan::changes()` compares each file's **installed file
with a fresh render**. (Normal deploy compares with the cache file instead.) Each `Change` has
one of these `State`s:

| State | Meaning |
|---|---|
| `NotInstalled` | the target doesn't exist yet |
| `Modified` | the installed template differs from what the repo renders |
| `ReplacedByFile` | a symlink was replaced by a regular file or directory |
| `WrongLink(dest)` | the symlink points somewhere else |
| `Removed` | in the cache but no longer in the config |
| `Untracked` | the installed file already matches but isn't in the cache; deploying records it, which is how pre-existing files are adopted |
| `Problem(msg)` | the file can't be compared or rendered |

`diff` holds the installed contents as old and the repo's output as new. `copy_back_blocked`
gives the reason copy-back is refused, if it is. `needs_root` is true when the target has an
`owner`, or the user can't write to the target or the nearest existing ancestor directory
(`libc::access`).

**Copy-back is only allowed when nothing is lost.** For templates, the source must have no `{{`
and no `append` or `prepend` (otherwise the template source would be overwritten with rendered
text). For symlinks, both the replaced target and the source must be regular files. It's
re-checked at apply time.

**`Plan::apply(opt, changes, direction)`:**

1. Runs `pre_deploy`.
2. Applies removals first.
3. Calls the `actions::*` functions with `force = true`. For copy-back it first runs
   `fs::copy(target, source)`, then deploys, which turns the target back into a symlink or
   refreshes the template's cache file.
4. Saves the cache and runs `post_deploy`.

It returns failure messages. "Cached" means `cache.<map>.get(source) == Some(target)`, not just
key presence, because a source can move to a new target.

## `tui::run`

- **Main loop:** waits on the channel with a 250 ms timeout (so captured messages show up) and
  redraws only when something changed, so idle CPU is zero.
- **`InputReader`:** reads terminal input on a separate std thread and sends
  `Message::Terminal(event)`. **Why:** when the terminal hangs up, crossterm's `event::poll`
  spins forever instead of returning an error. If input were read on the main loop, the `Quit`
  sent on SIGHUP would never be seen, and the process would be orphaned at 100% CPU. On a
  separate thread, a spinning read can't stop the process from exiting.
- **Confirmation:** every action asks y/n first (`Confirmation`). The confirmation stores
  `Change::key()`s, not list indices, and re-checks eligibility on `y`, because a refresh can
  land while the popup is open.
- **Elevation:** if any chosen change has `needs_root`, or a hook file contains `sudo`, and
  `sudo -n true` fails, the action is held in `App::elevation`. The main loop then pauses
  `InputReader` and waits until it's idle (otherwise it would swallow the password keystrokes),
  leaves raw mode and the alternate screen, runs `sudo -v`, re-enters the TUI, and applies the
  action. Hooks only get pre-authenticated if the hook file itself says `sudo`; scripts it calls
  aren't inspected.
- **Layout:** header; file list (paths truncated from the left with `…`); diff pane (the
  description wraps via `tui::wrap`); message area; key help; centered confirm popup, sized from
  the wrapped text, with its own row for the keys.
