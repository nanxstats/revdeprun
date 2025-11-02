# Changelog

## revdeprun 1.1.2

### Bug fixes

- Install TinyTeX via Quarto commands used in official GitHub Actions workflows
  and detect existing installations with the `quarto list tools` method.

## revdeprun 1.1.1

### Improvements

- Update progress messages during R prerequisite installation to
  accurately reflect that the system requirements being installed are
  for the R package {pak}, rather than for revdep dependencies (#66).

## revdeprun 1.1.0

### New features

- Accept local source package tarballs (`.tar.gz`) as inputs for reverse
  dependency checks (#61).

### Improvements

- Launch `Rscript` with an auto-tuned `--max-connections` value based
  on the CPU core count to avoid potential connection limits during parallel
  reverse dependency checks on high-CPU instances (#60).

## revdeprun 1.0.0

### Significant changes

- Replace the {revdepcheck.extras} workflow with a fast binary pre-installation
  phase followed by `xfun::rev_check()`.

  The new approach installs the binary packages required for checking reverse
  dependencies from Posit Public Package Manager (P3M) into `revdep/library/`
  before running the parallel checks.
  This combination dramatically reduces the pre-install time while making
  the checks more deterministic and robust (#48, #51).

  A special thanks to @yihui for his work on `xfun::rev_check()`
  and the helpful discussions enabling this improvement.

- Refactor workspace management to separate the cloned repo and temporary file
  directory (#52). This makes the directory structure canonical and predictable.

## revdeprun 0.6.0

### Significant changes

- Migrate from {revdepcheck} to Henrik Bengtsson's {revdepcheck.extras},
  which provides deterministic pre-installation and caching for
  revdep dependencies (#42).

  This might reduce "package suggested but not available" errors when
  running revdepcheck. It may also reduce potential repeated compilation of
  the same dependency across different revdeps due to the "caching dependencies
  while checking multiple packages" mechanism.

### Improvements

- Set `timeout` to the more sensible 60 minutes instead of the previous
  12 hours. This is consistent with the default value used by
  `revdepcheck.extras::check()`. From the documentation, `timeout` means the
  time limit for running `R CMD check` on one version of one package (#42).
- Deduplicate post-install commands in the system requirements script
  to prevent redundant execution of identical post-install hooks (#41).

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
