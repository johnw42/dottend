#[macro_use]
extern crate log;

mod actions;
mod args;
mod changes;
mod config;
mod deploy;
mod difference;
mod filesystem;
mod handlebars_helpers;
mod hooks;
mod init;
mod tui;
mod ui;
#[cfg(feature = "watch")]
mod watch;

use std::fmt::Write;
use std::io;

use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::{generate, generate_to};

fn main() {
    match run() {
        Ok(success) if success => std::process::exit(0),
        Ok(_) => std::process::exit(1),
        Err(e) => {
            display_error(e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn display_error(error: anyhow::Error) {
    let mut chain = error.chain();
    let mut error_message = format!("Failed to {}", chain.next().unwrap());

    for e in chain {
        write!(
            error_message,
            "\n  {} {e}",
            ui::paint(ui::DIM, "caused by:")
        )
        .unwrap();
    }

    error!("{}", error_message);
}

/// Fails with directions to migrate if the repository still has upstream's `.dotter/` directory,
/// instead of a bare "file not found". This is the only place the old name is used. A leftover
/// `.dotter/` counts even when `dottend/` exists, since `git pull` moves the tracked files but
/// leaves `local.toml` and the cache behind.
fn check_old_config_dir(global_config: &std::path::Path) -> Result<()> {
    if global_config != std::path::Path::new("dottend/global.toml")
        || !std::path::Path::new(".dotter").is_dir()
    {
        return Ok(());
    }
    let action = if std::path::Path::new("dottend").exists() {
        "Move the files left in .dotter/, such as local.toml, cache.toml and cache/, into \
         dottend/, then delete .dotter/."
    } else {
        "Rename it with `mv .dotter dottend`."
    };
    Err(anyhow::anyhow!(
        "found the old .dotter/ directory; dottend now keeps its configuration in dottend/. \
         {action} Templates also need the `dottend.*` variables, not `dotter.*` (an `if` on a \
         `dotter.*` variable is always false)."
    )
    .context("find the configuration directory"))
}

/// Returns true if program should exit with success status
fn run() -> Result<bool> {
    // Parse arguments
    let opt = args::get_options();

    use log::LevelFilter;

    ui::init_logger(if opt.quiet {
        LevelFilter::Error
    } else {
        match opt.verbosity {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            3 => LevelFilter::Trace,
            _ => unreachable!(),
        }
    });

    trace!("Loaded options: {:#?}", opt);

    #[cfg(unix)]
    {
        use std::os::unix::prelude::MetadataExt;
        if std::env::var("USER").unwrap_or_default() == "root"
            && !std::fs::metadata(&opt.global_config).is_ok_and(|m| m.uid() == 0)
        {
            warn!("It is not recommended to run Dottend as root, since the cache files and all files not marked with an `owner` field will default to being owned by root.
If you're truly logged in as root, it is safe to ignore this message.
Otherwise, run `dottend undeploy` as root, remove cache.toml and cache/ folders, then use Dottend as a regular user.");
        }
    }

    let action = opt.action.clone().unwrap_or_default();
    if !matches!(
        action,
        args::Action::Init | args::Action::GenCompletions { .. }
    ) {
        check_old_config_dir(&opt.global_config)?;
    }

    match action {
        args::Action::Deploy => {
            debug!("Deploying...");
            if deploy::deploy(&opt).context("deploy")? {
                // Some files were skipped or failed. Let the user resolve them if they can.
                if tui::can_resolve_interactively(&opt) {
                    return tui::resolve_after_deploy(&opt)
                        .context("resolve files that weren't deployed");
                }
                return Ok(false);
            }
        }
        args::Action::Undeploy => {
            debug!("Un-Deploying...");
            if deploy::undeploy(&opt).context("undeploy")? {
                // An error occurred
                return Ok(false);
            }
        }
        args::Action::Init => {
            debug!("Initializing repo...");
            init::init(opt).context("initalize directory")?;
        }
        #[cfg(feature = "watch")]
        args::Action::Watch => {
            debug!("Watching...");
            tokio::runtime::Runtime::new()
                .expect("create a tokio runtime")
                .block_on(watch::watch(opt))
                .context("watch repository")?;
        }
        args::Action::GenCompletions { shell, to } => {
            if let Some(to) = to {
                generate_to(shell, &mut args::Options::command(), "dottend", to)
                    .context("write completion to a file")?;
            } else {
                generate(
                    shell,
                    &mut args::Options::command(),
                    "dottend",
                    &mut io::stdout(),
                );
            }
        }
    }

    Ok(true)
}
