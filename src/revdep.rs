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

const PKGDEPENDS_PATCH_FILENAME: &str = "patch-pkgdepends.R";
const PKGDEPENDS_PATCH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/patch-pkgdepends.R"
));
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

    let patch_path = write_pkgdepends_patch(workspace)?;
    let install_contents =
        build_revdep_install_script(repo_path, num_workers, &codename, &patch_path)?;
    let run_contents = build_revdep_run_script(repo_path, num_workers, &patch_path)?;

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
    patch_path: &Path,
) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers, patch_path);
    let codename_literal = util::r_string_literal(&codename.to_lowercase());

    let script = format!(
        r#"{prelude}

# Configure repositories ----
binary_repo <- sprintf("https://packagemanager.posit.co/cran/__linux__/%s/latest", {codename_literal})
source_repo <- "https://packagemanager.posit.co/cran/latest"

# Configure install options ----
options(
  repos = c(CRAN = binary_repo, posit = binary_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers
)
Sys.setenv(NOT_CRAN = "true")

# Ensure pak is available ----
ensure_pak(source_repo)

# Apply pkgdepends parallel patch ----
source(pkgdepends_patch_path)
pak_patch_parallel_install(pkgdepends_patch_path)

# Ensure tooling prerequisites ----
ensure_installed("xfun")

# DESCRIPTION parsing helpers ----
strip_version <- function(entries) {{
  entries <- gsub("\\s*\\(.*?\\)", "", entries)
  trimws(entries)
}}

parse_description_dependencies <- function(desc_path, fields) {{
  if (!file.exists(desc_path)) {{
    return(character())
  }}
  desc <- read.dcf(desc_path, fields = fields)
  if (!nrow(desc)) {{
    return(character())
  }}
  deps <- character()
  for (field in intersect(fields, colnames(desc))) {{
    value <- desc[1, field]
    if (length(value) && !is.na(value) && nzchar(value)) {{
      entries <- unlist(strsplit(value, ',', fixed = TRUE), use.names = FALSE)
      entries <- strip_version(entries)
      entries <- entries[nzchar(entries) & entries != 'R']
      deps <- c(deps, entries)
    }}
  }}
  sort(unique(deps))
}}

# Gather package metadata ----
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

# Determine installation targets ----
dependency_kinds <- c("Depends", "Imports", "LinkingTo", "Suggests")
cran_package_deps <- tools::package_dependencies(
  packages = package_name,
  db = db,
  which = dependency_kinds,
  reverse = FALSE
)[[package_name]]
cran_package_deps <- cran_package_deps[!is.na(cran_package_deps) & nzchar(cran_package_deps)]
cran_package_deps <- setdiff(cran_package_deps, base_pkgs)

dev_package_deps <- parse_description_dependencies("DESCRIPTION", dependency_kinds)
dev_package_deps <- setdiff(dev_package_deps, base_pkgs)

install_targets <- sort(unique(c(package_name, dev_package_deps, cran_package_deps, revdeps)))

available_packages <- rownames(db)
missing_packages <- setdiff(install_targets, available_packages)
if (length(missing_packages) > 0) {{
  message(
    "Skipping packages not available from repository: ",
    paste(missing_packages, collapse = ", ")
  )
}}
install_targets <- setdiff(install_targets, missing_packages)
install_targets <- setdiff(install_targets, base_pkgs)

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

# Install packages ----
if (length(install_targets) > 0) {{
  pak_install_retry(install_targets)
}} else {{
  stop("No installation targets determined for pak::pkg_install().")
}}
"#
    );

    Ok(script)
}

fn build_revdep_run_script(
    repo_path: &Path,
    num_workers: usize,
    patch_path: &Path,
) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers, patch_path);

    let script = format!(
        r#"{prelude}

# Configure repositories ----
source_repo <- "https://packagemanager.posit.co/cran/latest"

# Configure runtime options ----
options(
  repos = c(CRAN = source_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers,
  mc.cores = install_workers
)
Sys.setenv(NOT_CRAN = "true")

# Ensure pak is available ----
ensure_pak(source_repo)

# Apply pkgdepends parallel patch ----
source(pkgdepends_patch_path)
pak_patch_parallel_install(pkgdepends_patch_path)

# Ensure runtime prerequisites ----
ensure_installed("xfun")
ensure_installed("markdown")
ensure_installed("rmarkdown")

# Configure xfun::rev_check() options ----
options(
  browser = "false",
  install.packages.compile.from.source = "always",
  xfun.rev_check.compare = TRUE,
  xfun.rev_check.timeout = 30 * 60,
  xfun.rev_check.summary = TRUE,
  xfun.rev_check.sample = Inf,
  xfun.rev_check.keep_md = TRUE,
  xfun.rev_check.timeout_total = Inf
)

package_name <- read.dcf("DESCRIPTION", fields = "Package")[1, 1]
if (!nzchar(package_name)) {{
  stop("Failed to read package name from DESCRIPTION")
}}

# Run xfun::rev_check() ----
results <- xfun::rev_check(package_name, src = ".")
invisible(results)
"#
    );

    Ok(script)
}

