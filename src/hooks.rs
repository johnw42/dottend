use anyhow::{Context, Result};
use handlebars::Handlebars;

use std::path::Path;
use std::process::{Command, Stdio};

use crate::filesystem::{Filesystem, RealFilesystem};

pub(crate) fn run_hook(
    location: &Path,
    cache_dir: &Path,
    handlebars: &Handlebars<'_>,
    variables: &crate::config::Variables,
) -> Result<()> {
    let bat_sibling = if cfg!(windows) && location.extension().is_some_and(|e| e == "sh") {
        let candidate = location.with_extension("bat");
        if candidate.exists() {
            debug!("Using hook file {:?} on windows", candidate);
            Some(candidate)
        } else {
            None
        }
    } else {
        None
    };
    let location: &Path = bat_sibling.as_deref().unwrap_or(location);

    if !location.exists() {
        debug!("Hook file at {:?} missing", location);
        return Ok(());
    }

    let mut script_file = cache_dir.join(location);
    if cfg!(windows) {
        script_file.set_extension("bat");
    }

    debug!("Rendering script {:?} -> {:?}", location, script_file);
    let mut fs = RealFilesystem::new(false);
    crate::actions::perform_template_deploy(
        location,
        &script_file,
        None,
        &mut fs,
        handlebars,
        variables,
    )
    .context("deploy script")?;
    fs.copy_permissions(location, &script_file, &None)
        .context("copy permissions from source to cache")?;

    debug!("Running script file");
    let capture = crate::ui::capturing();
    let (mut command, spawn_context) = script_command(&script_file)?;
    if capture {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }
    let child = command.spawn().context(spawn_context)?;

    let output = child.wait_with_output().context("wait for child shell")?;
    if capture {
        let script = location.display();
        for (level, stream) in [
            (log::Level::Info, &output.stdout),
            (log::Level::Warn, &output.stderr),
        ] {
            for line in String::from_utf8_lossy(stream).lines() {
                crate::ui::capture(level, format!("{script}: {line}"));
            }
        }
    }

    anyhow::ensure!(output.status.success(), "subshell returned error");

    Ok(())
}

#[cfg(unix)]
fn script_command(script: &Path) -> Result<(Command, &'static str)> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = script.metadata()?.permissions();
    if !script.is_dir() && permissions.mode() & 0o111 != 0 {
        Ok((Command::new(script), "spawn script file"))
    } else {
        let mut command = Command::new("sh");
        command.arg(script);
        Ok((command, "spawn shell"))
    }
}

#[cfg(windows)]
fn script_command(script: &Path) -> Result<(Command, &'static str)> {
    Ok((Command::new(script), "spawn batch file"))
}
