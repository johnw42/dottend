//! Detects differences between the repository and the installed files, and resolves them in
//! either direction: deploying the repository's version, or copying the installed file back into
//! the repository. Used by the interactive `watch` interface.

use anyhow::{Context, Result};
use handlebars::Handlebars;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::actions;
use crate::args::Options;
use crate::config::{self, Cache, SymbolicTarget, TemplateTarget, Variables};
use crate::deploy::desired_files;
use crate::difference::Diff;
use crate::filesystem::{self, Filesystem, RealFilesystem, SymlinkComparison};
use crate::handlebars_helpers::create_new_handlebars;
use crate::hooks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Symlink,
    Template,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Symlink => "symlink",
            Kind::Template => "template",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The file isn't installed yet
    NotInstalled,
    /// The installed file's contents differ from what the repository produces
    Modified,
    /// The installed symlink was replaced by a regular file or directory
    ReplacedByFile,
    /// The installed symlink points somewhere else
    WrongLink(PathBuf),
    /// The file was removed from the configuration but is still deployed
    Removed,
    /// The installed file already matches the repository, but isn't recorded as deployed
    Untracked,
    /// Something prevents comparing or deploying the file
    Problem(String),
}

/// A difference between a file in the repository and its installed counterpart.
pub struct Change {
    pub kind: Kind,
    pub source: PathBuf,
    pub target: PathBuf,
    pub state: State,
    /// The installed contents (old) against the contents the repository produces (new), when both
    /// are available as text
    pub diff: Option<Diff>,
    /// Why the installed file can't be copied into the repository, if it can't
    pub copy_back_blocked: Option<String>,
    /// Whether resolving the change requires administrator rights
    pub needs_root: bool,
}

impl Change {
    /// Identifies the file across refreshes.
    pub fn key(&self) -> (Kind, PathBuf, PathBuf) {
        (self.kind, self.source.clone(), self.target.clone())
    }

    pub fn can_deploy(&self) -> bool {
        !matches!(self.state, State::Problem(_))
    }

    pub fn can_copy_back(&self) -> bool {
        self.copy_back_blocked.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Install the repository's version of the file
    Deploy,
    /// Copy the installed file into the repository
    CopyBack,
}

/// The loaded configuration and cache, ready to compare against the installed files.
pub struct Plan {
    handlebars: Handlebars<'static>,
    variables: Variables,
    symlinks: BTreeMap<PathBuf, SymbolicTarget>,
    templates: BTreeMap<PathBuf, TemplateTarget>,
    cache: Cache,
}

impl Plan {
    pub fn load(opt: &Options) -> Result<Plan> {
        let mut config = config::load_configuration(&opt.local_config, &opt.global_config, None)
            .context("get a configuration")?;
        let cache = filesystem::load_file(&opt.cache_file)?.unwrap_or_default();
        let handlebars = create_new_handlebars(&mut config).context("initialize handlebars")?;
        let (symlinks, templates) = desired_files(std::mem::take(&mut config.files))?;

        Ok(Plan {
            handlebars,
            variables: config.variables,
            symlinks,
            templates,
            cache,
        })
    }

    /// The installed locations of every file that is or was deployed.
    pub fn targets(&self) -> BTreeSet<PathBuf> {
        self.symlinks
            .values()
            .map(|t| t.target.clone())
            .chain(self.templates.values().map(|t| t.target.clone()))
            .chain(self.cache.symlinks.values().cloned())
            .chain(self.cache.templates.values().cloned())
            .collect()
    }

    /// Every file whose installed state differs from what the repository produces.
    pub fn changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();

        for (source, target) in &self.cache.symlinks {
            if self.symlinks.get(source).map(|t| &t.target) != Some(target) {
                changes.push(removed(Kind::Symlink, source, target));
            }
        }
        for (source, target) in &self.cache.templates {
            if self.templates.get(source).map(|t| &t.target) != Some(target) {
                changes.push(removed(Kind::Template, source, target));
            }
        }
        changes.extend(self.symlinks.iter().filter_map(|(source, target)| {
            let tracked = self.cache.symlinks.get(source) == Some(&target.target);
            symlink_change(source, target, tracked)
        }));
        changes.extend(
            self.templates
                .iter()
                .filter_map(|(source, target)| self.template_change(source, target)),
        );

        changes.sort_by(|a, b| a.source.cmp(&b.source).then(a.target.cmp(&b.target)));
        changes
    }

    fn render(&self, source: &Path, target: &TemplateTarget) -> Result<String> {
        let contents = fs::read_to_string(source).context("read template source file")?;
        self.handlebars
            .render_template(&target.apply_actions(contents), &self.variables)
            .context("render template")
    }

