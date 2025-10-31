use std::{
    env, fs,
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
            util::emit_command_output(
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

/// Runs reverse dependency checks for the repository under `repo_path`.
pub fn run_revdepcheck(
    shell: &Shell,
    workspace: &Path,
    repo_path: &Path,
    num_workers: usize,
    progress: &Progress,
) -> Result<()> {
    let codename = detect_ubuntu_codename().context("failed to detect Ubuntu release codename")?;

    let install_contents = build_revdep_install_script(repo_path, num_workers, &codename)?;
    let run_contents = build_revdep_run_script(repo_path, num_workers)?;

    let mut install_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    let mut run_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;

    install_script
        .write_all(install_contents.as_bytes())
        .context("failed to write pak install script")?;
    run_script
        .write_all(run_contents.as_bytes())
        .context("failed to write reverse dependency check script")?;

    let install_path = install_script.path().to_owned();
    let run_path = run_script.path().to_owned();

    fs::create_dir_all(repo_path.join("revdep"))
        .with_context(|| format!("failed to create {}", repo_path.join("revdep").display()))?;

    let _dir_guard = shell.push_dir(repo_path);

    let install_task = progress.task("Installing reverse dependencies with pak");
    let install_output = cmd!(shell, "Rscript --vanilla {install_path}")
        .quiet()
        .ignore_status()
        .output();

    match install_output {
        Ok(output) if output.status.success() => {
            install_task.finish_with_message("Reverse dependencies installed".to_string());
        }
        Ok(output) => {
            install_task.fail("Failed to install reverse dependencies via pak".to_string());
            util::emit_command_output(
                progress,
                "pak reverse dependency installation",
                &output.stdout,
                &output.stderr,
            );
            bail!(
                "pak reverse dependency installation failed with status {}",
                output.status
            );
        }
        Err(err) => {
            install_task.fail("Failed to launch pak reverse dependency installation".to_string());
            return Err(err).context("failed to install reverse dependencies via pak");
        }
    }

    progress.println("Launching xfun::rev_check()...");
    progress.suspend(|| {
        cmd!(shell, "Rscript --vanilla {run_path}")
            .quiet()
            .run()
            .context("xfun::rev_check() reported an error")
    })?;

    Ok(())
}

/// Returns the default results directory created by revdepcheck.
pub fn results_dir(repo_path: &Path) -> PathBuf {
    repo_path.join("revdep")
}

fn build_revdep_install_script(
    repo_path: &Path,
    num_workers: usize,
    codename: &str,
) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);
    let codename_literal = util::r_string_literal(&codename.to_lowercase());

    let script = format!(
        r#"{prelude}

binary_repo <- sprintf("https://packagemanager.posit.co/cran/__linux__/%s/latest", {codename_literal})
source_repo <- "https://packagemanager.posit.co/cran/latest"

options(
  repos = c(posit = binary_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers
)
Sys.setenv(NOT_CRAN = "true")

ensure_installed <- function(pkg, repo = source_repo) {{
  if (!requireNamespace(pkg, quietly = TRUE)) {{
    install.packages(
      pkg,
      repos = repo,
      lib = library_dir,
      quiet = TRUE,
      Ncpus = install_workers
    )
  }}
}}

ensure_installed("pak")
ensure_installed("xfun")

pak::repo_add(posit = binary_repo)

package_name <- read.dcf("DESCRIPTION", fields = "Package")[1, 1]
if (!nzchar(package_name)) {{
  stop("Failed to read package name from DESCRIPTION")
}}

db <- available.packages(repos = source_repo, type = "source")
revdeps <- tools::package_dependencies(
  packages = package_name,
  db = db,
  which = c("Depends", "Imports", "LinkingTo", "Suggests"),
  reverse = TRUE
)[[package_name]]

revdeps <- sort(unique(stats::na.omit(revdeps)))

if (length(revdeps) == 0) {{
  message("No CRAN reverse dependencies detected; skipping pak::pkg_install().")
}} else {{
  base_pkgs <- unique(c(.BaseNamespaceEnv$basePackage, rownames(installed.packages(priority = "base"))))
  revdeps <- setdiff(revdeps, base_pkgs)
  if (length(revdeps) == 0) {{
    message("Reverse dependencies only included base packages; nothing to install.")
  }} else {{
    pak::pkg_install(
      paste0("any::", revdeps),
      ask = FALSE,
      dependencies = TRUE,
      lib = library_dir,
      upgrade = FALSE
    )
  }}
}}
"#
    );

    Ok(script)
}

fn build_revdep_run_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);

    let script = format!(
        r#"{prelude}

source_repo <- "https://packagemanager.posit.co/cran/latest"

options(
  repos = c(CRAN = source_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers,
  mc.cores = install_workers
)
Sys.setenv(NOT_CRAN = "true")

if (!requireNamespace("xfun", quietly = TRUE)) {{
  install.packages(
    "xfun",
    repos = source_repo,
    lib = library_dir,
    quiet = TRUE,
    Ncpus = install_workers
  )
}}

package_name <- read.dcf("DESCRIPTION", fields = "Package")[1, 1]
if (!nzchar(package_name)) {{
  stop("Failed to read package name from DESCRIPTION")
}}

results <- xfun::rev_check(package_name, src = ".")
invisible(results)
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

revdep_dir <- file.path("revdep")
dir.create(revdep_dir, recursive = TRUE, showWarnings = FALSE)

library_dir <- file.path(revdep_dir, "library")
dir.create(library_dir, recursive = TRUE, showWarnings = FALSE)

Sys.setenv(R_LIBS_USER = library_dir)
.libPaths(c(library_dir, .libPaths()))

install_workers <- max({workers}, parallel::detectCores())
options(Ncpus = install_workers)
"#
    )
}

