//! Terminal presentation: styles, the logger backend, and interactive prompts.
//!
//! Output is styled through `anstream`, which strips the styling when the stream isn't a terminal
//! and honors the `NO_COLOR` / `CLICOLOR_FORCE` conventions.

use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::Mutex;

use anstyle::{AnsiColor, Style};
use log::{Level, LevelFilter, Log, Metadata, Record};

pub const ADDED: Style = AnsiColor::Green.on_default();
pub const REMOVED: Style = AnsiColor::Red.on_default();
pub const CHANGED: Style = AnsiColor::Yellow.on_default();
pub const DIM: Style = Style::new().dimmed();
pub const BOLD: Style = Style::new().bold();
pub const ERROR: Style = AnsiColor::Red.on_default().bold();
pub const WARNING: Style = AnsiColor::Yellow.on_default().bold();
pub const HINT: Style = AnsiColor::Cyan.on_default().bold();

/// Wraps `value` in `style`. The escape codes are removed by `anstream` when not on a terminal.
pub fn paint(style: Style, value: impl Display) -> String {
    format!("{style}{value}{style:#}")
}

fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Displays a path without debug quoting, abbreviating the home directory to `~`.
pub fn path(path: &Path) -> String {
    if let Some(rest) = home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned))
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{MAIN_SEPARATOR}{}", rest.display());
    }
    path.display().to_string()
}

#[derive(Debug, Clone, Copy)]
pub enum Op {
    Add,
    Remove,
    Change,
}

/// A single line describing an action on a file, e.g. `+ symlink   zshrc → ~/.zshrc`
pub fn action(op: Op, kind: &str, source: &Path, target: &Path) -> String {
    let (style, sign) = match op {
        Op::Add => (ADDED, '+'),
        Op::Remove => (REMOVED, '-'),
        Op::Change => (CHANGED, '~'),
    };
    format!(
        "{} {kind:<8}  {} {} {}",
        paint(style.bold(), sign),
        path(source),
        paint(DIM, "→"),
        path(target)
    )
}

// === Logger ===

struct Logger;

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level() && metadata.target().starts_with("dotter")
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) || capture(record.level(), record.args().to_string()) {
            return;
        }

        // Errors and warnings go to stderr, everything else is regular program output.
        match record.level() {
            Level::Error => {
                anstream::eprintln!("{} {}", paint(ERROR, "error:"), record.args());
            }
            Level::Warn => {
                anstream::eprintln!("{} {}", paint(WARNING, "warning:"), record.args());
            }
            Level::Info => {
                anstream::println!("{}", record.args());
            }
            level @ (Level::Debug | Level::Trace) => {
                let location = match (record.file(), record.line()) {
                    (Some(file), Some(line)) => format!(" {file}:{line}"),
                    _ => String::new(),
                };
                anstream::println!(
                    "{} {}",
                    paint(DIM, format!("{}{location}", level.as_str().to_lowercase())),
                    record.args()
                );
            }
        }
    }

    fn flush(&self) {
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
}

pub fn init_logger(level: LevelFilter) {
    log::set_logger(&Logger).expect("logger is only initialized once");
    log::set_max_level(level);
}

// === Capture ===

/// A message collected while capturing.
#[cfg_attr(not(feature = "watch"), allow(dead_code))]
pub struct Captured {
    pub level: Level,
    pub message: String,
}

static CAPTURED: Mutex<Option<Vec<Captured>>> = Mutex::new(None);

/// Starts collecting log records and subprocess output instead of writing them to the terminal,
/// for while a fullscreen interface owns it. Prompts answer with their default while capturing.
#[cfg_attr(not(feature = "watch"), allow(dead_code))]
pub fn start_capture() {
    *CAPTURED.lock().unwrap() = Some(Vec::new());
}

#[cfg_attr(not(feature = "watch"), allow(dead_code))]
pub fn stop_capture() {
    *CAPTURED.lock().unwrap() = None;
}

pub fn capturing() -> bool {
    CAPTURED.lock().unwrap().is_some()
}

/// Collects a message if capturing. Returns whether it was collected.
pub fn capture(level: Level, message: String) -> bool {
    match CAPTURED.lock().unwrap().as_mut() {
        Some(captured) => {
            captured.push(Captured { level, message });
            true
        }
        None => false,
    }
}

/// Returns the messages collected since the last call.
#[cfg_attr(not(feature = "watch"), allow(dead_code))]
pub fn take_captured() -> Vec<Captured> {
    CAPTURED
        .lock()
        .unwrap()
        .as_mut()
        .map(std::mem::take)
        .unwrap_or_default()
}

// === Prompts ===

/// Asks whether to delete a directory that was left empty, defaulting to no.
pub fn confirm_delete_empty_dir(dir: &Path) -> bool {
    if capturing() {
        return false;
    }

    let prompt = format!("Directory {} is now empty. Delete it?", path(dir));
    if io::stdin().is_terminal() && io::stderr().is_terminal() {
        use dialoguer::{Confirm, console::Term, theme::ColorfulTheme};

        return Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .default(false)
            .interact_on_opt(&Term::stderr())
            .ok()
            .flatten()
            .unwrap_or(false);
    }

    // Not interactive, so read an answer from stdin, where it may have been piped in
    ask_boolean(&format!("{prompt} [y/N]"))
}

fn ask_boolean(prompt: &str) -> bool {
    let mut buf = String::from("a"); // enter the loop at least once
    while !(buf.to_lowercase().starts_with('y')
        || buf.to_lowercase().starts_with('n')
        || buf.is_empty())
    {
        eprintln!("{prompt}");
        buf.clear();
        io::stdin()
            .read_line(&mut buf)
            .expect("Failed to read line from stdin");
    }

    // If empty defaults to no
    buf.to_lowercase().starts_with('y')
}
