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
    util,
    workspace::{self, Workspace},
};

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
    if candidate.exists() {
        if candidate.is_dir() {
            return prepare_local_directory(candidate, progress);
        } else if candidate.is_file() && is_tarball(candidate) {
            return prepare_tarball(shell, workspace, candidate, progress);
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
    }

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

    workspace::canonicalized(&destination)
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
    let extraction_path = extraction_dir.keep();

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

    let canonical_dir = match workspace::canonicalized(&package_dir) {
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

    let install_contents = build_revdep_install_script(repo_path, num_workers, &codename)?;
    let run_contents = build_revdep_run_script(repo_path, num_workers)?;

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

ensure_installed("xfun")

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

base_pkgs <- unique(c(.BaseNamespaceEnv$basePackage, rownames(installed.packages(priority = "base"))))
revdeps <- setdiff(revdeps, base_pkgs)

install_targets <- sort(unique(c(package_name, revdeps)))

available_packages <- rownames(db)
missing_packages <- setdiff(install_targets, available_packages)
if (length(missing_packages) > 0) {{
  message(
    "Skipping packages not available from repository: ",
    paste(missing_packages, collapse = ", ")
  )
}}
install_targets <- setdiff(install_targets, missing_packages)

dependency_kinds <- c("Depends", "Imports", "LinkingTo", "Suggests")
dependency_map <- tools::package_dependencies(
  packages = install_targets,
  db = db,
  which = dependency_kinds,
  recursive = FALSE
)
extra_deps <- unique(unlist(dependency_map, use.names = FALSE))
extra_deps <- extra_deps[!is.na(extra_deps) & nzchar(extra_deps)]
extra_deps <- intersect(extra_deps, available_packages)
extra_deps <- setdiff(extra_deps, c(base_pkgs, install_targets))
install_targets <- sort(unique(c(install_targets, extra_deps)))

if (length(revdeps) == 0) {{
  message("No CRAN reverse dependencies detected; installing package binary only.")
}}

if (length(install_targets) > 0) {{
  install.packages(
    install_targets,
    repos = binary_repo,
    lib = library_dir,
    quiet = TRUE,
    Ncpus = install_workers
  )
}} else {{
  stop("No installation targets determined for install.packages().")
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

ensure_installed <- function(pkg) {{
  if (!requireNamespace(pkg, quietly = TRUE)) {{
    install.packages(
      pkg,
      repos = source_repo,
      lib = library_dir,
      quiet = TRUE,
      Ncpus = install_workers
    )
  }}
}}

ensure_installed("xfun")
ensure_installed("markdown")
ensure_installed("rmarkdown")

options(xfun.rev_check.summary = TRUE)

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
    use crate::workspace;
    use std::fs;
    use tempfile::tempdir;
    use xshell::Shell;

    #[test]
    fn build_install_script_uses_binary_repo() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_install_script(path, 8, "noble").expect("script must build");

        assert!(script.contains("https://packagemanager.posit.co/cran/__linux__/%s/latest"));
        assert!(script.contains(
            "sprintf(\"https://packagemanager.posit.co/cran/__linux__/%s/latest\", 'noble')"
        ));
        assert!(script.contains("install.packages("));
        assert!(script.contains("install_targets <- sort(unique(c(package_name, revdeps)))"));
        assert!(script.contains("dependency_map <- tools::package_dependencies("));
        assert!(script.contains("recursive = FALSE"));
        assert!(script.contains("repos = binary_repo"));
        assert!(script.contains("Skipping packages not available from repository"));
        assert!(script.contains("setwd('/tmp/example')"));
    }

    #[test]
    fn build_run_script_invokes_xfun() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_run_script(path, 8).expect("script must build");

        assert!(script.contains("xfun::rev_check"));
        assert!(script.contains("src = \".\""));
        assert!(script.contains("mc.cores = install_workers"));
        assert!(script.contains("ensure_installed(\"markdown\")"));
        assert!(script.contains("ensure_installed(\"rmarkdown\")"));
        assert!(script.contains("options(xfun.rev_check.summary = TRUE)"));
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

    #[test]
    fn detects_tarball_filenames() {
        assert!(is_tarball(Path::new("pkg_0.1.0.tar.gz")));
        assert!(is_tarball(Path::new("pkg.TAR.GZ")));
        assert!(!is_tarball(Path::new("pkg.zip")));
        assert!(!is_tarball(Path::new("pkg.tar")));
        assert!(!is_tarball(Path::new("pkg.tgz")));
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
        let canonical_root = workspace_root
            .canonicalize()
            .expect("canonical workspace root");
        assert!(repo_path.starts_with(&canonical_root));
    }
}
