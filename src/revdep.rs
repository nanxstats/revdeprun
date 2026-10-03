use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::{NamedTempFile, tempdir_in};
use xshell::{Shell, cmd};

use crate::{
    progress::Progress,
    r_scripts::{self, RConfig},
    util,
    workspace::{self, Workspace},
};

const PKGDEPENDS_PATCH_FILENAME: &str = "patch-pkgdepends.R";
const REVDEP_RBUILDIGNORE_LINE: &str = "^revdep$";

/// Ensures a checkout of the target repository exists within the configured
/// workspace clone root.
///
/// Local paths are used as-is, while remote Git URLs are cloned.
pub fn prepare_repository(
    shell: &Shell,
    workspace: &Workspace,
    spec: &str,
    progress: &Progress,
) -> Result<PathBuf> {
    let candidate = Path::new(spec);
    let repo_path = if candidate.exists() {
        if candidate.is_dir() {
            prepare_local_directory(candidate, progress)?
        } else if candidate.is_file() && is_tarball(candidate) {
            prepare_tarball(shell, workspace, candidate, progress)?
        } else if candidate.is_file() {
            bail!(
                "unsupported local package input {}; expected a directory or .tar.gz archive",
                candidate.display()
            );
        } else {
            bail!(
                "unsupported package input {}; expected a directory or .tar.gz archive",
                candidate.display()
            );
        }
    } else {
        fs::create_dir_all(workspace.clone_root()).with_context(|| {
            format!(
                "failed to create clone root directory {}",
                workspace.clone_root().display()
            )
        })?;

        let repo_name = util::guess_repo_name(spec)
            .ok_or_else(|| anyhow!("unable to infer repository name from {spec}"))?;
        let destination = workspace.clone_root().join(repo_name);
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

        workspace::canonicalized(&destination)?
    };

    ensure_revdep_ignored(&repo_path).with_context(|| {
        format!(
            "failed to update {}",
            repo_path.join(".Rbuildignore").display()
        )
    })?;
    Ok(repo_path)
}

fn prepare_local_directory(candidate: &Path, progress: &Progress) -> Result<PathBuf> {
    let task = progress.task(format!("Using local repository at {}", candidate.display()));
    match workspace::canonicalized(candidate) {
        Ok(path) => {
            task.finish_with_message(format!("Using {}", path.display()));
            Ok(path)
        }
        Err(err) => {
            task.fail(format!(
                "Failed to use local repository {}",
                candidate.display()
            ));
            Err(err)
        }
    }
}

fn prepare_tarball(
    shell: &Shell,
    workspace: &Workspace,
    tarball: &Path,
    progress: &Progress,
) -> Result<PathBuf> {
    let tarball_path = workspace::canonicalized(tarball)
        .with_context(|| format!("failed to resolve tarball path {}", tarball.display()))?;

    let task = progress.task(format!(
        "Preparing package from tarball {}",
        tarball_path.display()
    ));

    let extraction_dir = tempdir_in(workspace.temp_dir()).with_context(|| {
        format!(
            "failed to create extraction directory for {}",
            tarball_path.display()
        )
    })?;
    let extraction_path = extraction_dir.path().to_path_buf();

    let extraction_output = progress.suspend(|| {
        cmd!(shell, "tar -xzf {tarball_path} -C {extraction_path}")
            .quiet()
            .ignore_status()
            .output()
    });

    let output = match extraction_output {
        Ok(output) => output,
        Err(err) => {
            task.fail(format!("Failed to extract {}", tarball_path.display()));
            return Err(err).context("failed to launch tar for package extraction");
        }
    };

    if !output.status.success() {
        task.fail(format!("Failed to extract {}", tarball_path.display()));
        util::emit_command_output(
            progress,
            &format!(
                "tar -xzf {} -C {}",
                tarball_path.display(),
                extraction_path.display()
            ),
            &output.stdout,
            &output.stderr,
        );
        bail!(
            "failed to extract package tarball {}",
            tarball_path.display()
        );
    }

    let package_dir = match locate_package_root(&extraction_path, &tarball_path) {
        Ok(path) => path,
        Err(err) => {
            task.fail(format!("Invalid contents in {}", tarball_path.display()));
            return Err(err);
        }
    };

    let package_name = package_dir
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .or_else(|| infer_package_name(&tarball_path));
    let package_name = match package_name {
        Some(name) => name,
        None => {
            task.fail(format!(
                "Failed to determine package name for {}",
                tarball_path.display()
            ));
            bail!(
                "failed to infer package name from tarball {}",
                tarball_path.display()
            );
        }
    };

    let destination = workspace.temp_dir().join(&package_name);
    if destination.exists() {
        task.fail(format!(
            "Destination {} already exists",
            destination.display()
        ));
        bail!(
            "refusing to overwrite existing directory {}; remove it or choose a different workspace",
            destination.display()
        );
    }

    fs::rename(&package_dir, &destination).with_context(|| {
        format!(
            "failed to move extracted package into {}",
            destination.display()
        )
    })?;

    let canonical_dir = match workspace::canonicalized(&destination) {
        Ok(path) => path,
        Err(err) => {
            task.fail(format!(
                "Failed to resolve extracted directory for {}",
                tarball_path.display()
            ));
            return Err(err);
        }
    };

    task.finish_with_message(format!("Using {}", canonical_dir.display()));
    Ok(canonical_dir)
}

