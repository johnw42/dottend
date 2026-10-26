# Deploying

See also: [Configuration](configuration.md), [Output](output.md), [Interactive TUI](tui.md).

## The cache

- **`cache.toml`** (`config::Cache`) has two maps, `symlinks` and `templates`, from source to
  target.
- **`cache/`** (`--cache-directory`) holds the last rendered output of every template at
  `cache/<source>`. Rendered hook scripts go there too.
- **How edits are detected:** `compare_template(target, cache_file)` compares the installed file
  with the cached copy, *not* with a fresh render. If they differ, someone edited the installed
  file, and dottend refuses to overwrite it without `--force`.

## Deploy algorithm (`deploy::run_deploy`)

Keyed by (source, target) pairs:

| Set | Action |
|---|---|
| cached but no longer desired | delete |
| desired but not cached | create |
| both | update |

Each action in `actions.rs` matches on a comparison and decides to act, warn and act anyway,
force, or skip:

- **`SymlinkComparison`:** `Identical`, `OnlySourceExists`, `OnlyTargetExists`, `BothMissing`,
  `Changed` (link points elsewhere), `TargetNotSymlink`.
- **`TemplateComparison`:** `Identical` (target equals cache), `OnlyCacheExists`,
  `OnlyTargetExists`, `BothMissing`, `Changed`, `TargetNotRegularFile`.

Return values: `Ok(true)` means done (the cache is updated), `Ok(false)` means skipped (counted,
and the run exits with status 1), `Err` means failed.

- **`update_template` with `Identical`** (target matches cache): if the fresh render also matches
  the cache, nothing is rewritten; only the owner and mode are corrected if needed. So an
  up-to-date deploy uses no sudo, even for files with an `owner`. Keep it that way: rewriting
  unconditionally made every deploy print "Elevating permissions" warnings.
- **`update_template` with `Changed`:** it first diffs the fresh render against the target. If
  they're equal, it quietly re-renders anyway.
- **Logging:** the action line is printed at info level (visible with `-v`), and diffs only when
  info is enabled.
- **Hooks:** `pre_deploy` and `post_deploy` run around `deploy`, and the undeploy hooks around
  `undeploy`. Nothing runs in dry run. A hook is rendered as a template, then executed directly
  if it's executable, else with `sh`. On Windows a `.bat` next to a `.sh` is preferred.
- **Dry run** uses `DryRunFilesystem`, which only tracks would-be states.

## Filesystem and sudo (`filesystem.rs`, unix)

- **`owner` set:** operations run as `sudo -u <owner> mkdir/ln/tee`, plus `sudo chmod` and
  `sudo chown`. `set_owner` and `copy_permissions` first check whether the owner (resolved with
  `UnixUser::uid`) or the mode is already right, and do nothing if so.
- **No owner:** plain std calls, falling back to `sudo rm` / `sudo rmdir` on `PermissionDenied`.
- **Mode:** `copy_permissions` copies the *source file's* mode onto the target.
- **Empty directories** left behind by a removal are deleted after `ui::confirm_delete_empty_dir`
  unless `--noconfirm`.
