# revdeprun

[![crates.io version](https://img.shields.io/crates/v/revdeprun)](https://crates.io/crates/revdeprun)
[![CI tests](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml/badge.svg)](https://github.com/nanxstats/revdeprun/actions/workflows/ci.yml)
![License](https://img.shields.io/crates/l/revdeprun)

One-key reverse dependency checks for R via revdepcheck with cloud-ready environment setup.

## Installation

Install build-essential:

```bash
sudo apt-get update && sudo apt-get install build-essential
```

[Install Rust](https://rust-lang.org/tools/install/).

Install `revdeprun` from crates.io using Cargo:

```bash
cargo install revdeprun
```

To try the latest development version directly from GitHub:

```bash
cargo install --git https://github.com/nanxstats/revdeprun.git
```

## Requirements

- Ubuntu 20.04 or newer with `sudo` access (tooling uses `apt-get`, `gdebi`, and system-wide `/opt/R` installs).
- Network access to download R binaries, R packages, and the target Git repository.
- Git available on `PATH`.

Running inside a fresh cloud instance is recommended because reverse dependency
checks execute third-party code.

## Usage

The CLI provisions the requested R toolchain, prepares the package repository,
and runs `revdepcheck` end-to-end:

```bash
revdeprun https://github.com/nanxstats/ggsci.git
```

By default, the current release version of R for Ubuntu is installed
and the number of workers is set to use all available CPU cores.

### Command options

```
$ revdeprun --help
Provision R and run revdepcheck end-to-end

Usage: revdeprun [OPTIONS] <REPOSITORY>

Arguments:
  <REPOSITORY>  Git URL or path to the R package you want to check

Options:
      --r-version <R_VERSION>  R version spec (e.g. release, 4.3.3, oldrel-1) [default: release]
      --num-workers <N>        Override parallel worker count (defaults to host CPUs)
      --work-dir <PATH>        Custom workspace directory for clones and temp files
      --skip-r-install         Reuse an existing system R instead of installing
  -h, --help                   Print help
  -V, --version                Print version
```

### Typical workflow

1. Provision a clean Ubuntu VM with sufficient CPU and memory.
2. Install `revdeprun` (for example, `cargo install revdeprun`).
3. Run `revdeprun <repo>` with optional flags, such as:
   - `revdeprun --r-version release https://github.com/nanxstats/ggsci.git`
   - `revdeprun --num-workers 48 --work-dir /data/workspace git@github.com:nanxstats/ggsci.git`
4. Review the results under `<repo>/revdep`.

### Notes

- `revdeprun` installs R into `/opt/R/...` and symlinks binaries to `/usr/local/bin`.
- The tool uses the latest development version of revdepcheck via `remotes::install_github("r-lib/revdepcheck")`.
- If you already provision R and required packages, pass `--skip-r-install`.
- To point at a local checkout instead of cloning, supply the path directly:
  `revdeprun ~/workspace/ggsci`.
