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

    match opt.action.clone().unwrap_or_default() {
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