fn script_prelude(repo_path: &Path, num_workers: usize, patch_path: &Path) -> String {
    let path_literal = util::r_string_literal(&repo_path.to_string_lossy());
    let patch_literal = util::r_string_literal(&patch_path.to_string_lossy());
    let workers = num_workers.max(1);

    format!(
        r#"
# Prepare workspace directories ----
setwd({path_literal})

revdep_dir <- file.path(getwd(), "revdep")
dir.create(revdep_dir, recursive = TRUE, showWarnings = FALSE)
revdep_dir <- normalizePath(revdep_dir, winslash = "/", mustWork = TRUE)

# Configure library paths ----
library_dir <- file.path(revdep_dir, "library")
dir.create(library_dir, recursive = TRUE, showWarnings = FALSE)
library_dir <- normalizePath(library_dir, winslash = "/", mustWork = TRUE)

Sys.setenv(R_LIBS_USER = library_dir)
.libPaths(unique(c(library_dir, .libPaths())))

# Configure parallelism ----
install_workers <- {workers}
options(Ncpus = install_workers)

# Configure pkgdepends patch ----
pkgdepends_patch_path <- {patch_literal}
pkgdepends_patch_path <- normalizePath(pkgdepends_patch_path, winslash = "/", mustWork = TRUE)

# Helpers for package installation ----
pak_install_retry <- function(pkgs, attempts = 5) {{
  pkgs <- as.character(pkgs)
  pkgs <- pkgs[!is.na(pkgs) & nzchar(pkgs)]
  if (!length(pkgs)) {{
    return(invisible(TRUE))
  }}

  install_pkgs <- vapply(
    pkgs,
    function(pkg) {{
      if (grepl("\\?", pkg)) {{
        pkg
      }} else {{
        paste0(pkg, "?ignore-build-errors&ignore-unavailable")
      }}
    }},
    FUN.VALUE = character(1),
    USE.NAMES = FALSE
  )

  for (attempt in seq_len(attempts)) {{
    tryCatch(
      {{
        pak::pkg_install(
          install_pkgs,
          lib = library_dir,
          upgrade = FALSE,
          ask = FALSE,
          dependencies = NA
        )
        return(invisible(TRUE))
      }},
      error = function(err) {{
        if (attempt < attempts) {{
          message(
            sprintf(
              "pak::pkg_install failed (%d/%d) for %s: %s; retrying...",
              attempt,
              attempts,
              paste(pkgs, collapse = ', '),
              conditionMessage(err)
            )
          )
          Sys.sleep(3)
        }} else {{
          stop(err)
        }}
      }}
    )
  }}
}}

ensure_pak <- function(repo) {{
  if (!requireNamespace("pak", quietly = TRUE)) {{
    install.packages(
      "pak",
      repos = repo,
      lib = library_dir,
      quiet = TRUE,
      Ncpus = install_workers
    )
  }}
}}

ensure_installed <- function(pkg) {{
  if (!requireNamespace(pkg, quietly = TRUE)) {{
    pak_install_retry(pkg)
  }}
}}
"#
    )
}

fn write_pkgdepends_patch(workspace: &Workspace) -> Result<PathBuf> {
    let patch_path = workspace.temp_dir().join(PKGDEPENDS_PATCH_FILENAME);
    fs::write(&patch_path, PKGDEPENDS_PATCH).with_context(|| {
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

    #[test]
    fn build_install_script_uses_binary_repo() {
        let path = Path::new("/tmp/example");
        let patch_path = Path::new("/tmp/patch-pkgdepends.R");
        let script =
            build_revdep_install_script(path, 8, "noble", patch_path).expect("script must build");
        assert!(script.contains("https://packagemanager.posit.co/cran/__linux__/%s/latest"));
        assert!(script.contains(
            "sprintf(\"https://packagemanager.posit.co/cran/__linux__/%s/latest\", 'noble')"
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
        assert!(script.contains("source(pkgdepends_patch_path)"));
        assert!(script.contains("pak_patch_parallel_install(pkgdepends_patch_path)"));
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
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains(".libPaths(unique(c(library_dir, .libPaths())))"));
    }

    #[test]
    fn build_run_script_invokes_xfun() {
        let path = Path::new("/tmp/example");
        let patch_path = Path::new("/tmp/patch-pkgdepends.R");
        let script = build_revdep_run_script(path, 8, patch_path).expect("script must build");

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
        assert!(script.contains("source(pkgdepends_patch_path)"));
        assert!(script.contains("pak_patch_parallel_install(pkgdepends_patch_path)"));
        assert!(script.contains("?ignore-build-errors&ignore-unavailable"));
        assert!(script.contains("options("));
        assert!(script.contains("browser = \"false\""));
        assert!(script.contains("install.packages.compile.from.source = \"always\""));
        assert!(script.contains("xfun.rev_check.compare = TRUE"));
        assert!(script.contains("xfun.rev_check.timeout = 30 * 60"));
        assert!(script.contains("xfun.rev_check.summary = TRUE"));
        assert!(script.contains("xfun.rev_check.sample = Inf"));
        assert!(script.contains("xfun.rev_check.keep_md = TRUE"));
        assert!(script.contains("xfun.rev_check.timeout_total = Inf"));
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains("library_dir <- file.path(revdep_dir, \"library\")"));
        assert!(script.contains(
            "library_dir <- normalizePath(library_dir, winslash = \"/\", mustWork = TRUE)"
        ));
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
        fs::write(repo_path.join(".Rbuildignore"), "^README\\.Rmd$")
            .expect("write ignore file");

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
