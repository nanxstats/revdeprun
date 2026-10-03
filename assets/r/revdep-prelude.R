# Shared prelude for the revdep dependency installation and check scripts.
#
# revdeprun prepends a generated configuration block that defines:
# - repo_path: path to the package source directory to check
# - install_workers: number of parallel workers for package installation
# - max_connections: total async HTTP connections for pak/pkgcache downloads
# - pkgdepends_patch_path: path to patch-pkgdepends.R
# - p3m_state_path: path to the P3M rate-limit state file shared by both scripts

# Prepare workspace directories ----
setwd(repo_path)

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
options(Ncpus = install_workers)

# Configure pak/pkgcache async HTTP concurrency for binary downloads ----
options(
  async_http_total_con = max_connections,
  async_http_host_con = 50
)

# Configure pkgdepends patch ----
pkgdepends_patch_path <- normalizePath(pkgdepends_patch_path, winslash = "/", mustWork = TRUE)
p3m_state_path <- normalizePath(p3m_state_path, winslash = "/", mustWork = TRUE)

# Helpers for package installation ----
pak_install_retry <- function(pkgs, attempts = 5) {
  pkgs <- as.character(pkgs)
  pkgs <- pkgs[!is.na(pkgs) & nzchar(pkgs)]
  if (!length(pkgs)) {
    return(invisible(TRUE))
  }

  install_pkgs <- vapply(
    pkgs,
    function(pkg) {
      if (grepl("\\?", pkg)) {
        pkg
      } else {
        paste0(pkg, "?ignore-build-errors&ignore-unavailable")
      }
    },
    FUN.VALUE = character(1),
    USE.NAMES = FALSE
  )

  for (attempt in seq_len(attempts)) {
    tryCatch(
      {
        pak::pkg_install(
          install_pkgs,
          lib = library_dir,
          upgrade = FALSE,
          ask = FALSE,
          dependencies = NA
        )
        return(invisible(TRUE))
      },
      error = function(err) {
        if (attempt < attempts) {
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
        } else {
          stop(err)
        }
      }
    )
  }
}

ensure_pak <- function(repo) {
  if (!requireNamespace("pak", quietly = TRUE)) {
    install.packages(
      "pak",
      repos = repo,
      lib = library_dir,
      quiet = TRUE,
      Ncpus = install_workers
    )
  }
}

ensure_installed <- function(pkg) {
  if (!requireNamespace(pkg, quietly = TRUE)) {
    pak_install_retry(pkg)
  }
}
