# Resolve Ubuntu system requirements for the reverse dependencies of a package
# and print them as JSON for revdeprun to install.
#
# revdeprun prepends a generated configuration block that defines:
# - pkg_name: name of the package whose reverse dependencies are resolved
# - install_workers: number of parallel workers for package installation

options(warn = 2)

source_repo <- "https://packagemanager.posit.co/cran/latest"

options(
  repos = c(CRAN = source_repo),
  BioC_mirror = "https://packagemanager.posit.co/bioconductor",
  Ncpus = install_workers
)
Sys.setenv(NOT_CRAN = "true")

user_lib <- Sys.getenv("R_LIBS_USER")
if (!nzchar(user_lib)) {
  stop('R_LIBS_USER is empty; cannot install packages into user library')
}
dir.create(user_lib, recursive = TRUE, showWarnings = FALSE)
.libPaths(c(user_lib, .libPaths()))

ensure_installed <- function(pkg) {
  if (!requireNamespace(pkg, quietly = TRUE)) {
    install.packages(
      pkg,
      repos = getOption("repos"),
      lib = user_lib,
      quiet = TRUE,
      Ncpus = install_workers
    )
  }
}

ensure_installed("pak")
ensure_installed("jsonlite")

db <- available.packages(repos = source_repo, type = "source")
revdeps <- tools::package_dependencies(
  packages = pkg_name,
  db = db,
  which = c("Depends", "Imports", "LinkingTo", "Suggests"),
  reverse = TRUE
)[[pkg_name]]
if (is.null(revdeps)) {
  revdeps <- character()
}
revdeps <- sort(unique(stats::na.omit(revdeps)))
if (length(revdeps) > 0) {
  base_pkgs <- unique(c(.BaseNamespaceEnv$basePackage, rownames(installed.packages(priority = "base"))))
  revdeps <- setdiff(revdeps, base_pkgs)
}

sysreqs <- if (length(revdeps) == 0) {
  list(install_scripts = character(), post_install = character())
} else {
  pak::pkg_sysreqs(revdeps, sysreqs_platform = "ubuntu")
}

if (!is.list(sysreqs) || is.null(sysreqs$install_scripts) || is.null(sysreqs$post_install)) {
  stop("unexpected sysreqs payload")
}
sysreqs$post_install <- unique(sysreqs$post_install)

cat(jsonlite::toJSON(sysreqs[c('install_scripts', 'post_install')], auto_unbox = TRUE))
