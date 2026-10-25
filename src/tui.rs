//! The fullscreen interface of `dotter watch`, also opened by `dotter deploy` when files couldn't
//! be deployed: lists the differences between the repository and
//! the installed files, shows each one as a diff, and resolves them in either direction.

use anyhow::Result;
use log::Level;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use similar::ChangeTag;

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::args::Options;
use crate::changes::{Change, Direction, Kind, Plan, State};
use crate::difference::DiffRow;
use crate::ui;

/// Sent to the interface by the file watcher, and by the thread reading terminal input.
pub enum Message {
    /// Watched files changed
    #[cfg_attr(not(feature = "watch"), allow(dead_code))]
    Refresh,
    Quit,
    Terminal(Event),
}

const MAX_MESSAGES: usize = 100;
const VISIBLE_MESSAGES: usize = 4;

/// Which command the interface was opened by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `dotter watch`: the files are watched and the interface refreshes when they change
    #[cfg_attr(not(feature = "watch"), allow(dead_code))]
    Watch,
    /// `dotter deploy`: opened to resolve the files a deploy couldn't handle
    Deploy,
}

/// Whether `dotter deploy` should open the interface to resolve files it couldn't deploy: only when
/// someone is at a terminal to use it, and it could change something.
pub fn can_resolve_interactively(opt: &Options) -> bool {
    use std::io::IsTerminal;
    !opt.no_tui
        && !opt.dry_run
        && !opt.quiet
        // stdin held the patch, so it can't also provide keypresses
        && !opt.patch
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

/// Opens the interface after a deploy that couldn't deploy some files. Returns whether everything
/// was resolved.
pub fn resolve_after_deploy(opt: &Options) -> Result<bool> {
    let message = (
        Level::Warn,
        "Some files weren't deployed. Resolve them here; the deploy's output shows when you quit"
            .to_string(),
    );
    run(
        opt,
        Mode::Deploy,
        std::sync::mpsc::channel(),
        message,
        |_| {},
    )
}

/// Runs the interface until the user quits or a `Message::Quit` arrives, starting with
/// `first_message` in the message area. `watch_targets` is called with the installed locations of
/// all deployed files whenever they change, so they can be watched. Returns whether everything
/// was resolved: the configuration loads and no differences are left.
pub fn run(
    opt: &Options,
    mode: Mode,
    (sender, messages): (Sender<Message>, Receiver<Message>),
    first_message: (Level, String),
    mut watch_targets: impl FnMut(&BTreeSet<PathBuf>),
) -> Result<bool> {
    // Collect log output so it doesn't draw over the interface. Only errors are shown, since
    // warnings about forcing changes are expected here.
    ui::start_capture();
    let previous_level = log::max_level();
    log::set_max_level(log::LevelFilter::Error);

    let mut terminal = TerminalGuard(ratatui::init());
    let input = InputReader::spawn(sender);
    let mut app = App::new(opt, mode);
    let mut watched_targets = BTreeSet::new();
    app.message(first_message.0, first_message.1);
    app.refresh();

    let mut redraw = true;
    let result = loop {
        if app.targets != watched_targets {
            watch_targets(&app.targets);
            watched_targets = app.targets.clone();
        }
        redraw |= app.collect_captured();

        if redraw {
            if let Err(e) = terminal.0.draw(|frame| app.draw(frame)) {
                break Err(e.into());
            }
            redraw = false;
        }
        if app.quit {
            break Ok(());
        }

        // Wake up periodically to show messages collected from other threads
        let first = match messages.recv_timeout(Duration::from_millis(250)) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
        };

        let mut refresh = false;
        let mut failure = None;
        for message in std::iter::once(first).chain(messages.try_iter()) {
            match message {
                Message::Refresh => refresh = true,
                Message::Quit => app.quit = true,
                Message::Terminal(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    app.on_key(key);
                    redraw = true;
                }
                Message::Terminal(Event::Resize(..)) => redraw = true,
                Message::Terminal(_) => {}
            }
            if let Some(request) = app.elevation.take() {
                input.pause();
                let elevated = elevate(&mut terminal.0, &mut app, request);
                input.resume();
                if let Err(e) = elevated {
                    failure = Some(e);
                    break;
                }
            }
            if app.quit {
                break;
            }
        }
        if let Some(e) = failure {
            break Err(e);
        }
        if refresh && !app.quit {
            app.refresh();
            redraw = true;
        }
        redraw |= app.quit;
    };

    drop(terminal);
    log::set_max_level(previous_level);
    ui::stop_capture();
    result.map(|()| app.load_error.is_none() && app.changes.is_empty())
}

