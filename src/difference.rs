use anyhow::{Context, Result};
use crossterm::style::Stylize;
use handlebars::Handlebars;
use similar::{ChangeTag, TextDiff};

use std::cmp::{max, min};
use std::fs;
use std::path::Path;

use crate::config::{TemplateTarget, Variables};
use crate::ui;

/// The difference between two versions of a file's contents.
pub struct Diff {
    old: String,
    new: String,
}

type LineDiff<'a> = Vec<diff::Result<&'a str>>;
type HunkDiff<'a> = Vec<(usize, usize, LineDiff<'a>)>;

impl Diff {
    pub fn new(old: String, new: String) -> Diff {
        Diff { old, new }
    }

    /// Whether there are any differences between the two versions.
    pub fn has_changes(&self) -> bool {
        diff::lines(&self.old, &self.new).iter().any(is_different)
    }

    /// Prints the changed hunks, each surrounded by `context_lines` lines of unchanged context.
    pub fn print(&self, context_lines: usize) {
        if ui::fancy() {
            self.print_fancy(context_lines);
        } else {
            print_diff(&diff::lines(&self.old, &self.new), context_lines);
        }
    }

    /// The diff as structured rows: the changed hunks, each surrounded by `context_lines` lines of
    /// unchanged context, with separators between hunks.
    pub fn rows(&self, context_lines: usize) -> Vec<DiffRow> {
        let diff = TextDiff::from_lines(&self.old, &self.new);
        let mut rows = Vec::new();

        for (i, group) in diff.grouped_ops(context_lines).iter().enumerate() {
            if i > 0 {
                rows.push(DiffRow::Separator);
            }

            for op in group {
                for change in diff.iter_inline_changes(op) {
                    let segments = change
                        .iter_strings_lossy()
                        .map(|(emphasized, value)| {
                            (emphasized, value.trim_end_matches(['\n', '\r']).to_string())
                        })
                        .collect();
                    rows.push(DiffRow::Line {
                        old_line: change.old_index().map(|i| i + 1),
                        new_line: change.new_index().map(|i| i + 1),
                        tag: change.tag(),
                        segments,
                        missing_newline: change.missing_newline(),
                    });
                }
            }
        }

        rows
    }

    /// Width needed to display the largest line number in this diff.
    pub fn line_number_width(&self) -> usize {
        max(self.old.lines().count(), self.new.lines().count())
            .to_string()
            .len()
    }

    fn print_fancy(&self, context_lines: usize) {
        let width = self.line_number_width();

        for row in self.rows(context_lines) {
            let DiffRow::Line {
                old_line,
                new_line,
                tag,
                segments,
                missing_newline,
            } = row
            else {
                anstream::println!(
                    "    {}",
                    ui::paint(ui::DIM, format!("{:w$} {:w$} ┊", "", "", w = width))
                );
                continue;
            };

            let (sign, style) = match tag {
                ChangeTag::Delete => ('-', ui::REMOVED),
                ChangeTag::Insert => ('+', ui::ADDED),
                ChangeTag::Equal => (' ', anstyle::Style::new()),
            };
            let line_number = |line: Option<usize>| line.map_or(String::new(), |l| l.to_string());

            let mut line = String::new();
            for (emphasized, value) in segments {
                if emphasized {
                    line.push_str(&ui::paint(style.bold().underline(), value));
                } else {
                    line.push_str(&ui::paint(style, value));
                }
            }

            if missing_newline {
                line.push_str(&ui::paint(ui::DIM, " (no newline at end of file)"));
            }

            anstream::println!(
                "    {} {}{}",
                ui::paint(
                    ui::DIM,
                    format!(
                        "{:>w$} {:>w$} │",
                        line_number(old_line),
                        line_number(new_line),
                        w = width
                    )
                ),
                ui::paint(style, format!("{sign} ")),
                line
            );
        }
    }
}

/// One row of a rendered diff.
pub enum DiffRow {
    /// Marks skipped unchanged lines between two hunks
    Separator,
    Line {
        old_line: Option<usize>,
        new_line: Option<usize>,
        tag: ChangeTag,
        /// The line's text, split into `(emphasized, text)` segments for inline highlighting
        segments: Vec<(bool, String)>,
        missing_newline: bool,
    },
}

pub fn print_template_diff(
    source: &Path,
    target: &TemplateTarget,
    handlebars: &Handlebars<'_>,
    variables: &Variables,
    diff_context_lines: usize,
) {
    if log_enabled!(log::Level::Info) {
        match generate_template_diff(source, target, handlebars, variables, true) {
            Ok(diff) => {
                if diff.has_changes() {
                    info!(
                        "{}",
                        ui::action(ui::Op::Change, "template", source, &target.target)
                    );
                    diff.print(diff_context_lines);
                }
            }
            Err(e) => {
                warn!(
                    "Failed to generate diff for template {:?} -> {:?} on step: {}",
                    source, target.target, e
                );
            }
        }
    }
}

