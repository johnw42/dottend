# Configuration

See also: [Deploying](deploy.md), [Overview](README.md).

- **`global.toml`**: packages (`[name]` with `depends`, `files`, `variables`), plus optional
  `[settings] default_target_type` and `[helpers]` (script helpers, feature `scripting`).
- **`local.toml`**: `packages = [...]`, optional `includes`, `files`, `variables`. If it's
  missing, `<hostname>.toml` is tried instead.
- **Merging** (`merge_configuration_files`):
  1. Apply included files to their packages.
  2. Enable packages transitively through `depends`.
  3. Merge the enabled packages. The same file in two packages is an error. The same variable is
     an error unless both values are tables, which are merged recursively.
  4. Apply `local.toml`'s `files` and `variables`, then the `--patch` package read from stdin.
  5. Drop files whose target is `""`.
- **File targets** (`FileTarget`):
  - `Automatic(path)`: resolved by `deploy::desired_files`. A template if the source contains
    `{{` (`filesystem::is_template`), else a symlink. `default_target_type` can force symlinks or
    templates instead.
  - `Symbolic`: symlink. Fields `owner`, `recurse`, `if`.
  - `ComplexTemplate`: template. Fields `owner`, `append`, `prepend`, `if`.
- **Directory sources** are expanded into one entry per file (`expand_directories`). `recurse`
  defaults to true and is set per symlink target.
- **Targets** are passed through `shellexpand::full` (`~`, env vars), so they're absolute.
- **`if` conditions** are evaluated in `handlebars_helpers::filter_files_condition` by rendering
  `{{#if <cond>}}true{{/if}}`.
- **The `dotter` variable** (named after upstream, and kept for compatibility) holds `packages.<name>` (bool), `files`, `os`, `unix`, `windows`,
  `linux`, `macos`, `current_dir`, `hostname`.
- **Handlebars settings:** strict mode is on, so missing variables are errors, and HTML escaping
  is off.
- **Windows without symlink permission:** `desired_files` probes by creating
  `DOTTER_SYMLINK_TEST` in the current directory. If that fails, every file becomes a template
  (copied instead of linked).