    fn template_change(&self, source: &Path, target: &TemplateTarget) -> Option<Change> {
        let change = |state, diff, copy_back_blocked: Option<&str>| {
            Some(Change {
                kind: Kind::Template,
                source: source.to_owned(),
                target: target.target.clone(),
                state,
                diff,
                copy_back_blocked: copy_back_blocked.map(str::to_owned),
                needs_root: needs_root(&target.target, target.owner.is_some()),
            })
        };

        let rendered = match self.render(source, target) {
            Ok(rendered) => rendered,
            Err(e) => return change(State::Problem(format!("{e:#}")), None, Some(PROBLEM)),
        };

        match fs::symlink_metadata(&target.target) {
            Err(e) if e.kind() == ErrorKind::NotFound => change(
                State::NotInstalled,
                Some(Diff::new(String::new(), rendered)),
                Some(NOT_INSTALLED),
            ),
            Err(e) => change(State::Problem(e.to_string()), None, Some(PROBLEM)),
            Ok(metadata) if !metadata.is_file() => change(
                State::Problem("the installed file isn't a regular file".into()),
                None,
                Some(PROBLEM),
            ),
            Ok(_) => match fs::read_to_string(&target.target) {
                Err(e) => change(State::Problem(e.to_string()), None, Some(PROBLEM)),
                Ok(installed) if installed == rendered => {
                    if self.cache.templates.get(source) == Some(&target.target) {
                        None
                    } else {
                        change(State::Untracked, None, Some(UNTRACKED))
                    }
                }
                Ok(installed) => {
                    let blocked = template_copy_back_blocked(source, target);
                    change(
                        State::Modified,
                        Some(Diff::new(installed, rendered)),
                        blocked.as_deref(),
                    )
                }
            },
        }
    }

    /// Resolves the given changes in the given direction, running the deploy hooks around them and
    /// saving the cache afterwards. Returns a message for each change that couldn't be applied.
    pub fn apply(
        &mut self,
        opt: &Options,
        changes: &[&Change],
        direction: Direction,
    ) -> Result<Vec<String>> {
        hooks::run_hook(
            &opt.pre_deploy,
            &opt.cache_directory,
            &self.handlebars,
            &self.variables,
        )
        .context("run pre-deploy hook")?;

        // Removals go first, in case a file's new location reuses the old one's cache file
        let mut ordered = changes.to_vec();
        ordered.sort_by_key(|c| c.state != State::Removed);

        let mut fs = RealFilesystem::new(opt.noconfirm);
        let mut failures = Vec::new();
        for change in ordered {
            let result = match direction {
                Direction::Deploy => self.deploy(change, &mut fs, opt),
                Direction::CopyBack => self.copy_back(change, &mut fs, opt),
            };
            match result {
                Ok(true) => {}
                Ok(false) => failures.push(format!("{}: skipped", change.source.display())),
                Err(e) => failures.push(format!("{}: {e:#}", change.source.display())),
            }
        }

        filesystem::save_file(&opt.cache_file, self.cache.clone()).context("save cache")?;

        hooks::run_hook(
            &opt.post_deploy,
            &opt.cache_directory,
            &self.handlebars,
            &self.variables,
        )
        .context("run post-deploy hook")?;

        Ok(failures)
    }

    /// Returns false if the change was skipped.
    fn deploy(&mut self, change: &Change, fs: &mut dyn Filesystem, opt: &Options) -> Result<bool> {
        anyhow::ensure!(change.can_deploy(), "can't deploy: {:?}", change.state);
        let source = &change.source;
        let cache_file = opt.cache_directory.join(source);

        match (change.kind, &change.state) {
            (Kind::Symlink, State::Removed) => {
                let deleted = actions::delete_symlink(source, &change.target, fs, true)?;
                if deleted && self.cache.symlinks.get(source) == Some(&change.target) {
                    self.cache.symlinks.remove(source);
                }
                Ok(deleted)
            }
            (Kind::Template, State::Removed) => {
                let deleted =
                    actions::delete_template(source, &cache_file, &change.target, fs, true)?;
                if deleted && self.cache.templates.get(source) == Some(&change.target) {
                    self.cache.templates.remove(source);
                }
                Ok(deleted)
            }
            (Kind::Symlink, _) => {
                let target = self
                    .symlinks
                    .get(source)
                    .context("file is no longer in the configuration")?
                    .clone();
                let deployed = if self.cache.symlinks.get(source) == Some(&target.target) {
                    actions::update_symlink(source, &target, fs, true)?
                } else {
                    actions::create_symlink(source, &target, fs, true)?
                };
                if deployed {
                    self.cache.symlinks.insert(source.clone(), target.target);
                }
                Ok(deployed)
            }
            (Kind::Template, _) => {
                let target = self
                    .templates
                    .get(source)
                    .context("file is no longer in the configuration")?
                    .clone();
                let deployed = if self.cache.templates.get(source) == Some(&target.target) {
                    actions::update_template(
                        source,
                        &cache_file,
                        &target,
                        fs,
                        &self.handlebars,
                        &self.variables,
                        true,
                        opt.diff_context_lines,
                    )?
                } else {
                    actions::create_template(
                        source,
                        &cache_file,
                        &target,
                        fs,
                        &self.handlebars,
                        &self.variables,
                        true,
                    )?
                };
                if deployed {
                    self.cache.templates.insert(source.clone(), target.target);
                }
                Ok(deployed)
            }
        }
    }