pub fn generate_template_diff(
    source: &Path,
    target: &TemplateTarget,
    handlebars: &Handlebars<'_>,
    variables: &Variables,
    source_to_target: bool,
) -> Result<Diff> {
    let file_contents = fs::read_to_string(source).context("read template source file")?;
    let file_contents = target.apply_actions(file_contents);
    let rendered = handlebars
        .render_template(&file_contents, variables)
        .context("render template")?;

    let target_contents =
        fs::read_to_string(&target.target).context("read template target file")?;

    Ok(if source_to_target {
        Diff::new(target_contents, rendered)
    } else {
        Diff::new(rendered, target_contents)
    })
}

// === Classic renderer, used when stdout is not a terminal ===

fn hunkify_diff<'a>(diff: &[diff::Result<&'a str>], extra_lines: usize) -> HunkDiff<'a> {
    let mut hunks = vec![];

    let mut left_line_number: usize = 1;
    let mut right_line_number: usize = 1;

    let mut current_hunk = None;

    for (position, line) in diff.iter().enumerate() {
        match line {
            diff::Result::Left(_) | diff::Result::Right(_) => {
                // The central part of a hunk
                if current_hunk.is_none() {
                    current_hunk = Some((left_line_number, right_line_number, vec![]));
                }
                current_hunk.as_mut().unwrap().2.push(line.clone());
            }
            diff::Result::Both(_, _) => {
                if diff[position..=min(position + extra_lines, diff.len() - 1)]
                    .iter()
                    .any(is_different)
                {
                    // There's a hunk soon - but we might already be in a hunk
                    if current_hunk.is_none() {
                        current_hunk = Some((left_line_number, right_line_number, vec![]));
                    }
                    current_hunk.as_mut().unwrap().2.push(line.clone());
                } else if diff[position.saturating_sub(extra_lines)..position]
                    .iter()
                    .any(is_different)
                {
                    // We're just after a hunk
                    current_hunk.as_mut().unwrap().2.push(line.clone());
                } else if let Some(hunk) = current_hunk.take() {
                    // We're finished with the current hunk
                    hunks.push(hunk);
                }
            }
        }

        // Keep track of line numbers
        match line {
            diff::Result::Left(_) => {
                left_line_number += 1;
            }
            diff::Result::Right(_) => {
                right_line_number += 1;
            }
            diff::Result::Both(_, _) => {
                left_line_number += 1;
                right_line_number += 1;
            }
        }
    }

    // Last hunk - in case the last line is included in a hunk, it was never added
    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    hunks
}

fn is_different(diff: &diff::Result<&str>) -> bool {
    !matches!(diff, diff::Result::Both(..))
}

fn print_hunk(mut left_line: usize, mut right_line: usize, hunk: LineDiff<'_>, max_digits: usize) {
    for line in hunk {
        match line {
            diff::Result::Left(l) => {
                println!(
                    " {:>width$} | {:>width$} | {}",
                    left_line.to_string().red(),
                    "",
                    l.red(),
                    width = max_digits
                );
                left_line += 1;
            }
            diff::Result::Both(l, _) => {
                println!(
                    " {:>width$} | {:>width$} | {}",
                    left_line.to_string().dark_grey(),
                    right_line.to_string().dark_grey(),
                    l,
                    width = max_digits
                );
                left_line += 1;
                right_line += 1;
            }
            diff::Result::Right(r) => {
                println!(
                    " {:>width$} | {:>width$} | {}",
                    "",
                    right_line.to_string().green(),
                    r.green(),
                    width = max_digits
                );
                right_line += 1;
            }
        }
    }
}

fn print_diff(diff: &[diff::Result<&str>], extra_lines: usize) {
    let mut diff = hunkify_diff(diff, extra_lines);

    let last_hunk = diff.pop().expect("at least one hunk");
    let max_possible_line = max(last_hunk.0, last_hunk.1) + last_hunk.2.len();
    let max_possible_digits = max_possible_line.to_string().len(); // yes I could log10, whatever

    for hunk in diff {
        print_hunk(hunk.0, hunk.1, hunk.2, max_possible_digits);
        println!();
    }

    print_hunk(last_hunk.0, last_hunk.1, last_hunk.2, max_possible_digits);
}

#[cfg(test)]
mod test {
    use super::Diff;

    fn changed(old: &str, new: &str) -> bool {
        Diff::new(old.into(), new.into()).has_changes()
    }

    #[test]
    fn identical_is_not_a_change() {
        assert!(!changed("a\nb\n", "a\nb\n"));
        assert!(!changed("a\r\nb\r\n", "a\nb\n"));
        assert!(!changed("", ""));
    }

    #[test]
    fn content_change_is_a_change() {
        assert!(changed("a\nb\n", "a\nc\n"));
        assert!(changed("", "a\n"));
        assert!(changed("a\nb", "a\nb\n"));
    }
}