/// Reads terminal input on its own thread and sends it as messages.
///
/// When the terminal goes away, crossterm's polling spins instead of failing, so reading input on
/// the interface's thread would keep it from ever seeing the `Quit` message sent on SIGHUP. On its
/// own thread, a spinning read can't prevent the process from exiting.
struct InputReader {
    paused: Arc<AtomicBool>,
    /// Whether the thread has noticed it's paused and stopped reading
    idle: Arc<AtomicBool>,
}

impl InputReader {
    fn spawn(sender: Sender<Message>) -> InputReader {
        let paused = Arc::new(AtomicBool::new(false));
        let idle = Arc::new(AtomicBool::new(false));
        let reader = InputReader {
            paused: paused.clone(),
            idle: idle.clone(),
        };

        std::thread::spawn(move || {
            loop {
                if paused.load(Ordering::SeqCst) {
                    idle.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                idle.store(false, Ordering::SeqCst);

                let message = match event::poll(Duration::from_millis(50)) {
                    Ok(false) => continue,
                    Ok(true) => match event::read() {
                        Ok(event) => Message::Terminal(event),
                        Err(_) => Message::Quit,
                    },
                    Err(_) => Message::Quit,
                };
                let quit = matches!(message, Message::Quit);
                if sender.send(message).is_err() || quit {
                    break;
                }
            }
        });

        reader
    }

    /// Stops reading input, so something else can read from the terminal. Returns once the thread
    /// has stopped.
    fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.idle.store(false, Ordering::SeqCst);
        while !self.idle.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }
}