    /// Returns false if the change was skipped.
    fn copy_back(
        &mut self,
        change: &Change,
        fs: &mut dyn Filesystem,
        opt: &Options,
    ) -> Result<bool> {
        // The source may have changed since the change was detected, so check again
        let blocked = match (change.kind, self.templates.get(&change.source)) {
            (Kind::Template, Some(target)) => template_copy_back_blocked(&change.source, target),
            _ => change.copy_back_blocked.clone(),
        };
        if let Some(reason) = blocked {
            anyhow::bail!("can't copy into the repository: {reason}");
        }

        fs::copy(&change.target, &change.source)
            .context("copy installed file into the repository")?;

        // Turn the installed file back into a symlink, or refresh the template's cache
        self.deploy(change, fs, opt)
    }
}

/// Whether the deploy hooks call `sudo` themselves. Only the hook files are checked, not the
/// scripts they run.
pub fn hooks_use_sudo(opt: &Options) -> bool {
    [&opt.pre_deploy, &opt.post_deploy]
        .iter()
        .any(|hook| fs::read_to_string(hook).is_ok_and(|script| script.contains("sudo")))
}

const PROBLEM: &str = "the file can't be compared";
const NOT_INSTALLED: &str = "nothing is installed yet";
const UNTRACKED: &str = "the installed file already matches the repository";

fn removed(kind: Kind, source: &Path, target: &Path) -> Change {
    Change {
        kind,
        source: source.to_owned(),
        target: target.to_owned(),
        state: State::Removed,
        diff: None,
        copy_back_blocked: Some("the file was removed from the configuration".into()),
        needs_root: needs_root(target, false),
    }
}

/// Whether changing the file at `target` requires administrator rights: it has an owner set, or
/// the current user can't write to it or to the directory it's in.
#[cfg(unix)]
fn needs_root(target: &Path, has_owner: bool) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let writable = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::W_OK) } == 0)
    };

    if has_owner {
        return true;
    }
    if target.symlink_metadata().is_ok() && !target.is_symlink() && !writable(target) {
        return true;
    }
    // Creating or removing the file needs write access to the nearest existing directory
    target
        .ancestors()
        .skip(1)
        .find(|dir| dir.is_dir())
        .is_some_and(|dir| !writable(dir))
}

#[cfg(not(unix))]
fn needs_root(_target: &Path, _has_owner: bool) -> bool {
    false
}

fn symlink_change(source: &Path, target: &SymbolicTarget, tracked: bool) -> Option<Change> {
    let change = |state, diff, copy_back_blocked: Option<&str>| {
        Some(Change {
            kind: Kind::Symlink,
            source: source.to_owned(),
            target: target.target.clone(),
            state,
            diff,
            copy_back_blocked: copy_back_blocked.map(str::to_owned),
            needs_root: needs_root(&target.target, target.owner.is_some()),
        })
    };
    let read = |path: &Path| fs::read_to_string(path).ok();

    let comparison = RealFilesystem::new(true).compare_symlink(source, &target.target);
    match comparison {
        Err(e) => change(State::Problem(format!("{e:#}")), None, Some(PROBLEM)),
        Ok(SymlinkComparison::Identical) if tracked => None,
        Ok(SymlinkComparison::Identical) => change(State::Untracked, None, Some(UNTRACKED)),
        Ok(SymlinkComparison::OnlySourceExists) => change(
            State::NotInstalled,
            read(source).map(|contents| Diff::new(String::new(), contents)),
            Some(NOT_INSTALLED),
        ),
        Ok(SymlinkComparison::OnlyTargetExists | SymlinkComparison::BothMissing) => change(
            State::Problem("the source file is missing".into()),
            None,
            Some(PROBLEM),
        ),
        Ok(SymlinkComparison::Changed) => change(
            State::WrongLink(fs::read_link(&target.target).unwrap_or_default()),
            None,
            Some("the installed file is a symlink to somewhere else"),
        ),
        Ok(SymlinkComparison::TargetNotSymlink) => {
            if !target.target.is_file() || !source.is_file() {
                return change(
                    State::ReplacedByFile,
                    None,
                    Some("only regular files can be copied into the repository"),
                );
            }
            let diff = read(&target.target)
                .zip(read(source))
                .map(|(installed, repo)| Diff::new(installed, repo));
            change(State::ReplacedByFile, diff, None)
        }
    }
}

/// Copying a template's installed file into the repository would replace the template source
/// with its rendered output, so it's only allowed when rendering doesn't change anything.
fn template_copy_back_blocked(source: &Path, target: &TemplateTarget) -> Option<String> {
    if target.append.is_some() || target.prepend.is_some() {
        return Some("the file has `append` or `prepend` set; edit the source instead".into());
    }
    match fs::read_to_string(source) {
        Ok(contents) if contents.contains("{{") => {
            Some("the source is a template; edit it in the repository instead".into())
        }
        Ok(_) => None,
        Err(_) => Some("the source isn't readable text".into()),
    }
}