fn detect_ubuntu_codename() -> Result<String> {
    if let Ok(value) = env::var("REVDEPRUN_UBUNTU_CODENAME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_lowercase());
        }
    }

    let contents =
        fs::read_to_string("/etc/os-release").context("failed to read /etc/os-release")?;

    if let Some(codename) = ubuntu_codename_from_os_release(&contents) {
        return Ok(codename);
    }

    bail!("VERSION_CODENAME not found in /etc/os-release")
}

fn ubuntu_codename_from_os_release(contents: &str) -> Option<String> {
    let mut fallback = None;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        let key = key.trim();
        let mut value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if value.is_empty() {
            continue;
        }
        value = value.to_lowercase();

        if key == "VERSION_CODENAME" {
            return Some(value);
        }
        if key == "UBUNTU_CODENAME" {
            fallback = Some(value);
        }
    }

    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_install_script_uses_binary_repo() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_install_script(path, 8, "noble").expect("script must build");

        assert!(script.contains("https://packagemanager.posit.co/cran/__linux__/%s/latest"));
        assert!(script.contains(
            "sprintf(\"https://packagemanager.posit.co/cran/__linux__/%s/latest\", 'noble')"
        ));
        assert!(script.contains("ensure_installed(\"pak\")"));
        assert!(script.contains("pak::pkg_install"));
        assert!(script.contains("paste0(\"any::\", revdeps)"));
        assert!(script.contains("setwd('/tmp/example')"));
    }

    #[test]
    fn build_run_script_invokes_xfun() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_run_script(path, 8).expect("script must build");

        assert!(script.contains("xfun::rev_check"));
        assert!(script.contains("src = \".\""));
        assert!(script.contains("mc.cores = install_workers"));
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains("library_dir <- file.path(revdep_dir, \"library\")"));
    }

    #[test]
    fn parses_codename_from_os_release() {
        let contents = r#"
NAME="Ubuntu"
VERSION="24.04 LTS (Noble Nimbus)"
VERSION_CODENAME=noble
UBUNTU_CODENAME=noble
"#;
        let codename = ubuntu_codename_from_os_release(contents);
        assert_eq!(codename.as_deref(), Some("noble"));
    }
}
