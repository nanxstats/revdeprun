//! Core library for the `revdeprun` CLI.
//!
//! The library exposes a single [`run`] function that orchestrates the end-to-end
//! workflow for provisioning R, preparing the target package repository, and
//! executing `xfun::rev_check()`.

use anyhow::{Context, Result, bail};
use clap::Parser;
use progress::Progress;
use xshell::Shell;

pub mod cli;
mod progress;
mod r_install;
mod r_version;
mod revdep;
mod sysreqs;
pub mod util;
mod workspace;

/// Executes the CLI workflow using the command-line arguments from [`std::env::args`].
///
/// # Errors
///
/// Returns an error whenever preparing the workspace, installing R, cloning the
/// repository, or launching `xfun::rev_check()` fails.
pub fn run() -> Result<()> {
    let args = cli::Args::parse();

    if std::env::consts::OS != "linux" {
        bail!("revdeprun currently supports Ubuntu Linux environments only.");
    }

    let progress = Progress::new();
    let shell = Shell::new().context("failed to initialize shell environment")?;

    let workspace_label = args
        .work_dir
        .as_ref()
        .map(|path| format!("Preparing workspace {}", path.display()))
        .unwrap_or_else(|| "Preparing workspace directory".to_string());
    let workspace = {
        let task = progress.task(workspace_label.clone());
        match workspace::prepare(args.work_dir.clone()).context("failed to prepare workspace") {
            Ok(workspace) => {
                task.finish_with_message(format!(
                    "Workspace ready (clone root: {})",
                    workspace.clone_root().display()
                ));
                workspace
            }
            Err(err) => {
                task.fail(format!("{workspace_label} (failed)"));
                return Err(err);
            }
        }
    };

    let version_label = format!("Resolving R version '{}'", args.r_version);
    let resolution_task = (!args.skip_r_install).then(|| progress.task(version_label.clone()));
    let resolved_version = match resolve_r_version_if_installing(
        &args.r_version,
        args.skip_r_install,
        r_version::resolve,
    )
    .context("failed to resolve requested R version")
    {
        Ok(Some(version)) => {
            if let Some(task) = resolution_task {
                task.finish_with_message(format!("Resolved R {}", version.version));
            }
            Some(version)
        }
        Ok(None) => {
            progress.println("Skipping R version resolution and installation as requested.");
            None
        }
        Err(err) => {
            if let Some(task) = resolution_task {
                task.fail(format!("{version_label} (failed)"));
            }
            return Err(err);
        }
    };

    if let Some(version) = resolved_version.as_ref() {
        r_install::install_r(&shell, version, &progress)
            .context("failed to install the requested R toolchain")?;
    }

    let repository_path =
        revdep::prepare_repository(&shell, &workspace, &args.repository, &progress)
            .context("failed to prepare target repository")?;

    let num_workers = args
        .num_workers
        .map(|value| value.get())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|cpus| cpus.get())
                .unwrap_or(1)
        });

    sysreqs::install_reverse_dep_sysreqs(
        &shell,
        &workspace,
        &repository_path,
        num_workers,
        &progress,
    )
    .context("failed to install system requirements for reverse dependencies")?;

    revdep::run_revcheck(&shell, &workspace, &repository_path, num_workers, &progress)
        .context("reverse dependency check invocation failed")?;

    let r_version_summary = resolved_version
        .as_ref()
        .map(|version| version.version.as_str())
        .unwrap_or("system installation (version resolution skipped)");
    progress.println(format!(
        "Reverse dependency check finished successfully.\n  • R version: {}\n  • repository: {}\n  • library: {}",
        r_version_summary,
        repository_path.display(),
        revdep::revlib_dir(&repository_path).display()
    ));

    Ok(())
}

fn resolve_r_version_if_installing<F>(
    spec: &str,
    skip_r_install: bool,
    resolver: F,
) -> Result<Option<r_version::ResolvedRVersion>>
where
    F: FnOnce(&str) -> Result<r_version::ResolvedRVersion>,
{
    if skip_r_install {
        return Ok(None);
    }

    resolver(spec).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipping_r_install_does_not_resolve_a_version() {
        let resolved = resolve_r_version_if_installing("release", true, |_| {
            panic!("the resolver must not be called when R installation is skipped")
        })
        .unwrap();

        assert!(resolved.is_none());
    }
}