/// Leaves the interface to let sudo ask for a password, then applies the action if that worked.
fn elevate(terminal: &mut DefaultTerminal, app: &mut App<'_>, request: Elevation) -> Result<()> {
    use ratatui::crossterm::{execute, terminal};

    terminal::disable_raw_mode()?;
    execute!(std::io::stdout(), terminal::LeaveAlternateScreen)?;

    anstream::println!(
        "{} Administrator rights are needed to change:",
        ui::paint(ui::HINT, "::")
    );
    for reason in &request.reasons {
        anstream::println!("     {reason}");
    }
    let authenticated = std::process::Command::new("sudo")
        .arg("-v")
        .status()
        .is_ok_and(|status| status.success());

    execute!(std::io::stdout(), terminal::EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;
    terminal.clear()?;

    if authenticated {
        app.apply_now(&request.indices, request.direction);
    } else {
        app.message(
            Level::Error,
            "Nothing was changed: couldn't get administrator rights with sudo",
        );
    }
    Ok(())
}

/// Whether sudo can run without asking for a password, because the user is root or entered their
/// password recently.
fn sudo_authenticated() -> bool {
    #[cfg(unix)]
    if unsafe { libc::geteuid() } == 0 {
        return true;
    }
    std::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Restores the terminal even if the interface returns early.
struct TerminalGuard(DefaultTerminal);

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// An action waiting for confirmation.
struct Confirmation {
    direction: Direction,
    /// The files to apply it to. Identified by `Change::key`, since the list may be refreshed
    /// while waiting.
    keys: Vec<(Kind, PathBuf, PathBuf)>,
    question: String,
}

/// An action that needs administrator rights before it can be applied.
struct Elevation {
    indices: Vec<usize>,
    direction: Direction,
    /// What needs administrator rights, to explain the password prompt
    reasons: Vec<String>,
}

struct App<'a> {
    opt: &'a Options,
    mode: Mode,
    plan: Option<Plan>,
    /// Why the configuration couldn't be loaded
    load_error: Option<String>,
    changes: Vec<Change>,
    targets: BTreeSet<PathBuf>,
    list: ListState,
    diff_scroll: u16,
    messages: VecDeque<(Level, String)>,
    /// An action waiting for confirmation
    confirm: Option<Confirmation>,
    /// An action waiting for administrator rights, which requires leaving the interface
    elevation: Option<Elevation>,
    quit: bool,
}

impl<'a> App<'a> {
    fn new(opt: &'a Options, mode: Mode) -> App<'a> {
        App {
            opt,
            mode,
            plan: None,
            load_error: None,
            changes: Vec::new(),
            targets: BTreeSet::new(),
            list: ListState::default(),
            diff_scroll: 0,
            messages: VecDeque::new(),
            confirm: None,
            elevation: None,
            quit: false,
        }
    }

    fn selected(&self) -> Option<&Change> {
        self.list.selected().and_then(|i| self.changes.get(i))
    }

    /// Reloads the configuration and compares it against the installed files again.
    fn refresh(&mut self) {
        let selected_key = self.selected().map(Change::key);

        match Plan::load(self.opt) {
            Ok(plan) => {
                self.changes = plan.changes();
                self.targets = plan.targets();
                self.plan = Some(plan);
                self.load_error = None;
            }
            Err(e) => {
                self.changes.clear();
                self.plan = None;
                self.load_error = Some(format!("{e:#}"));
            }
        }

        // Keep the same file selected if it still has changes
        let index = selected_key
            .as_ref()
            .and_then(|key| self.changes.iter().position(|c| c.key() == *key))
            .or_else(|| {
                let previous = self.list.selected().unwrap_or(0);
                (!self.changes.is_empty()).then(|| previous.min(self.changes.len() - 1))
            });
        if index.map(|i| self.changes[i].key()) != selected_key {
            self.diff_scroll = 0;
        }
        self.list.select(index);
    }

    fn message(&mut self, level: Level, text: impl Into<String>) {
        self.messages.push_back((level, text.into()));
        while self.messages.len() > MAX_MESSAGES {
            self.messages.pop_front();
        }
    }

    /// Returns whether there were any new messages.
    fn collect_captured(&mut self) -> bool {
        let captured = ui::take_captured();
        let any = !captured.is_empty();
        for captured in captured {
            self.message(captured.level, captured.message);
        }
        any
    }

    fn on_key(&mut self, key: KeyEvent) {
        if let Some(confirmation) = self.confirm.take() {
            if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                self.apply_confirmed(confirmation);
            }
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.select_relative(-1),
            KeyCode::Down | KeyCode::Char('j') => self.select_relative(1),
            KeyCode::Home => self.select_relative(isize::MIN),
            KeyCode::End => self.select_relative(isize::MAX),
            KeyCode::PageUp => self.diff_scroll = self.diff_scroll.saturating_sub(10),
            KeyCode::PageDown => self.diff_scroll = self.diff_scroll.saturating_add(10),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('d') => self.apply_selected(Direction::Deploy),
            KeyCode::Char('c') => self.apply_selected(Direction::CopyBack),
            KeyCode::Char('D') => self.request_all(Direction::Deploy),
            KeyCode::Char('C') => self.request_all(Direction::CopyBack),
            _ => {}
        }
    }

    fn select_relative(&mut self, offset: isize) {
        if self.changes.is_empty() {
            return;
        }
        let current = self.list.selected().unwrap_or(0) as isize;
        let last = self.changes.len() as isize - 1;
        let index = current.saturating_add(offset).clamp(0, last) as usize;
        if Some(index) != self.list.selected() {
            self.diff_scroll = 0;
        }
        self.list.select(Some(index));
    }

    fn eligible(&self, direction: Direction) -> Vec<usize> {
        (0..self.changes.len())
            .filter(|&i| is_eligible(&self.changes[i], direction))
            .collect()
    }

    fn apply_selected(&mut self, direction: Direction) {
        let Some(change) = self.selected() else {
            return;
        };
        let blocked = match direction {
            Direction::Deploy => (!change.can_deploy()).then(|| "it can't be compared".to_string()),
            Direction::CopyBack => change.copy_back_blocked.clone(),
        };
        if let Some(reason) = blocked {
            let message = format!("Can't {} {}: {reason}", verb(direction), name(change));
            self.message(Level::Warn, message);
            return;
        }

        let source = name(change);
        let target = ui::path(&change.target);
        let question = match (direction, &change.state) {
            (Direction::Deploy, State::NotInstalled) => format!("Install {source} at {target}?"),
            (Direction::Deploy, State::Untracked) => {
                format!("Record {target} as deployed from {source}? Its contents won't change.")
            }
            (Direction::Deploy, State::Removed) => {
                format!("Delete the installed file {target}? It's no longer in the configuration.")
            }
            (Direction::Deploy, _) => {
                format!("Deploy {source}? The installed file {target} will be overwritten.")
            }
            (Direction::CopyBack, _) => format!(
                "Copy {target} into the repository? The repository's {source} will be overwritten."
            ),
        };
        self.confirm = Some(Confirmation {
            direction,
            keys: vec![change.key()],
            question,
        });
    }

    fn request_all(&mut self, direction: Direction) {
        let eligible = self.eligible(direction);
        if eligible.is_empty() {
            let message = match direction {
                Direction::Deploy => "Nothing can be deployed",
                Direction::CopyBack => "No installed files can be copied into the repository",
            };
            self.message(Level::Warn, message);
            return;
        }

        let count = eligible.len();
        let files = if count == 1 { "file" } else { "files" };
        let mut question = match direction {
            Direction::Deploy => {
                format!("Deploy {count} {files}? Installed files will be overwritten.")
            }
            Direction::CopyBack => format!(
                "Copy {count} installed {files} into the repository? Repository files will be overwritten."
            ),
        };
        let skipped = self.changes.len() - count;
        if skipped > 0 {
            question.push_str(&format!(" {skipped} other changes will be skipped."));
        }
        self.confirm = Some(Confirmation {
            direction,
            keys: eligible.iter().map(|&i| self.changes[i].key()).collect(),
            question,
        });
    }

    fn apply_confirmed(&mut self, confirmation: Confirmation) {
        // The list may have been refreshed since the question was asked
        let indices: Vec<usize> = confirmation
            .keys
            .iter()
            .filter_map(|key| self.changes.iter().position(|c| c.key() == *key))
            .filter(|&i| is_eligible(&self.changes[i], confirmation.direction))
            .collect();
        if indices.is_empty() {
            self.message(
                Level::Warn,
                "Nothing to do: the files changed in the meantime",
            );
            return;
        }
        self.apply(&indices, confirmation.direction);
    }

    fn apply(&mut self, indices: &[usize], direction: Direction) {
        if self.opt.dry_run {
            self.message(Level::Warn, "Dry run: nothing was changed");
            return;
        }

        // sudo would ask for a password on top of the interface, so ask for it beforehand
        let mut reasons: Vec<String> = indices
            .iter()
            .map(|&i| &self.changes[i])
            .filter(|change| change.needs_root)
            .map(|change| ui::path(&change.target))
            .collect();
        if crate::changes::hooks_use_sudo(self.opt) {
            reasons.push("the deploy hooks".into());
        }
        if !reasons.is_empty() && !sudo_authenticated() {
            self.elevation = Some(Elevation {
                indices: indices.to_vec(),
                direction,
                reasons,
            });
            return;
        }

        self.apply_now(indices, direction);
    }

    fn apply_now(&mut self, indices: &[usize], direction: Direction) {
        let Some(plan) = self.plan.as_mut() else {
            return;
        };

        let changes: Vec<&Change> = indices.iter().map(|&i| &self.changes[i]).collect();
        let description = match changes.as_slice() {
            [change] => name(change),
            _ => format!("{} files", changes.len()),
        };
        let deletes = match changes.as_slice() {
            [change] if change.state == State::Removed => Some(ui::path(&change.target)),
            _ => None,
        };
        let result = plan.apply(self.opt, &changes, direction);
        let total = changes.len();

        self.collect_captured();
        match result {
            Ok(failures) if failures.is_empty() => {
                let message = match (direction, deletes) {
                    (Direction::Deploy, Some(target)) => format!("Deleted {target}"),
                    (Direction::Deploy, None) => format!("Deployed {description}"),
                    (Direction::CopyBack, _) => {
                        format!("Copied {description} into the repository")
                    }
                };
                self.message(Level::Info, message);
            }
            Ok(failures) => {
                let succeeded = total - failures.len();
                for failure in failures {
                    self.message(Level::Error, failure);
                }
                let message = format!("{} {succeeded} of {total} files", past(direction));
                self.message(Level::Warn, message);
            }
            Err(e) => self.message(Level::Error, format!("{e:#}")),
        }
        self.refresh();
    }

    // === Drawing ===

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let message_height = match self.messages.len() {
            0 => 0,
            n => n.min(VISIBLE_MESSAGES) as u16 + 2,
        };
        let [header, main, messages, help] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(message_height),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.draw_header(frame, header);
        if let Some(error) = &self.load_error {
            let paragraph = Paragraph::new(error.as_str())
                .red()
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" Configuration error "));
            frame.render_widget(paragraph, main);
        } else if self.changes.is_empty() {
            draw_up_to_date(frame, main, self.mode);
        } else {
            let [list, diff] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(main);
            self.draw_list(frame, list);
            self.draw_diff(frame, diff);
        }
        self.draw_messages(frame, messages);
        self.draw_help(frame, help);

        if let Some(confirmation) = &self.confirm {
            draw_confirm(frame, &confirmation.question);
        }
    }

    fn draw_header(&self, frame: &mut Frame<'_>, area: Rect) {
        let status = match (&self.load_error, self.changes.len()) {
            (Some(_), _) => Span::raw("configuration error").red(),
            (None, 0) => Span::raw("up to date").green(),
            (None, 1) => Span::raw("1 change").yellow(),
            (None, n) => Span::raw(format!("{n} changes")).yellow(),
        };
        let title = match self.mode {
            Mode::Watch => " dotter watch ",
            Mode::Deploy => " dotter deploy ",
        };
        let mut spans = vec![Span::raw(title).bold().reversed(), Span::raw("  "), status];
        if self.opt.dry_run {
            spans.push(Span::raw("  dry run: read-only").dim());
        }
        frame.render_widget(Line::from(spans), area);
    }

    fn draw_list(&mut self, frame: &mut Frame<'_>, area: Rect) {
        // Borders, highlight symbol and sign
        let path_width = (area.width as usize).saturating_sub(2 + 2 + 2);
        let items: Vec<ListItem<'_>> = self
            .changes
            .iter()
            .map(|change| {
                let (sign, color) = state_sign(&change.state);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{sign} "), Style::new().fg(color).bold()),
                    Span::raw(truncate_start(
                        &change.source.display().to_string(),
                        path_width,
                    )),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(Block::bordered().title(" Files "))
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.list);
    }

    fn draw_diff(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(change) = self.selected() else {
            return;
        };
        let inner_width = (area.width as usize).saturating_sub(2);
        let title = format!(
            " {} {} → {} ",
            change.kind.name(),
            change.source.display(),
            ui::path(&change.target)
        );
        let title = truncate_start(&title, inner_width);

        let (description, style) = describe(change);
        let mut lines: Vec<Line<'_>> = wrap(&description, inner_width)
            .into_iter()
            .map(|line| Line::styled(line, style))
            .collect();
        if let Some(reason) = &change.copy_back_blocked
            && !matches!(
                change.state,
                State::NotInstalled | State::Removed | State::Untracked | State::Problem(_)
            )
        {
            let text = format!("Can't copy to the repository: {reason}");
            lines.extend(
                wrap(&text, inner_width)
                    .into_iter()
                    .map(|l| Line::from(l).dim()),
            );
        }

        if let Some(diff) = &change.diff {
            lines.push(Line::from(vec![
                Span::raw("- installed").red(),
                Span::raw("  "),
                Span::raw("+ repository").green(),
            ]));
            lines.push(Line::default());

            let rows = diff.rows(self.opt.diff_context_lines);
            if rows.is_empty() {
                lines.push(Line::from("The contents are identical.").dim());
            }
            let width = diff.line_number_width();
            lines.extend(rows.into_iter().map(|row| diff_line(row, width)));
        }

        // Keep the scroll position within the content
        let inner_height = area.height.saturating_sub(2) as usize;
        let max_scroll = lines.len().saturating_sub(inner_height) as u16;
        self.diff_scroll = self.diff_scroll.min(max_scroll);

        let paragraph = Paragraph::new(Text::from(lines))
            .block(Block::bordered().title(title))
            .scroll((self.diff_scroll, 0));
        frame.render_widget(paragraph, area);
    }

    fn draw_messages(&self, frame: &mut Frame<'_>, area: Rect) {
        if area.height == 0 {
            return;
        }
        let lines: Vec<Line<'_>> = self
            .messages
            .iter()
            .skip(self.messages.len().saturating_sub(VISIBLE_MESSAGES))
            .map(|(level, text)| {
                let (label, color) = match level {
                    Level::Error => ("error: ", Color::Red),
                    Level::Warn => ("warning: ", Color::Yellow),
                    _ => ("", Color::Reset),
                };
                Line::from(vec![
                    Span::styled(label, Style::new().fg(color).bold()),
                    Span::raw(text.as_str()),
                ])
            })
            .collect();
        let paragraph = Paragraph::new(lines).block(Block::bordered().title(" Messages "));
        frame.render_widget(paragraph, area);
    }

    fn draw_help(&self, frame: &mut Frame<'_>, area: Rect) {
        let keys: &[(&str, &str)] = if self.changes.is_empty() {
            &[("r", "refresh"), ("q", "quit")]
        } else {
            &[
                ("↑↓", "select"),
                ("d", "deploy"),
                ("c", "copy to repo"),
                ("D", "deploy all"),
                ("C", "copy all to repo"),
                ("PgUp/PgDn", "scroll"),
                ("r", "refresh"),
                ("q", "quit"),
            ]
        };
        let mut spans = vec![Span::raw(" ")];
        for (key, action) in keys {
            spans.push(Span::raw(*key).bold());
            spans.push(Span::raw(format!(" {action}  ")).dim());
        }
        frame.render_widget(Line::from(spans), area);
    }
}

