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
    let shell = Shell::new().context("failed to initialise shell environment")?;

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
    let resolved_version = {
        let task = progress.task(version_label.clone());
        match r_version::resolve(&args.r_version).context("failed to resolve requested R version") {
            Ok(version) => {
                task.finish_with_message(format!("Resolved R {}", version.version));
                version
            }
            Err(err) => {
                task.fail(format!("{version_label} (failed)"));
                return Err(err);
            }
        }
    };

    if args.skip_r_install {
        progress.println("Skipping R installation as requested.");
    } else {
        r_install::install_r(&shell, &resolved_version, &progress)
            .context("failed to install the requested R toolchain")?;
    }

    let repository_path =
        revdep::prepare_repository(&shell, &workspace, &args.repository, &progress)
            .context("failed to prepare target repository")?;

    let num_workers = args
        .num_workers
        .map(|value| value.get())
        .unwrap_or_else(num_cpus::get);

    sysreqs::install_reverse_dep_sysreqs(
        &shell,
        &workspace,
        &repository_path,
        num_workers,
        &progress,
    )
    .context("failed to install system requirements for reverse dependencies")?;

    revdep::run_revdepcheck(&shell, &workspace, &repository_path, num_workers, &progress)
        .context("reverse dependency check invocation failed")?;

    progress.println(format!(
        "Reverse dependency check finished successfully.\n  • R version: {}\n  • repository: {}\n  • results: {}",
        resolved_version.version,
        repository_path.display(),
        revdep::results_dir(&repository_path).display()
    ));

    Ok(())
}
