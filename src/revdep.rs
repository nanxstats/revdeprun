use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::NamedTempFile;
use xshell::{Shell, cmd};

use crate::{progress::Progress, util, workspace};

/// Ensures a checkout of the target repository exists within `workspace`.
///
/// Local paths are used as-is, while remote Git URLs are cloned.
pub fn prepare_repository(
    shell: &Shell,
    workspace: &Path,
    spec: &str,
    progress: &Progress,
) -> Result<PathBuf> {
    let candidate = Path::new(spec);
    if candidate.exists() {
        let task = progress.task(format!("Using local repository at {}", candidate.display()));
        match workspace::canonicalized(candidate) {
            Ok(path) => {
                task.finish_with_message(format!("Using {}", path.display()));
                return Ok(path);
            }
            Err(err) => {
                task.fail(format!(
                    "Failed to use local repository {}",
                    candidate.display()
                ));
                return Err(err);
            }
        }
    }

    fs::create_dir_all(workspace).with_context(|| {
        format!(
            "failed to create workspace directory {}",
            workspace.display()
        )
    })?;

    let repo_name = util::guess_repo_name(spec)
        .ok_or_else(|| anyhow!("unable to infer repository name from {spec}"))?;
    let destination = workspace.join(repo_name);
    if destination.exists() {
        anyhow::bail!(
            "refusing to clone into {} because the directory already exists",
            destination.display()
        );
    }

    let clone_task = progress.task(format!("Cloning {spec} into {}", destination.display()));
    let output = cmd!(shell, "git clone --depth 1 {spec} {destination}")
        .quiet()
        .ignore_status()
        .output();

    match output {
        Ok(output) if output.status.success() => {
            clone_task.finish_with_message(format!("Cloned into {}", destination.display()));
        }
        Ok(output) => {
            clone_task.fail(format!("Cloning {spec} failed"));
            emit_command_output(
                progress,
                &format!("git clone {spec}"),
                &output.stdout,
                &output.stderr,
            );
            bail!("failed to clone repository {spec}");
        }
        Err(err) => {
            clone_task.fail(format!("Cloning {spec} failed to start"));
            return Err(err).with_context(|| format!("failed to clone repository {spec}"));
        }
    }

    workspace::canonicalized(&destination)
}

fn emit_command_output(progress: &Progress, label: &str, stdout: &[u8], stderr: &[u8]) {
    emit_stream(progress, label, "stdout", stdout);
    emit_stream(progress, label, "stderr", stderr);
}

fn emit_stream(progress: &Progress, label: &str, stream: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    progress.println(format!("{label} {stream}:\n{trimmed}"));
}

/// Runs `revdepcheck` for the repository under `repo_path`.
pub fn run_revdepcheck(
    shell: &Shell,
    workspace: &Path,
    repo_path: &Path,
    num_workers: usize,
    progress: &Progress,
) -> Result<()> {
    let setup_contents = build_revdep_setup_script(repo_path, num_workers)?;
    let mut setup_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    setup_script
        .write_all(setup_contents.as_bytes())
        .context("failed to write revdep bootstrap script")?;

    let run_contents = build_revdep_run_script(repo_path, num_workers)?;
    let mut run_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    run_script
        .write_all(run_contents.as_bytes())
        .context("failed to write revdep execution script")?;

    let setup_path = setup_script.path().to_owned();
    let run_path = run_script.path().to_owned();

    let _dir_guard = shell.push_dir(repo_path);

    let bootstrap_task = progress.task("Bootstrapping revdep dependencies");
    let setup_output = cmd!(shell, "Rscript --vanilla {setup_path}")
        .quiet()
        .ignore_status()
        .output();

    match setup_output {
        Ok(output) if output.status.success() => {
            bootstrap_task.finish_with_message("Revdep dependencies ready".to_string());
        }
        Ok(output) => {
            bootstrap_task.fail("Failed to prepare revdep dependencies".to_string());
            emit_command_output(
                progress,
                "revdep dependency bootstrap",
                &output.stdout,
                &output.stderr,
            );
            bail!(
                "revdep dependency bootstrap failed with status {}",
                output.status
            );
        }
        Err(err) => {
            bootstrap_task.fail("Failed to launch revdep bootstrap".to_string());
            return Err(err).context("failed to prepare revdep dependencies");
        }
    }

    progress.println(format!(
        "Launching revdepcheck with {num_workers} workers..."
    ));
    progress.suspend(|| {
        cmd!(shell, "Rscript --vanilla {run_path}")
            .quiet()
            .run()
            .context("revdepcheck reported an error")
    })?;

    Ok(())
}

/// Returns the default results directory created by revdepcheck.
pub fn results_dir(repo_path: &Path) -> PathBuf {
    repo_path.join("revdep")
}

fn build_revdep_setup_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);
    let workers = num_workers.max(1);

    let script = format!(
        r#"{prelude}

if (!requireNamespace("BiocManager", quietly = TRUE)) {{
  install.packages(
    "BiocManager",
    repos = "https://cloud.r-project.org/",
    lib = user_lib,
    quiet = TRUE,
    Ncpus = {workers}
  )
}}
if (!requireNamespace("remotes", quietly = TRUE)) {{
  install.packages(
    "remotes",
    repos = "https://cloud.r-project.org/",
    lib = user_lib,
    quiet = TRUE,
    Ncpus = {workers}
  )
}}
if (!requireNamespace("revdepcheck", quietly = TRUE)) {{
  remotes::install_github(
    "r-lib/revdepcheck",
    lib = user_lib,
    upgrade = "never",
    quiet = TRUE,
    Ncpus = {workers}
  )
}}
"#
    );

    Ok(script)
}

fn build_revdep_run_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);
    let workers = num_workers.max(1);

    let script = format!(
        r#"{prelude}

Sys.setenv(R_BIOC_VERSION = as.character(BiocManager::version()))
revdepcheck::revdep_reset()
revdepcheck::revdep_check(num_workers = {workers}, quiet = FALSE)
"#
    );

    Ok(script)
}

fn script_prelude(repo_path: &Path, num_workers: usize) -> String {
    let path_literal = util::r_string_literal(&repo_path.to_string_lossy());
    let workers = num_workers.max(1);

    format!(
        r#"
setwd({path_literal})
options(
  repos = "https://cloud.r-project.org/",
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = {workers}
)
Sys.setenv(NOT_CRAN = "true")

user_lib <- Sys.getenv("R_LIBS_USER")
if (!nzchar(user_lib)) {{
  stop('R_LIBS_USER is empty; cannot install packages into user library')
}}
dir.create(user_lib, recursive = TRUE, showWarnings = FALSE)
.libPaths(c(user_lib, .libPaths()))
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_setup_script_installs_dependencies_quietly() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_setup_script(path, 8).expect("script must build");

        assert!(script.contains("install.packages("));
        assert!(script.contains("quiet = TRUE"));
        assert!(script.contains("remotes::install_github"));
    }

    #[test]
    fn build_run_script_triggers_revdepcheck() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_run_script(path, 8).expect("script must build");

        assert!(script.contains("revdepcheck::revdep_check"));
        assert!(script.contains("num_workers = 8"));
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains(".libPaths(c(user_lib, .libPaths()))"));
        assert!(script.contains("revdepcheck::revdep_reset"));
    }
}
