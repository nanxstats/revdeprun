# Install the package, its dependencies, and all reverse dependencies into the
# revdep library before running xfun::rev_check().
#
# revdeprun appends this file after revdep-prelude.R. In addition to the
# variables documented there, the generated configuration block defines:
# - ubuntu_codename: lowercase Ubuntu release codename for P3M binary URLs

# Configure repositories ----
binary_repo <- sprintf("https://packagemanager.posit.co/cran/__linux__/%s/latest", ubuntu_codename)
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
pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)

# Ensure tooling prerequisites ----
ensure_installed("xfun")

# Inform user about dependency resolution work ----
message("Parsing package metadata and dependency lists...\nThis can take a few minutes for large revdep sets.")

# DESCRIPTION parsing helpers ----
strip_version <- function(entries) {
  entries <- gsub("\\s*\\(.*?\\)", "", entries)
  trimws(entries)
}

parse_description_dependencies <- function(desc_path, fields) {
  if (!file.exists(desc_path)) {
    return(character())
  }
  desc <- read.dcf(desc_path, fields = fields)
  if (!nrow(desc)) {
    return(character())
  }
  deps <- character()
  for (field in intersect(fields, colnames(desc))) {
    value <- desc[1, field]
    if (length(value) && !is.na(value) && nzchar(value)) {
      entries <- unlist(strsplit(value, ',', fixed = TRUE), use.names = FALSE)
      entries <- strip_version(entries)
      entries <- entries[nzchar(entries) & entries != 'R']
      deps <- c(deps, entries)
    }
  }
  sort(unique(deps))
}

# Gather package metadata ----
package_name <- read.dcf("DESCRIPTION", fields = "Package")[1, 1]
if (!nzchar(package_name)) {
  stop("Failed to read package name from DESCRIPTION")
}

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
if (length(missing_packages) > 0) {
  message(
    "Skipping packages not available from repository: ",
    paste(missing_packages, collapse = ", ")
  )
}
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

if (length(revdeps) == 0) {
  message("No CRAN reverse dependencies detected; installing package binary only.")
}

# Install packages ----
if (length(install_targets) > 0) {
  message(sprintf(
    "Installing %d packages with pak::pkg_install()...",
    length(install_targets)
  ))
  pak_install_retry(install_targets)
} else {
  stop("No installation targets determined for pak::pkg_install().")
}
