//! Core library for the `revdeprun` CLI.
//!
//! The library exposes a single [`run`] function that orchestrates the end-to-end
//! workflow for provisioning R, preparing the target package repository, and
//! executing `xfun::rev_check()`, as well as the `bundle` subcommand that packs
//! the check results for transfer.

use anyhow::{Context, Result, bail};
use clap::Parser;
use progress::Progress;
use xshell::Shell;

mod bundle;
pub mod cli;
mod progress;
mod r_install;
mod r_scripts;
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
/// repository, launching `xfun::rev_check()`, or bundling results fails.
pub fn run() -> Result<()> {
    let args = cli::Args::parse();
    let progress = Progress::new();

    match &args.command {
        Some(cli::Command::Bundle(bundle_args)) => bundle::run(bundle_args, &progress),
        None => run_check(&args, &progress),
    }
}

/// Runs the end-to-end reverse dependency check described by `args`.
fn run_check(args: &cli::Args, progress: &Progress) -> Result<()> {
    let Some(repository) = args.repository.as_deref() else {
        bail!("a repository argument is required; see `revdeprun --help`");
    };

    if std::env::consts::OS != "linux" {
        bail!("revdeprun currently supports Ubuntu Linux environments only.");
    }

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
        r_install::install_r(&shell, version, progress)
            .context("failed to install the requested R toolchain")?;
    }

    let repository_path = revdep::prepare_repository(&shell, &workspace, repository, progress)
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
        progress,
    )
    .context("failed to install system requirements for reverse dependencies")?;

    revdep::run_revcheck(&shell, &workspace, &repository_path, num_workers, progress)
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
    progress.println(bundle_hint(&repository_path));

    Ok(())
}

/// Tells the user how to transfer the results, or that there is nothing to
/// transfer because `xfun::rev_check()` reported no differences.
fn bundle_hint(repository_path: &std::path::Path) -> String {
    if bundle::has_results(repository_path) {
        format!(
            "Bundle the results for transfer to another machine with:\n  revdeprun bundle {}",
            util::shell_quote(&repository_path.to_string_lossy())
        )
    } else {
        "No check differences were reported: xfun::rev_check() left no *.Rcheck/ directories \
         or 00check_diffs reports to review."
            .to_string()
    }
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
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn skipping_r_install_does_not_resolve_a_version() {
        let resolved = resolve_r_version_if_installing("release", true, |_| {
            panic!("the resolver must not be called when R installation is skipped")
        })
        .unwrap();

        assert!(resolved.is_none());
    }

    #[test]
    fn bundle_hint_depends_on_leftover_results() {
        let tmp = tempdir().expect("tempdir");
        let pkg = tmp.path().join("my pkg");
        fs::create_dir_all(pkg.join("revdep/library")).expect("library");

        assert!(bundle_hint(&pkg).starts_with("No check differences were reported"));

        fs::create_dir_all(pkg.join("alpha.Rcheck")).expect("Rcheck dir");
        let hint = bundle_hint(&pkg);
        assert!(hint.starts_with("Bundle the results"));
        assert!(hint.ends_with(&format!("revdeprun bundle '{}'", pkg.display())));
    }
}
