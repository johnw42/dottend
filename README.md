> **Dottend is a fork** of [Dotter](https://github.com/SuperCuber/dotter) by SuperCuber, maintained
> at [johnw42/dottend](https://github.com/johnw42/dottend). It uses the same configuration format,
> including the `.dotter/` directory and the `dotter.*` template variables, but adds an interactive
> `dottend watch`, changes the command-line output, and requires the latest stable Rust. See
> [What's different in this fork](#whats-different-in-this-fork). The text below is adapted from
> the original README.

# What is Dottend?
Dottend is a dotfile manager and templater.

Dotfiles are *configuration* files that usually live in the home directory and start with a dot.
Often times, it is desirable to have a backup of all the configurations on your system, which is why a lot of users have their dotfiles saved in a git repository, then symlinking them to their target locations using `ln -s`.

However, there are several issues with that barebones approach:
- Hard to keep track of what comes from where once you have more than a handful of dotfiles
- Tedious to setup on a new machine - you need to manually create every single link
- No real way to handle differences between machines - say you want the battery meter on your bar to not appear on your desktop machine

Dottend aims to solve all those problems by providing a flexible configuration and automatic templating or symlinking to the target locations.

# What's different in this fork
- **Interactive `dottend watch`.** In a terminal, `dottend watch` deploys once, then opens a fullscreen view of every difference between the repository and the installed files, refreshing as either changes. Each change is shown as a diff, and you can deploy the repository's version or copy the installed file back into the repository (offered only when nothing would be lost, so never for real templates). It also watches the installed files, so edits made outside the repository show up immediately. When a change needs root, it steps aside for `sudo` to ask for your password. Use `dottend watch --no-tui` for the old behavior of deploying on every change.
- **Resolve deploy problems interactively.** When `dottend deploy` can't deploy some files (for example because an installed file was edited), it opens the same view on just those files, so you can deploy or copy back each one. It only does this in a terminal; use `--no-tui` to only report the problems.
- **New output format.** Errors and warnings are labelled `error:`/`warning:`, file actions read like `+ symlink   bashrc → ~/.bashrc`, diffs have `+`/`-` signs with changed words highlighted, and deploys end with a summary such as `Deployed: 2 added, 1 skipped`. Colors honor `NO_COLOR` and `CLICOLOR_FORCE`. The output differs from the original Dotter's, so scripts that parse it may need updating.
- **A new name.** The program is called `dottend`, so it can be installed alongside the original `dotter`.
- **Watch fixes.** `dottend watch` no longer redeploys in an endless loop (also fixed upstream), no longer prints errors for git's short-lived lock files, and exits when its terminal is closed.
- **Requires the latest stable Rust** (currently 1.99).

# Installation
The packages for Homebrew, the AUR, Scoop and crates.io, and the release binaries, all install the original Dotter. To install this fork, use Cargo with an up-to-date Rust toolchain:

```sh
cargo install --locked --git https://github.com/johnw42/dottend
```

Or from a clone of the repository:

```sh
cargo install --locked --path .
```

`--locked` builds with the dependency versions the fork is tested with.

# Wiki
The original project's [wiki](https://github.com/SuperCuber/dotter/wiki) applies to this fork too, since the configuration format is the same.
Among other things, it explains how to setup and configure Dotter (and so Dottend), as well as giving insight on how the templating and deployment works.

# Usage
Now that you've configured all the global and local file sections, you can simply run `dottend` from within your repository.\
All the files will be deployed to their target locations.

Check out `dottend -h` for the command-line flags that Dottend supports:

```
A dotfile manager and templater written in rust

A dotfile manager and templater written in rust

Usage: dottend [OPTIONS] [COMMAND]

Commands:
  deploy           Deploy the files to their respective targets. This is the default subcommand
  undeploy         Delete all deployed files from their target locations. Note that this operates on all files that are currently in cache
  init             Initialize global.toml with a single package containing all the files in the current directory pointing to a dummy value and a local.toml that selects that package
  watch            Run continuously, watching the repository and the deployed files for changes
  gen-completions  Generate shell completions
  help             Print this message or the help of the given subcommand(s)

Options:
  -g, --global-config <GLOBAL_CONFIG>
          Location of the global configuration [default: .dotter/global.toml]
  -l, --local-config <LOCAL_CONFIG>
          Location of the local configuration [default: .dotter/local.toml]
      --cache-file <CACHE_FILE>
          Location of cache file [default: .dotter/cache.toml]
      --cache-directory <CACHE_DIRECTORY>
          Directory to cache into [default: .dotter/cache]
      --pre-deploy <PRE_DEPLOY>
          Location of optional pre-deploy hook [default: .dotter/pre_deploy.sh]
      --post-deploy <POST_DEPLOY>
          Location of optional post-deploy hook [default: .dotter/post_deploy.sh]
      --pre-undeploy <PRE_UNDEPLOY>
          Location of optional pre-undeploy hook [default: .dotter/pre_undeploy.sh]
      --post-undeploy <POST_UNDEPLOY>
          Location of optional post-undeploy hook [default: .dotter/post_undeploy.sh]
  -d, --dry-run
          Dry run - don't do anything, only print information. Implies -v at least once
  -v, --verbose...
          Verbosity level - specify up to 3 times to get more detailed output. Specifying at least once prints the differences between what was before and after Dottend's run
  -q, --quiet
          Quiet - only print errors
  -f, --force
          Force - instead of skipping, overwrite target files if their content is unexpected. Overrides --dry-run
  -y, --noconfirm
          Assume "yes" instead of prompting when removing empty directories
      --no-tui
          Don't open the interactive interface: `deploy` just reports files it couldn't deploy, and `watch` deploys changes automatically
  -p, --patch
          Take standard input as an additional files/variables patch, added after evaluating `local.toml`. Assumes --noconfirm flag because all of stdin is taken as the patch
      --diff-context-lines <DIFF_CONTEXT_LINES>
          Amount of lines that are printed before and after a diff hunk [default: 3]
  -h, --help
          Print help
  -V, --version
          Print version
```

# Contributing
Issues and pull requests for this fork go to [johnw42/dottend](https://github.com/johnw42/dottend). Bugs that also affect the original Dotter are worth reporting [upstream](https://github.com/SuperCuber/dotter) too.

# Credits
Dotter was created by [SuperCuber](https://github.com/SuperCuber). To support the original author, you can donate via [PayPal](https://www.paypal.com/cgi-bin/webscr?cmd=_s-xclick&hosted_button_id=329HKDXK9UB84).
