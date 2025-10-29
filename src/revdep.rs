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
            emit_command_output(progress, spec, &output.stdout, &output.stderr);
            bail!("failed to clone repository {spec}");
        }
        Err(err) => {
            clone_task.fail(format!("Cloning {spec} failed to start"));
            return Err(err).with_context(|| format!("failed to clone repository {spec}"));
        }
    }

    workspace::canonicalized(&destination)
}

fn emit_command_output(progress: &Progress, spec: &str, stdout: &[u8], stderr: &[u8]) {
    emit_stream(progress, spec, "stdout", stdout);
    emit_stream(progress, spec, "stderr", stderr);
}

fn emit_stream(progress: &Progress, spec: &str, stream: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    progress.println(format!("git clone {spec} {stream}:\n{trimmed}"));
}

/// Runs `revdepcheck` for the repository under `repo_path`.
pub fn run_revdepcheck(
    shell: &Shell,
    workspace: &Path,
    repo_path: &Path,
    num_workers: usize,
    progress: &Progress,
) -> Result<()> {
    let script_contents = build_revdep_script(repo_path, num_workers)?;
    let mut script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    script
        .write_all(script_contents.as_bytes())
        .context("failed to write revdepcheck bootstrap script")?;

    progress.clear();
    progress.println(format!(
        "Launching revdepcheck with {num_workers} workers..."
    ));
    let script_path = script.path().to_owned();
    let _dir_guard = shell.push_dir(repo_path);
    progress.suspend(|| {
        cmd!(shell, "Rscript --vanilla {script_path}")
            .run()
            .context("revdepcheck reported an error")
    })?;

    Ok(())
}

/// Returns the default results directory created by revdepcheck.
pub fn results_dir(repo_path: &Path) -> PathBuf {
    repo_path.join("revdep")
}

fn build_revdep_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let path_literal = util::r_string_literal(&repo_path.to_string_lossy());
    let workers = num_workers.max(1);

    let script = format!(
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

if (!requireNamespace("BiocManager", quietly = TRUE)) {{
  install.packages("BiocManager", repos = "https://cloud.r-project.org/", lib = user_lib)
}}
if (!requireNamespace("remotes", quietly = TRUE)) {{
  install.packages("remotes", repos = "https://cloud.r-project.org/", lib = user_lib)
}}
if (!requireNamespace("revdepcheck", quietly = TRUE)) {{
  remotes::install_github("r-lib/revdepcheck", lib = user_lib, upgrade = "never", Ncpus = {workers})
}}

Sys.setenv(R_BIOC_VERSION = as.character(BiocManager::version()))
revdepcheck::revdep_reset()
revdepcheck::revdep_check(num_workers = {workers}, quiet = FALSE)
"#
    );

    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_script_with_expected_fragments() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_script(path, 8).expect("script must build");

        assert!(script.contains("revdepcheck::revdep_check"));
        assert!(script.contains("num_workers = 8"));
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains(".libPaths(c(user_lib, .libPaths()))"));
        assert!(script.contains("lib = user_lib"));
    }
}
