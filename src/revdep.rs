use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use tempfile::NamedTempFile;
use xshell::{Shell, cmd};

use crate::{
    progress::{Progress, Task},
    util, workspace,
};

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
        .context("failed to write revdepcheck bootstrap script")?;

    let prepare_contents = build_revdep_prepare_script(repo_path, num_workers)?;
    let run_contents = build_revdep_run_script(repo_path, num_workers)?;
    let mut prepare_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    let mut run_script =
        NamedTempFile::new_in(workspace).context("failed to create temporary R script file")?;
    prepare_script
        .write_all(prepare_contents.as_bytes())
        .context("failed to write revdep preparation script")?;
    run_script
        .write_all(run_contents.as_bytes())
        .context("failed to write revdep execution script")?;

    let setup_path = setup_script.path().to_owned();
    let prepare_path = prepare_script.path().to_owned();
    let run_path = run_script.path().to_owned();

    let _dir_guard = shell.push_dir(repo_path);

    let bootstrap_task = progress.task("Bootstrapping revdepcheck dependencies");
    let setup_output = cmd!(shell, "Rscript --vanilla {setup_path}")
        .quiet()
        .ignore_status()
        .output();

    match setup_output {
        Ok(output) if output.status.success() => {
            bootstrap_task.finish_with_message("revdepcheck dependencies ready".to_string());
        }
        Ok(output) => {
            bootstrap_task.fail("Failed to prepare revdepcheck dependencies".to_string());
            util::emit_command_output(
                progress,
                "revdepcheck dependency bootstrap",
                &output.stdout,
                &output.stderr,
            );
            bail!(
                "revdepcheck dependency bootstrap failed with status {}",
                output.status
            );
        }
        Err(err) => {
            bootstrap_task.fail("Failed to launch revdepcheck bootstrap".to_string());
            return Err(err).context("failed to prepare revdepcheck dependencies");
        }
    }

    let prepare_summary = run_revdep_prepare_phase(
        repo_path,
        &prepare_path,
        progress,
        progress.task("Pre-installing reverse dependencies"),
    )?;

    report_prepare_summary(progress, &prepare_summary);

    progress.println(format!(
        "Launching revdepcheck.extras with {num_workers} workers..."
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

#[derive(Debug, Deserialize)]
struct PrepareSummary {
    todo_count: usize,
    #[serde(default)]
    precache_failed: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
}

fn parse_revdep_prepare_summary(progress: &Progress, stdout: &[u8]) -> Result<PrepareSummary> {
    let payload = String::from_utf8(stdout.to_vec())
        .context("revdep metadata preparation emitted non-UTF-8 output")?;
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        bail!("revdep metadata preparation returned no summary output");
    }
    serde_json::from_str(trimmed).map_err(|err| {
        util::emit_command_output(progress, "revdep metadata summary (raw)", stdout, b"");
        err.into()
    })
}

fn report_prepare_summary(progress: &Progress, summary: &PrepareSummary) {
    if summary.todo_count == 0 {
        progress.println(
            "No CRAN reverse dependencies detected; revdepcheck.extras will exit immediately.",
        );
    } else {
        progress.println(format!(
            "Queued {} reverse dependencies for revdepcheck.extras.",
            summary.todo_count
        ));
    }

    if !summary.precache_failed.is_empty() {
        let failed = summary.precache_failed.join(", ");
        progress.println(format!(
            "Warning: failed to precache {} packages: {failed}",
            summary.precache_failed.len()
        ));
    }

    if !summary.warnings.is_empty() {
        for warning in &summary.warnings {
            progress.println(format!("Warning from revdep preparation: {warning}"));
        }
    }
}

fn run_revdep_prepare_phase(
    repo_path: &Path,
    prepare_path: &Path,
    progress: &Progress,
    prepare_task: Task,
) -> Result<PrepareSummary> {
    let progress_bar = prepare_task.progress_bar();

    let mut command = Command::new("Rscript");
    command.arg("--vanilla");
    command.arg(prepare_path);
    command.current_dir(repo_path);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            prepare_task.fail("Failed to launch revdep metadata preparation".to_string());
            return Err(err).context("failed to launch revdep metadata preparation");
        }
    };

    let monitor = CrancacheMonitor::start(progress_bar, repo_path.to_path_buf());

    let output_result = child.wait_with_output();
    monitor.shutdown();

    let output = match output_result {
        Ok(output) => output,
        Err(err) => {
            prepare_task.fail("Failed to capture revdep metadata output".to_string());
            return Err(err).context("failed to capture revdep metadata output");
        }
    };

    if output.status.success() {
        match parse_revdep_prepare_summary(progress, &output.stdout) {
            Ok(summary) => {
                prepare_task.finish_with_message("Reverse dependencies discovered".to_string());
                Ok(summary)
            }
            Err(err) => {
                prepare_task.fail("Failed to parse revdep metadata summary".to_string());
                Err(err)
            }
        }
    } else {
        prepare_task.fail("Failed to prepare revdep metadata".to_string());
        util::emit_command_output(
            progress,
            "revdep metadata preparation",
            &output.stdout,
            &output.stderr,
        );
        bail!(
            "revdep metadata preparation failed with status {}",
            output.status
        );
    }
}