fn ensure_revdep_ignored(repo_path: &Path) -> Result<()> {
    let ignore_path = repo_path.join(".Rbuildignore");
    if ignore_path.exists() {
        let contents = fs::read_to_string(&ignore_path).with_context(|| {
            format!("failed to read .Rbuildignore at {}", ignore_path.display())
        })?;
        if contents
            .lines()
            .any(|line| line.trim() == REVDEP_RBUILDIGNORE_LINE)
        {
            return Ok(());
        }
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&ignore_path)
            .with_context(|| {
                format!(
                    "failed to open .Rbuildignore for append at {}",
                    ignore_path.display()
                )
            })?;
        if !contents.is_empty() && !contents.ends_with('\n') {
            file.write_all(b"\n")
                .context("failed to write newline to .Rbuildignore")?;
        }
        writeln!(file, "{REVDEP_RBUILDIGNORE_LINE}")
            .context("failed to append revdep ignore rule")?;
    } else {
        fs::write(&ignore_path, format!("{REVDEP_RBUILDIGNORE_LINE}\n")).with_context(|| {
            format!(
                "failed to create .Rbuildignore at {}",
                ignore_path.display()
            )
        })?;
    }
    Ok(())
}

fn locate_package_root(extraction_root: &Path, tarball: &Path) -> Result<PathBuf> {
    if extraction_root.join("DESCRIPTION").is_file() {
        return Ok(extraction_root.to_path_buf());
    }

    let entries = fs::read_dir(extraction_root).with_context(|| {
        format!(
            "failed to inspect extracted contents of {}",
            tarball.display()
        )
    })?;

    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to inspect extracted contents of {}",
                tarball.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("DESCRIPTION").is_file() {
            candidates.push(path);
        }
    }

    match candidates.len() {
        1 => Ok(candidates.pop().unwrap()),
        0 => bail!(
            "package tarball {} did not contain a DESCRIPTION file",
            tarball.display()
        ),
        _ => {
            let list = candidates
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "package tarball {} contained multiple candidate package roots: {list}",
                tarball.display()
            )
        }
    }
}

fn is_tarball(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    name.to_ascii_lowercase().ends_with(".tar.gz")
}

fn infer_package_name(tarball: &Path) -> Option<String> {
    let file_name = tarball.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".tar.gz")?;
    let package = stem.split_once('_').map(|(head, _)| head).unwrap_or(stem);
    if package.is_empty() {
        None
    } else {
        Some(package.to_string())
    }
}

