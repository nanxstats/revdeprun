# Changelog

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