struct CrancacheMonitor {
    stop: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl CrancacheMonitor {
    fn start(bar: indicatif::ProgressBar, repo_root: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = thread::spawn(move || {
            let src_dir = repo_root.join("revdep/crancache/cran/src/contrib");
            let bin_dir = repo_root.join("revdep/crancache/cran-bin/src/contrib");
            let mut last_message = String::new();

            while !stop_flag.load(Ordering::SeqCst) {
                let src_count = count_cached_packages(&src_dir);
                let bin_count = count_cached_packages(&bin_dir);
                let message = format!(
                    "Pre-installing reverse dependencies (cache: {src_count} src tarballs, {bin_count} binary tarballs)"
                );

                if message != last_message {
                    bar.set_message(message.clone());
                    last_message = message;
                }

                for _ in 0..10 {
                    if stop_flag.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        });

        Self { stop, handle }
    }

    fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.handle.join();
    }
}

const CRANCACHE_METADATA_FILES: &[&str] =
    &["PACKAGES", "PACKAGES.db", "PACKAGES.gz", "PACKAGES.rds"];

fn count_cached_packages(dir: &Path) -> usize {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| match entry.file_type() {
                Ok(kind) => kind.is_file(),
                Err(_) => false,
            })
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| !CRANCACHE_METADATA_FILES.contains(&name))
                    .unwrap_or(true)
            })
            .count(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => 0,
        Err(_) => 0,
    }
}

fn build_revdep_setup_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);

    let script = format!(
        r#"{prelude}

if (!requireNamespace("BiocManager", quietly = TRUE)) {{
  install.packages(
    "BiocManager",
    repos = getOption("repos"),
    lib = user_lib,
    quiet = TRUE,
    Ncpus = install_workers
  )
}}
if (!requireNamespace("pak", quietly = TRUE)) {{
  install.packages(
    "pak",
    repos = getOption("repos"),
    lib = user_lib,
    quiet = TRUE,
    Ncpus = install_workers
  )
}}
if (!requireNamespace("revdepcheck.extras", quietly = TRUE)) {{
  pak::pkg_install(
    "HenrikBengtsson/revdepcheck.extras",
    lib = user_lib,
    ask = FALSE,
    upgrade = FALSE,
    dependencies = TRUE
  )
}}
"#
    );

    Ok(script)
}

