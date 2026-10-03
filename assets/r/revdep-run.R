# Run xfun::rev_check() against the prepared revdep library.
#
# revdeprun appends this file after revdep-prelude.R and relies on the
# configuration variables documented there.

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
pak_patch_parallel_install(pkgdepends_patch_path, p3m_state_path)

# Ensure runtime prerequisites ----
ensure_installed("xfun")
ensure_installed("markdown")
ensure_installed("rmarkdown")

# Apply P3M rate limiting to xfun source downloads ----
xfun_patch_p3m_downloads(p3m_state_path)

# Configure xfun::rev_check() options ----
options(
  browser = "false",
  install.packages.compile.from.source = "always",
  xfun.rev_check.compare = TRUE,
  xfun.rev_check.download_cores = 50,
  xfun.rev_check.timeout = 30 * 60,
  xfun.rev_check.summary = TRUE,
  xfun.rev_check.sample = Inf,
  xfun.rev_check.keep_md = TRUE,
  xfun.rev_check.timeout_total = Inf
)

package_name <- read.dcf("DESCRIPTION", fields = "Package")[1, 1]
if (!nzchar(package_name)) {
  stop("Failed to read package name from DESCRIPTION")
}

# Run xfun::rev_check() ----
results <- xfun::rev_check(package_name, src = ".")
invisible(results)
