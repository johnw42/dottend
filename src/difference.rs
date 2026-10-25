use anyhow::{Context, Result};
use handlebars::Handlebars;
use similar::{ChangeTag, TextDiff};

use std::cmp::max;
use std::fs;
use std::path::Path;

use crate::config::{TemplateTarget, Variables};
use crate::ui;

/// The difference between two versions of a file's contents.
pub struct Diff {
    old: String,
    new: String,
}

impl Diff {
    pub fn new(old: String, new: String) -> Diff {
        Diff { old, new }
    }

    /// Whether there are any differences between the two versions. Line endings (`\n` or
    /// `\r\n`) don't count, but a missing newline at the end of the file does.
    pub fn has_changes(&self) -> bool {
        self.old.lines().ne(self.new.lines())
            || self.old.ends_with('\n') != self.new.ends_with('\n')
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

    /// Prints the changed hunks, each surrounded by `context_lines` lines of unchanged context.
    pub fn print(&self, context_lines: usize) {
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