fn build_revdep_prepare_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);
    let workers = num_workers.max(1);

    let script = format!(
        r#"{prelude}

options(revdepcheck.num_workers = {workers})
if (!nzchar(Sys.getenv("R_REVDEPCHECK_NUM_WORKERS"))) {{
  Sys.setenv(R_REVDEPCHECK_NUM_WORKERS = as.character({workers}))
}}
if (!nzchar(Sys.getenv("R_REVDEPCHECK_TIMEOUT"))) {{
  Sys.setenv(R_REVDEPCHECK_TIMEOUT = "720")
}}
Sys.setenv(R_BIOC_VERSION = as.character(BiocManager::version()))

revdepcheck.extras::revdep_reset(".")
revdepcheck.extras::revdep_init(".")

children <- revdepcheck.extras::revdep_children(".")
if (length(children) > 0) {{
  revdepcheck::revdep_add(".", packages = children)
}}

todo_pkgs <- revdepcheck.extras::todo(".", print = FALSE)

summary <- list(
  todo_count = length(todo_pkgs),
  precache_failed = character(),
  warnings = character()
)

if (length(todo_pkgs) > 0) {{
  handler <- function(w) {{
    summary$warnings <<- c(summary$warnings, conditionMessage(w))
    invokeRestart("muffleWarning")
  }}

  precache_failures <- suppressWarnings(withCallingHandlers(
    suppressMessages(revdepcheck.extras::revdep_precache(package = ".")),
    warning = handler
  ))
  summary$precache_failed <- precache_failures

  suppressWarnings(withCallingHandlers(
    suppressMessages(revdepcheck.extras::revdep_preinstall(
      todo_pkgs,
      chunk_size = 16L
    )),
    warning = handler
  ))
}}

cat(jsonlite::toJSON(summary, auto_unbox = TRUE))
"#
    );

    Ok(script)
}

fn build_revdep_run_script(repo_path: &Path, num_workers: usize) -> Result<String> {
    let prelude = script_prelude(repo_path, num_workers);
    let workers = num_workers.max(1);

    let script = format!(
        r#"{prelude}

options(revdepcheck.num_workers = {workers})
if (!nzchar(Sys.getenv("R_REVDEPCHECK_NUM_WORKERS"))) {{
  Sys.setenv(R_REVDEPCHECK_NUM_WORKERS = as.character({workers}))
}}
if (!nzchar(Sys.getenv("R_REVDEPCHECK_TIMEOUT"))) {{
  Sys.setenv(R_REVDEPCHECK_TIMEOUT = "720")
}}
Sys.setenv(R_BIOC_VERSION = as.character(BiocManager::version()))

revdepcheck.extras::run()
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

cran_repo <- "https://cloud.r-project.org/"
install_workers <- max({workers}, parallel::detectCores())

options(
  repos = c(CRAN = cran_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers
)
Sys.setenv(NOT_CRAN = "true")

user_lib <- Sys.getenv("R_LIBS_USER")
if (!nzchar(user_lib)) {{
  stop('R_LIBS_USER is empty; cannot install packages into user library')
}}
dir.create(user_lib, recursive = TRUE, showWarnings = FALSE)
.libPaths(c(user_lib, .libPaths()))

crancache_dir <- file.path("revdep", "crancache")
dir.create(crancache_dir, recursive = TRUE, showWarnings = FALSE)
Sys.setenv(CRANCACHE_DIR = crancache_dir)
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
        assert!(script.contains("HenrikBengtsson/revdepcheck.extras"));
        assert!(script.contains("repos = getOption(\"repos\")"));
        assert!(script.contains("https://cloud.r-project.org/"));
        assert!(script.contains("parallel::detectCores"));
    }

    #[test]
    fn build_prepare_script_summarises_work() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_prepare_script(path, 8).expect("script must build");

        assert!(script.contains("revdepcheck.extras::revdep_precache"));
        assert!(script.contains("revdepcheck.extras::revdep_preinstall"));
        assert!(script.contains("suppressMessages"));
        assert!(script.contains("jsonlite::toJSON"));
        assert!(script.contains("revdepcheck::revdep_add"));
        assert!(script.contains("todo_pkgs <- revdepcheck.extras::todo"));
        assert!(script.contains("chunk_size = 16L"));
    }

    #[test]
    fn build_run_script_triggers_revdepcheck() {
        let path = Path::new("/tmp/example");
        let script = build_revdep_run_script(path, 8).expect("script must build");

        assert!(script.contains("revdepcheck.extras::run"));
        assert!(script.contains("R_REVDEPCHECK_NUM_WORKERS"));
        assert!(script.contains("R_REVDEPCHECK_TIMEOUT"));
        assert!(script.contains("num_workers = 8"));
        assert!(script.contains("setwd('/tmp/example')"));
        assert!(script.contains(".libPaths(c(user_lib, .libPaths()))"));
        assert!(script.contains("https://cloud.r-project.org/"));
    }
}