/// Runs reverse dependency checks for the repository under `repo_path`.
pub fn run_revcheck(
    shell: &Shell,
    workspace: &Workspace,
    repo_path: &Path,
    num_workers: usize,
    progress: &Progress,
) -> Result<()> {
    let max_connections = util::optimal_max_connections(num_workers);
    let codename = detect_ubuntu_codename().context("failed to detect Ubuntu release codename")?;

    let pkgdepends_patch_path = write_pkgdepends_patch(workspace)?;
    let p3m_state = NamedTempFile::new_in(workspace.temp_dir())
        .context("failed to create P3M rate-limit state file")?;
    let p3m_state_path = workspace::canonicalized(p3m_state.path())
        .context("failed to resolve P3M rate-limit state file")?;
    let install_contents = build_revdep_install_script(
        repo_path,
        num_workers,
        max_connections,
        &codename,
        &pkgdepends_patch_path,
        &p3m_state_path,
    );
    let run_contents = build_revdep_run_script(
        repo_path,
        num_workers,
        max_connections,
        &pkgdepends_patch_path,
        &p3m_state_path,
    );

    let mut install_script = NamedTempFile::new_in(workspace.temp_dir())
        .context("failed to create temporary R script file")?;
    let mut run_script = NamedTempFile::new_in(workspace.temp_dir())
        .context("failed to create temporary R script file")?;

    install_script
        .write_all(install_contents.as_bytes())
        .context("failed to write revdep dependencies install script")?;
    run_script
        .write_all(run_contents.as_bytes())
        .context("failed to write reverse dependency check script")?;

    let install_path = install_script.path().to_owned();
    let run_path = run_script.path().to_owned();

    fs::create_dir_all(repo_path.join("revdep"))
        .with_context(|| format!("failed to create {}", repo_path.join("revdep").display()))?;

    let _dir_guard = shell.push_dir(repo_path);

    let install_task = progress.task("Installing revdep dependencies");
    let install_result = progress.suspend(|| {
        let install_max_connections = max_connections.to_string();
        cmd!(
            shell,
            "Rscript --vanilla --max-connections={install_max_connections} {install_path}"
        )
        .quiet()
        .run()
    });

    match install_result {
        Ok(_) => {
            install_task.finish_with_message("Reverse dependencies installed".to_string());
        }
        Err(err) => {
            install_task.fail("Failed to install revdep dependencies".to_string());
            return Err(err).context("failed to install revdep dependencies");
        }
    }

    progress.println("Launching xfun::rev_check()...");
    progress.suspend(|| {
        let run_max_connections = max_connections.to_string();
        cmd!(
            shell,
            "Rscript --vanilla --max-connections={run_max_connections} {run_path}"
        )
        .quiet()
        .run()
        .context("xfun::rev_check() reported an error")
    })?;

    Ok(())
}

/// Returns the default library directory created for xfun::rev_check().
pub fn revlib_dir(repo_path: &Path) -> PathBuf {
    repo_path.join("revdep")
}

/// Assembles the dependency installation script from the shared prelude and
/// `assets/r/revdep-install.R`.
fn build_revdep_install_script(
    repo_path: &Path,
    num_workers: usize,
    max_connections: usize,
    codename: &str,
    pkgdepends_patch_path: &Path,
    p3m_state_path: &Path,
) -> String {
    let config = script_config(
        repo_path,
        num_workers,
        max_connections,
        pkgdepends_patch_path,
        p3m_state_path,
    )
    .string("ubuntu_codename", &codename.to_lowercase());

    r_scripts::assemble(
        &config,
        &[r_scripts::REVDEP_PRELUDE, r_scripts::REVDEP_INSTALL],
    )
}

/// Assembles the `xfun::rev_check()` script from the shared prelude and
/// `assets/r/revdep-run.R`.
fn build_revdep_run_script(
    repo_path: &Path,
    num_workers: usize,
    max_connections: usize,
    pkgdepends_patch_path: &Path,
    p3m_state_path: &Path,
) -> String {
    let config = script_config(
        repo_path,
        num_workers,
        max_connections,
        pkgdepends_patch_path,
        p3m_state_path,
    );

    r_scripts::assemble(&config, &[r_scripts::REVDEP_PRELUDE, r_scripts::REVDEP_RUN])
}

