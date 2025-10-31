# Changelog

## Unreleased

### Improvements

- Replace the `revdepcheck` invocation with Henrik Bengtsson's
  `revdepcheck.extras`, enabling cached package downloads plus pre-installation
  of reverse dependencies to reduce "package suggested but not available"
  failures on freshly provisioned machines.
- Install `revdepcheck.extras` during system requirement resolution so the
  bundled `revdepcheck` fork is used consistently across the workflow.

## revdeprun 0.5.0

### Improvements

- Resolve the Ubuntu system requirements of all CRAN reverse dependencies with
  `pak::pkg_sysreqs()` and run the installation with post-installation commands
  before running revdepcheck (#35).
- Provision Quarto and TinyTeX with `PATH` updates by default so `.qmd` and PDF
  vignettes can render during reverse dependency checks (#34).

## revdeprun 0.4.0

### Bug fixes

- Increased the `revdepcheck::revdep_check()` timeout to 12 hours to
  accommodate longer installation and reverse dependency checks (#25).
  Keep using the source package mirror so that revdepcheck can run (#28, #30).

### Improvements

- Provision `pandoc` by default so R package vignettes can build during
  reverse dependency checks (#26).
- Set `revdepcheck::revdep_check(bioc = FALSE)` to follow the default
  behavior of `revdepcheck::cloud_check()` on **not** checking Bioconductor
  packages by default (#20).
- Set `revdepcheck::revdep_check(quiet = TRUE)` to suppress the package
  installation outputs (#21).

## revdeprun 0.3.0

### Speed optimization

- `revdep.rs` now sets `options(Ncpus = workers)` so the dependency compilation
  phase can also use all available CPU cores (#12).

### Compact terminal output

- Progress for R and R package installation, workspace preparation, and
  repository cloning is now presented with compact spinners
  using `indicatif` (#13).
  - Suppress revdepcheck bootstrap logs from R dependency installation.
    They will only surface when something fails.
  - Once revdepcheck dependencies are bootstrapped, only the interactive
    `revdepcheck` session streams to stdout.

## revdeprun 0.2.0

### Improvements

- Install `libcurl4-openssl-dev` and `libssl-dev` automatically, which are required by
  openssl and curl, dependencies of revdepcheck (#4).

### Documentation

- Improved `README.md` for clarity (#5, #6).

## revdeprun 0.1.0

### New features

- Initial public release of the `revdeprun` CLI.
- Automated Ubuntu R installer using the R-hub version resolution API.
- Implemented workflow that clones an R package repository and runs
  revdepcheck with configurable worker numbers.
- Support for custom workspaces, version overrides, and reusing pre-installed R.