fn draw_confirm(frame: &mut Frame<'_>, question: &str) {
    let [area] = Layout::horizontal([Constraint::Max(64)])
        .flex(Flex::Center)
        .areas(frame.area());
    let inner_width = area.width.saturating_sub(2) as usize;
    let question = wrap(question, inner_width);

    // The wrapped question, a blank line, the keys and the border, within the screen
    let height = (question.len() as u16 + 4).min(frame.area().height);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);

    let block = Block::bordered()
        .title(" Confirm ")
        .border_style(Style::new().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    // The keys get their own row so they're shown even if the question doesn't fit
    let [question_area, keys_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let question: Vec<Line<'_>> = question.into_iter().map(Line::from).collect();
    frame.render_widget(Paragraph::new(question), question_area);
    let keys = Line::from(vec![
        Span::raw("y").bold(),
        Span::raw(" confirm   ").dim(),
        Span::raw("any other key").bold(),
        Span::raw(" cancel").dim(),
    ]);
    frame.render_widget(keys, keys_area);
}

fn is_eligible(change: &Change, direction: Direction) -> bool {
    match direction {
        Direction::Deploy => change.can_deploy(),
        Direction::CopyBack => change.can_copy_back(),
    }
}

fn draw_up_to_date(frame: &mut Frame<'_>, area: Rect, mode: Mode) {
    let detail = match mode {
        Mode::Watch => "Watching the repository and the installed files for changes...",
        Mode::Deploy => "Press q to quit.",
    };
    let text = Text::from(vec![
        Line::from("✓ Everything is up to date").green().bold(),
        Line::from(detail).dim(),
    ])
    .centered();
    let [area] = Layout::vertical([Constraint::Length(2)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(text, area);
}

fn state_sign(state: &State) -> (char, Color) {
    match state {
        State::NotInstalled => ('+', Color::Green),
        State::Modified | State::ReplacedByFile | State::WrongLink(_) => ('~', Color::Yellow),
        State::Removed => ('-', Color::Red),
        State::Untracked => ('=', Color::Cyan),
        State::Problem(_) => ('!', Color::Red),
    }
}

fn describe(change: &Change) -> (String, Style) {
    let text = match &change.state {
        State::NotInstalled => "Not installed yet. Deploying installs it.".to_string(),
        State::Modified => {
            "The installed file differs from what the repository produces.".to_string()
        }
        State::ReplacedByFile if change.kind == Kind::Symlink => {
            "The installed symlink was replaced by a regular file.".to_string()
        }
        State::ReplacedByFile => "The installed file was replaced.".to_string(),
        State::WrongLink(destination) => format!(
            "The installed symlink points to {} instead of the repository.",
            ui::path(destination)
        ),
        State::Removed => {
            "Removed from the configuration. Deploying deletes the installed file.".to_string()
        }
        State::Untracked => "The installed file already matches the repository, but dotter \
            hasn't recorded it as deployed. Deploying records it."
            .to_string(),
        State::Problem(problem) => {
            return (format!("Problem: {problem}"), Style::new().fg(Color::Red));
        }
    };
    (text, Style::new().add_modifier(Modifier::ITALIC))
}

/// Shortens `text` to `width` characters by replacing its start with an ellipsis, which keeps
/// the end of a path, usually the most specific part, visible.
fn truncate_start(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let tail: String = text.chars().skip(length - keep).collect();
    format!("…{tail}")
}

/// Wraps `text` at word boundaries into lines of at most `width` characters, breaking words that
/// are longer than a line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        let line_length = line.chars().count();
        if line_length > 0 && line_length + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        while word.len() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(word.drain(..width).collect());
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.extend(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn diff_line(row: DiffRow, width: usize) -> Line<'static> {
    let DiffRow::Line {
        old_line,
        new_line,
        tag,
        segments,
        missing_newline,
    } = row
    else {
        return Line::from(format!("{:w$} {:w$} ┊", "", "", w = width)).dim();
    };

    let (sign, style) = match tag {
        ChangeTag::Delete => ('-', Style::new().fg(Color::Red)),
        ChangeTag::Insert => ('+', Style::new().fg(Color::Green)),
        ChangeTag::Equal => (' ', Style::new()),
    };
    let number = |line: Option<usize>| line.map_or(String::new(), |l| l.to_string());

    let mut spans = vec![
        Span::raw(format!(
            "{:>w$} {:>w$} │ ",
            number(old_line),
            number(new_line),
            w = width
        ))
        .dim(),
        Span::styled(format!("{sign} "), style),
    ];
    for (emphasized, text) in segments {
        let style = if emphasized {
            style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            style
        };
        spans.push(Span::styled(text.replace('\t', "    "), style));
    }
    if missing_newline {
        spans.push(Span::raw(" (no newline at end of file)").dim());
    }
    Line::from(spans)
}

fn name(change: &Change) -> String {
    change.source.display().to_string()
}

fn verb(direction: Direction) -> &'static str {
    match direction {
        Direction::Deploy => "deploy",
        Direction::CopyBack => "copy",
    }
}

fn past(direction: Direction) -> &'static str {
    match direction {
        Direction::Deploy => "Deployed",
        Direction::CopyBack => "Copied",
    }
}

#[cfg(test)]
mod test {
    use super::{truncate_start, wrap};

    #[test]
    fn truncates_from_the_start() {
        assert_eq!(truncate_start("short", 10), "short");
        assert_eq!(truncate_start("hosts/gmktec/etc/file", 8), "…tc/file");
    }

    #[test]
    fn wraps_at_words() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), [""]);
    }
}