/// Builds the configuration block consumed by `assets/r/revdep-prelude.R`.
///
/// The install and run scripts must receive the same `p3m_state_path` so that
/// pak binary downloads and xfun source downloads draw from one shared P3M
/// request budget.
fn script_config(
    repo_path: &Path,
    num_workers: usize,
    max_connections: usize,
    pkgdepends_patch_path: &Path,
    p3m_state_path: &Path,
) -> RConfig {
    RConfig::new()
        .path("repo_path", repo_path)
        .integer("install_workers", num_workers.max(1))
        .integer("max_connections", max_connections.max(1))
        .path("pkgdepends_patch_path", pkgdepends_patch_path)
        .path("p3m_state_path", p3m_state_path)
}

fn write_pkgdepends_patch(workspace: &Workspace) -> Result<PathBuf> {
    let patch_path = workspace.temp_dir().join(PKGDEPENDS_PATCH_FILENAME);
    fs::write(&patch_path, r_scripts::PKGDEPENDS_PATCH).with_context(|| {
        format!(
            "failed to write pkgdepends patch to {}",
            patch_path.display()
        )
    })?;
    workspace::canonicalized(&patch_path).with_context(|| {
        format!(
            "failed to resolve pkgdepends patch path {}",
            patch_path.display()
        )
    })
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
    use crate::workspace;
    use std::fs;
    use tempfile::tempdir;
    use xshell::Shell;

    const CONFIG_HEADER: &str = "# Configuration generated by revdeprun ----\n";

    /// Returns the byte offset of `needle` in `script`, failing if absent.
    fn position(script: &str, needle: &str) -> usize {
        script
            .find(needle)
            .unwrap_or_else(|| panic!("script does not contain {needle:?}"))
    }

    fn sample_install_script() -> String {
        build_revdep_install_script(
            Path::new("/tmp/example"),
            8,
            util::optimal_max_connections(8),
            "resolute",
            Path::new("/tmp/patch-pkgdepends.R"),
            Path::new("/tmp/p3m-rate-limit.rds"),
        )
    }

    fn sample_run_script() -> String {
        build_revdep_run_script(
            Path::new("/tmp/example"),
            8,
            util::optimal_max_connections(8),
            Path::new("/tmp/patch-pkgdepends.R"),
            Path::new("/tmp/p3m-rate-limit.rds"),
        )
    }

    #[test]
    fn build_install_script_uses_binary_repo() {
        let max_connections = util::optimal_max_connections(8);
        let script = sample_install_script();

        assert!(script.starts_with(CONFIG_HEADER));
        assert!(script.contains("repo_path <- '/tmp/example'"));
        assert!(script.contains("install_workers <- 8"));
        assert!(script.contains(&format!("max_connections <- {max_connections}")));
        assert!(script.contains("ubuntu_codename <- 'resolute'"));
        assert!(script.contains("setwd(repo_path)"));
        assert!(script.contains("https://packagemanager.posit.co/cran/__linux__/%s/latest"));
        assert!(script.contains(
            "sprintf(\"https://packagemanager.posit.co/cran/__linux__/%s/latest\", ubuntu_codename)"
        ));
        assert!(script.contains("install.packages(\n      \"pak\""));
        assert!(script.contains("pak::pkg_install("));
        assert!(script.contains("pak_install_retry <- function"));
        assert!(script.contains("pak_install_retry <- function(pkgs, attempts = 5)"));
        assert!(script.contains("pak_install_retry(pkg)"));
        assert!(script.contains("pak_install_retry(install_targets)"));
        assert!(script.contains("?ignore-build-errors&ignore-unavailable"));
        assert!(script.contains("ensure_pak(source_repo)"));
        assert!(script.contains("pkgdepends_patch_path <- '/tmp/patch-pkgdepends.R'"));
        assert!(script.contains("p3m_state_path <- '/tmp/p3m-rate-limit.rds'"));
        assert!(script.contains("source(pkgdepends_patch_path)"));
        assert!(
            script.contains("pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)")
        );
        assert!(script.contains(
            "Parsing package metadata and dependency lists...\\nThis can take a few minutes for large revdep sets."
        ));
        assert!(script.contains("async_http_total_con = max_connections"));
        assert!(script.contains("async_http_host_con = 50"));
        assert!(script.contains("options(Ncpus = install_workers)"));
        assert!(script.contains("parse_description_dependencies <- function"));
        assert!(
            script.contains("dev_package_deps <- parse_description_dependencies(\"DESCRIPTION\"")
        );
        assert!(script.contains("cran_package_deps <- tools::package_dependencies("));
        assert!(script.contains("reverse = FALSE"));
        assert!(
            script.contains(
                "install_targets <- sort(unique(c(package_name, dev_package_deps, cran_package_deps, revdeps)))"
            )
        );
        assert!(script.contains("dependency_map <- tools::package_dependencies("));
        assert!(script.contains("recursive = FALSE"));
        assert!(script.contains("repos = c(CRAN = binary_repo, posit = binary_repo)"));
        assert!(script.contains("revdep_dir <- file.path(getwd(), \"revdep\")"));
        assert!(script.contains(
            "revdep_dir <- normalizePath(revdep_dir, winslash = \"/\", mustWork = TRUE)"
        ));
        assert!(script.contains("Skipping packages not available from repository"));
        assert!(script.contains(".libPaths(unique(c(library_dir, .libPaths())))"));
        assert!(script.contains("Installing %d packages with pak::pkg_install()..."));
    }

    #[test]
    fn build_install_script_preserves_execution_order() {
        let script = sample_install_script();

        // Configuration precedes the prelude, which precedes the install body.
        assert!(
            position(&script, "repo_path <- '/tmp/example'")
                < position(&script, "setwd(repo_path)")
        );
        assert!(
            position(&script, "ubuntu_codename <- 'resolute'")
                < position(&script, "# Prepare workspace directories ----")
        );
        assert!(
            position(&script, "ensure_installed <- function")
                < position(&script, "# Configure repositories ----")
        );
        // Within the body: repos, pak, patch, tooling, then installation.
        assert!(
            position(&script, "ensure_pak(source_repo)")
                < position(&script, "source(pkgdepends_patch_path)")
        );
        assert!(
            position(
                &script,
                "pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)"
            ) < position(&script, "ensure_installed(\"xfun\")")
        );
        assert!(
            position(&script, "ensure_installed(\"xfun\")")
                < position(&script, "pak_install_retry(install_targets)")
        );
    }

    #[test]
    fn build_run_script_invokes_xfun() {
        let max_connections = util::optimal_max_connections(8);
        let script = sample_run_script();

        assert!(script.starts_with(CONFIG_HEADER));
        assert!(script.contains("repo_path <- '/tmp/example'"));
        assert!(script.contains("install_workers <- 8"));
        assert!(script.contains(&format!("max_connections <- {max_connections}")));
        assert!(!script.contains("ubuntu_codename"));
        assert!(script.contains("setwd(repo_path)"));
        assert!(script.contains("xfun::rev_check"));
        assert!(script.contains("src = \".\""));
        assert!(script.contains("mc.cores = install_workers"));
        assert!(script.contains("ensure_installed(\"markdown\")"));
        assert!(script.contains("ensure_installed(\"rmarkdown\")"));
        assert!(script.contains("install.packages(\n      \"pak\""));
        assert!(script.contains("pak::pkg_install("));
        assert!(script.contains("pak_install_retry <- function"));
        assert!(script.contains("pak_install_retry <- function(pkgs, attempts = 5)"));
        assert!(script.contains("pak_install_retry(pkg)"));
        assert!(script.contains("ensure_pak(source_repo)"));
        assert!(script.contains("pkgdepends_patch_path <- '/tmp/patch-pkgdepends.R'"));
        assert!(script.contains("p3m_state_path <- '/tmp/p3m-rate-limit.rds'"));
        assert!(script.contains("source(pkgdepends_patch_path)"));
        assert!(
            script.contains("pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)")
        );
        assert!(script.contains("xfun_patch_p3m_downloads(p3m_state_path)"));
        assert!(script.contains("?ignore-build-errors&ignore-unavailable"));
        assert!(script.contains("async_http_total_con = max_connections"));
        assert!(script.contains("async_http_host_con = 50"));
        assert!(script.contains("options("));
        assert!(script.contains("browser = \"false\""));
        assert!(script.contains("install.packages.compile.from.source = \"always\""));
        assert!(script.contains("xfun.rev_check.compare = TRUE"));
        assert!(script.contains("xfun.rev_check.download_cores = 50"));
        assert!(script.contains("xfun.rev_check.timeout = 30 * 60"));
        assert!(script.contains("xfun.rev_check.summary = TRUE"));
        assert!(script.contains("xfun.rev_check.sample = Inf"));
        assert!(script.contains("xfun.rev_check.keep_md = TRUE"));
        assert!(script.contains("xfun.rev_check.timeout_total = Inf"));
        assert!(script.contains("library_dir <- file.path(revdep_dir, \"library\")"));
        assert!(script.contains(
            "library_dir <- normalizePath(library_dir, winslash = \"/\", mustWork = TRUE)"
        ));
    }

    #[test]
    fn build_run_script_preserves_execution_order() {
        let script = sample_run_script();

        assert!(
            position(&script, "repo_path <- '/tmp/example'")
                < position(&script, "setwd(repo_path)")
        );
        assert!(
            position(&script, "ensure_installed <- function")
                < position(&script, "# Configure repositories ----")
        );
        assert!(
            position(&script, "ensure_pak(source_repo)")
                < position(&script, "source(pkgdepends_patch_path)")
        );
        assert!(
            position(
                &script,
                "pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)"
            ) < position(&script, "ensure_installed(\"xfun\")")
        );
        assert!(
            position(&script, "ensure_installed(\"rmarkdown\")")
                < position(&script, "xfun_patch_p3m_downloads(p3m_state_path)")
        );
        assert!(
            position(&script, "xfun_patch_p3m_downloads(p3m_state_path)")
                < position(&script, "xfun.rev_check.compare = TRUE")
        );
        assert!(
            position(&script, "xfun.rev_check.timeout_total = Inf")
                < position(
                    &script,
                    "results <- xfun::rev_check(package_name, src = \".\")"
                )
        );
    }

    #[test]
    fn install_and_run_scripts_share_rate_limit_state() {
        let install = sample_install_script();
        let run = sample_run_script();

        for script in [&install, &run] {
            assert!(script.contains("p3m_state_path <- '/tmp/p3m-rate-limit.rds'"));
            assert!(script.contains(
                "p3m_state_path <- normalizePath(p3m_state_path, winslash = \"/\", mustWork = TRUE)"
            ));
            assert!(
                script
                    .contains("pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)")
            );
        }
        assert!(run.contains("xfun_patch_p3m_downloads(p3m_state_path)"));
        assert!(!install.contains("xfun_patch_p3m_downloads("));
    }

    #[test]
    fn script_config_escapes_paths_and_clamps_counts() {
        let config = script_config(
            Path::new("/tmp/O'Reilly"),
            0,
            0,
            Path::new("/tmp/patch.R"),
            Path::new("/tmp/state.rds"),
        );
        let rendered = config.render();

        assert!(rendered.contains("repo_path <- '/tmp/O\\'Reilly'"));
        assert!(rendered.contains("install_workers <- 1\n"));
        assert!(rendered.contains("max_connections <- 1\n"));
        assert!(rendered.contains("pkgdepends_patch_path <- '/tmp/patch.R'"));
        assert!(rendered.contains("p3m_state_path <- '/tmp/state.rds'"));
    }

    #[test]
    fn pkgdepends_patch_throttles_p3m_downloads_across_pak_and_xfun() {
        let patch = r_scripts::PKGDEPENDS_PATCH;
        assert!(patch.contains("P3M_REQUEST_LIMIT <- 1800L"));
        assert!(patch.contains("P3M_WINDOW_SECONDS <- 5 * 60 + 5"));
        assert!(patch.contains("patched_pkgplan_async_download_internal"));
        assert!(patch.contains("P3M_REQUEST_LIMIT - state$used"));
        assert!(patch.contains("async_delay(wait)$then("));
        assert!(!patch.contains("\n          delay(wait)$then("));
        assert!(patch.contains("p3m_rate_limit_record("));
        assert!(patch.contains("xfun_patch_p3m_downloads <- function"));
        assert!(patch.contains("original_download_tarball("));
        assert!(patch.contains(") & !file.exists(expected)"));
        assert_eq!(
            patch
                .matches("P3M request budget reached; resuming downloads in %.0f seconds.")
                .count(),
            2
        );
    }

    #[test]
    fn parses_codename_from_os_release() {
        let contents = r#"
NAME="Ubuntu"
VERSION="26.04 LTS (Resolute Raccoon)"
VERSION_CODENAME=resolute
UBUNTU_CODENAME=resolute
"#;
        let codename = ubuntu_codename_from_os_release(contents);
        assert_eq!(codename.as_deref(), Some("resolute"));
    }

    #[test]
    fn detects_tarball_filenames() {
        assert!(is_tarball(Path::new("pkg_0.1.0.tar.gz")));
        assert!(is_tarball(Path::new("pkg.TAR.GZ")));
        assert!(!is_tarball(Path::new("pkg.zip")));
        assert!(!is_tarball(Path::new("pkg.tar")));
        assert!(!is_tarball(Path::new("pkg.tgz")));
    }

    #[test]
    fn ensures_revdep_is_ignored() {
        let tmp = tempdir().expect("tempdir");
        let repo_path = tmp.path();
        ensure_revdep_ignored(repo_path).expect("ignore rule");
        let contents =
            fs::read_to_string(repo_path.join(".Rbuildignore")).expect("ignore contents");
        assert!(
            contents
                .lines()
                .any(|line| line.trim() == REVDEP_RBUILDIGNORE_LINE)
        );

        ensure_revdep_ignored(repo_path).expect("ignore rule");
        let updated = fs::read_to_string(repo_path.join(".Rbuildignore")).expect("ignore contents");
        let matches = updated
            .lines()
            .filter(|line| line.trim() == REVDEP_RBUILDIGNORE_LINE)
            .count();
        assert_eq!(matches, 1);
    }

    #[test]
    fn ensures_revdep_ignored_adds_newline() {
        let tmp = tempdir().expect("tempdir");
        let repo_path = tmp.path();
        fs::write(repo_path.join(".Rbuildignore"), "^README\\.Rmd$").expect("write ignore file");

        ensure_revdep_ignored(repo_path).expect("ignore rule");

        let contents =
            fs::read_to_string(repo_path.join(".Rbuildignore")).expect("ignore contents");
        assert_eq!(contents, "^README\\.Rmd$\n^revdep$\n");
    }

    #[test]
    fn prepares_repository_from_tarball() {
        let shell = Shell::new().expect("shell");
        let tmp = tempdir().expect("tempdir");

        let package_name = "mypkg";
        let package_root = tmp.path().join(package_name);
        fs::create_dir_all(&package_root).expect("package directory");
        fs::write(
            package_root.join("DESCRIPTION"),
            "Package: mypkg\nVersion: 0.1.0\n",
        )
        .expect("description");
        fs::create_dir_all(package_root.join("R")).expect("R directory");
        fs::write(
            package_root.join("R").join("hello.R"),
            "hello <- function() 1",
        )
        .expect("R script");

        let tarball_path = tmp.path().join("mypkg_0.1.0.tar.gz");
        {
            let _dir = shell.push_dir(tmp.path());
            cmd!(shell, "tar -czf {tarball_path} {package_name}")
                .quiet()
                .run()
                .expect("create tarball");
        }

        let workspace_root = tmp.path().join("workspace");
        let workspace = workspace::prepare(Some(workspace_root.clone())).expect("workspace");
        let progress = Progress::new();

        let repo_path = prepare_repository(
            &shell,
            &workspace,
            tarball_path.to_str().expect("utf8 path"),
            &progress,
        )
        .expect("prepared repository");

        assert!(repo_path.join("DESCRIPTION").exists());
        let ignore_path = repo_path.join(".Rbuildignore");
        assert!(ignore_path.is_file());
        let ignore_contents = fs::read_to_string(&ignore_path).expect("ignore contents");
        assert!(
            ignore_contents
                .lines()
                .any(|line| line.trim() == REVDEP_RBUILDIGNORE_LINE)
        );
        let expected = workspace::canonicalized(&workspace_root.join("mypkg"))
            .expect("canonical expected path");
        assert_eq!(repo_path, expected);
    }
}
