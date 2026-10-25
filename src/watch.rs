use anyhow::{Context, Result};
use watchexec::error::RuntimeError;
use watchexec::filter::Filterer;
use watchexec::sources::fs::Watcher;
use watchexec::{Config, WatchedPath, Watchexec};
use watchexec_events::filekind::FileEventKind;
use watchexec_events::{Event, Priority, Tag};
use watchexec_filterer_globset::GlobsetFilterer;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::display_error;
use crate::args::Options;
use crate::deploy;
use crate::tui;

/// Discards events that don't modify anything, since deploying reads the repository's files and
/// would otherwise retrigger itself forever.
///
/// Optionally also passes events for the installed files, which are outside the repository.
#[derive(Debug)]
struct ModificationFilterer {
    repository: GlobsetFilterer,
    installed: Option<InstalledFiles>,
}

#[derive(Debug)]
struct InstalledFiles {
    origin: PathBuf,
    targets: Arc<RwLock<BTreeSet<PathBuf>>>,
}

impl Filterer for ModificationFilterer {
    fn check_event(&self, event: &Event, priority: Priority) -> Result<bool, RuntimeError> {
        let is_access = event
            .tags
            .iter()
            .any(|tag| matches!(tag, Tag::FileEventKind(FileEventKind::Access(_))));
        if is_access {
            return Ok(false);
        }

        if let Some(installed) = &self.installed {
            let targets = installed.targets.read().unwrap();
            if event.paths().any(|(path, _)| targets.contains(path)) {
                return Ok(true);
            }
            // Other files in the installed files' directories are irrelevant
            let mut paths = event.paths().peekable();
            if paths.peek().is_some() && paths.all(|(path, _)| !path.starts_with(&installed.origin))
            {
                return Ok(false);
            }
        }

        self.repository.check_event(event, priority)
    }
}

async fn repository_filterer(opt: &Options, origin: &Path) -> Result<GlobsetFilterer> {
    let filterer = GlobsetFilterer::new(
        origin,
        vec![],
        // Directories are ignored with both `dir` and `dir/**`: a `dir/` pattern alone doesn't
        // reliably exclude the files inside it.
        vec![
            (format!("{}", opt.cache_directory.display()), None),
            (format!("{}/**", opt.cache_directory.display()), None),
            (format!("{}", opt.cache_file.display()), None),
            (".git".to_string(), None),
            (".git/**".to_string(), None),
            ("DOTTER_SYMLINK_TEST".to_string(), None),
        ],
        vec![],
        vec![],
        vec![],
    )
    .await?;
    Ok(filterer)
}

/// Returns whether the deploy succeeded without skipping anything.
fn deploy_and_report(opt: &Options, changes_detected: bool) -> bool {
    if crate::ui::fancy() {
        let message = if changes_detected {
            "Changes detected, deploying..."
        } else {
            "Deploying..."
        };
        anstream::println!("{} {message}", crate::ui::paint(crate::ui::HINT, "::"));
    } else {
        println!("[Dotter] Deploying...");
    }
    match deploy::deploy(opt) {
        Ok(error_occurred) => !error_occurred,
        Err(e) => {
            display_error(e);
            false
        }
    }
}

pub(crate) async fn watch(opt: Options, no_tui: bool) -> Result<()> {
    // The interface needs a terminal, and stdin can't hold both a patch and keypresses
    if no_tui || !crate::ui::fancy() || opt.patch {
        watch_and_deploy(opt).await
    } else {
        watch_interactive(opt).await
    }
}

/// Deploys whenever the repository changes.
async fn watch_and_deploy(opt: Options) -> Result<()> {
    let config = Config::default();

    config.file_watcher(Watcher::Native);
    config.pathset(["."]);

    let repository = repository_filterer(&opt, &std::env::current_dir()?).await?;
    config.filterer(ModificationFilterer {
        repository,
        installed: None,
    });

    deploy_and_report(&opt, false);

    config.on_action(move |mut action| {
        if action.signals().next().is_some() {
            action.quit();
            return action;
        }

        debug!("Changes detected in watched files.");
        trace!("Changed files: {:#?}", action.paths().collect::<Vec<_>>());

        deploy_and_report(&opt, true);

        action
    });

    config.on_error(move |e| {
        log::error!("Watcher error: {e:#?}");
    });

    let we = Watchexec::with_config(config)?;
    we.main().await.context("run watchexec main loop")??;
    Ok(())
}

/// Shows the differences between the repository and the installed files in a fullscreen
/// interface, refreshing whenever either changes.
async fn watch_interactive(opt: Options) -> Result<()> {
    // Deploy before opening the interface, like `dotter deploy`: output stays in the terminal and
    // sudo can ask for a password. Whatever couldn't be deployed is then listed in the interface.
    let startup_message = if deploy_and_report(&opt, false) {
        (log::Level::Info, "Deployed on startup".to_string())
    } else {
        (
            log::Level::Warn,
            "The startup deploy had problems; its output is in the terminal after you quit"
                .to_string(),
        )
    };

    let origin = crate::filesystem::real_path(Path::new(".")).context("get current directory")?;
    let targets = Arc::new(RwLock::new(BTreeSet::new()));

    let config = Config::default();
    config.file_watcher(Watcher::Native);
    config.pathset(["."]);
    config.filterer(ModificationFilterer {
        repository: repository_filterer(&opt, &origin).await?,
        installed: Some(InstalledFiles {
            origin: origin.clone(),
            targets: targets.clone(),
        }),
    });

    let (sender, receiver) = std::sync::mpsc::channel();
    config.on_action(move |mut action| {
        if action.signals().next().is_some() {
            let _ = sender.send(tui::Message::Quit);
            action.quit();
            return action;
        }

        trace!("Changed files: {:#?}", action.paths().collect::<Vec<_>>());
        let _ = sender.send(tui::Message::Refresh);
        action
    });

    config.on_error(move |e| {
        log::error!("Watcher error: {e:#?}");
    });

    let we = Watchexec::with_config(config)?;
    let main = we.main();
    let watch_config = we.config.clone();

    let interface = tokio::task::spawn_blocking(move || {
        tui::run(&opt, receiver, startup_message, |installed| {
            *targets.write().unwrap() = installed.clone();

            // Watch the directories containing the installed files rather than the files
            // themselves, so that files which are replaced or created are noticed too.
            let directories: BTreeSet<PathBuf> = installed
                .iter()
                .filter_map(|target| target.parent())
                .filter(|dir| dir.is_dir() && !dir.starts_with(&origin))
                .map(Path::to_owned)
                .collect();
            watch_config.pathset(
                std::iter::once(WatchedPath::recursive("."))
                    .chain(directories.into_iter().map(WatchedPath::non_recursive)),
            );
        })
    });

    let result = interface.await.context("run interactive interface");
    main.abort();
    result?
}
